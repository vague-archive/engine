use std::{
    array::from_fn,
    borrow::Borrow,
    cell::UnsafeCell,
    collections::HashMap,
    ffi::{CStr, CString},
    hash::Hash,
    marker::PhantomData,
    mem::{MaybeUninit, size_of},
    num::NonZero,
    ops::{Deref, DerefMut},
    ptr, slice,
};

use aligned_vec::AVec;
use atomic_refcell::{AtomicRef, AtomicRefCell, AtomicRefMut};
use events_generated::{
    Despawn, LoadScene, LoadSceneBuilder, RemoveComponents, RemoveComponentsBuilder,
    SetEntityLabel, SetEntityLabelBuilder, SetParent, SetSystemEnabled, SetSystemEnabledBuilder,
};
use flatbuffers::{FlatBufferBuilder, Follow, Push, root_unchecked};
use game_entity::EntityId;
use num_enum::TryFromPrimitive;
use platform::{Executor, Platform};
use void_public::ComponentId;

pub mod events_generated {
    #![allow(clippy::all, clippy::pedantic, warnings, unused, unused_imports)]
    include!(concat!(env!("OUT_DIR"), "/events_generated.rs"));
}

#[cfg(feature = "state_snapshots")]
mod serialize;

/// Helper macro to safely read flatbuffers events. Requires the `flatbuffers` dependency.
#[macro_export]
macro_rules! platform_event_iter {
    ($event_delegate:expr, $name:ident, $closure:expr) => {
        if let Some(storage) = $event_delegate.storage(::void_public::event_name!($name)) {
            (0..storage.count())
                .map(|i| unsafe {
                    let ptr = storage.read_event(i);
                    let len = ptr.read() as usize;
                    let data = ::std::slice::from_raw_parts(ptr.offset(1).cast(), len);
                    ::flatbuffers::root_unchecked::<$name>(data)
                })
                .for_each($closure)
        }
    };
}

/// This enum is used when reading commands. It is used to return command
/// references to the caller.
pub enum CommandRef<'a> {
    AddComponents(AddComponents<'a>),
    Despawn(&'a Despawn),
    LoadScene(LoadScene<'a>),
    RemoveComponents(RemoveComponents<'a>),
    SetEntityLabel(SetEntityLabel<'a>),
    SetParent(&'a SetParent),
    SetSystemEnabled(SetSystemEnabled<'a>),
    Spawn(AddComponents<'a>),
}

/// This enum is binary-encoded into the event data buffer, to indicate the
/// type of the event data to follow.
#[derive(TryFromPrimitive)]
#[repr(u8)]
enum CommandTag {
    AddComponents,
    Despawn,
    LoadScene,
    RemoveComponents,
    SetEntityLabel,
    SetParent,
    SetSystemEnabled,
    Spawn,
}

#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::SerializeMut))]
pub struct EventManager<P: Platform> {
    /// Stores events received from the platform.
    platform_event_data: PlatformEventData<P>,

    /// Stores events received from modules. For each event type, one storage is
    /// allocated per writing system (indexed by the system name).
    module_event_data: ModuleEventData<P>,

    /// Stores command data, i.e. spawn and despawn commands.
    ///
    /// The outer `Vec`'s length matches the number of threads in the executor,
    /// and contains one buffer per thread. When events are written from a
    /// thread, they have exclusive access to their corresponding thread buffer.
    /// The inner `AVec` is wrappen in `UnsafeCell` so that we can access the
    /// inner data via a shared `&EventManager` reference.
    ///
    /// The inner `AVec` is 8-byte aligned, and contains type-erased commands.
    /// Each command entry is a 1-byte tag (`CommandTag`), followed by the
    /// data for the command. Most of the command data is flatbuffers events,
    /// but some commands like `Spawn` have custom (de)serialization logic.
    command_data: CommandData,
}

/// Newtype pattern used for trait implementations.
struct PlatformEventData<P: Platform>(HashMap<CString, EventWriterStorage<P>>);

impl<P: Platform> Deref for PlatformEventData<P> {
    type Target = HashMap<CString, EventWriterStorage<P>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: Platform> DerefMut for PlatformEventData<P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Newtype pattern used for trait implementations.
struct ModuleEventData<P: Platform>(HashMap<CString, HashMap<String, EventWriterStorage<P>>>);

impl<P: Platform> Deref for ModuleEventData<P> {
    type Target = HashMap<CString, HashMap<String, EventWriterStorage<P>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<P: Platform> DerefMut for ModuleEventData<P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Newtype pattern used for trait implementations.
struct CommandData(Vec<UnsafeCell<AVec<MaybeUninit<u8>>>>);

impl Deref for CommandData {
    type Target = Vec<UnsafeCell<AVec<MaybeUninit<u8>>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for CommandData {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct PlatformEventDelegate<'a, P: Platform> {
    platform_event_data: &'a mut HashMap<CString, EventWriterStorage<P>>,
}

/// All `Sync` access to `command_data` is thread-safe, because the inner
/// buffers are only accessed by one thread at a time (indexed via
/// `Executor::thread_index()`).
unsafe impl<P: Platform> Sync for EventManager<P> {}

impl<P: Platform> Default for EventManager<P> {
    fn default() -> Self {
        let command_data = (0..P::Executor::available_parallelism().get())
            .map(|_| UnsafeCell::new(AVec::new(8)))
            .collect();

        Self {
            platform_event_data: PlatformEventData(HashMap::new()),
            module_event_data: ModuleEventData(HashMap::new()),
            command_data: CommandData(command_data),
        }
    }
}

impl<P: Platform> EventManager<P> {
    /// Creates a new event storage for the given event identifier.
    pub fn register_module_event_writer<E: Into<CString>, S: Into<String>>(
        &mut self,
        event_ident: E,
        system_name: S,
    ) {
        let per_thread_buffers = (0..P::Executor::available_parallelism().get())
            .map(|_| {
                ThreadBuffer {
                    data: Vec::new(),
                    event_count: 0,
                }
                .into()
            })
            .collect();

        let inner = EventWriterStorageInner { per_thread_buffers }.into();

        let event_buffer = EventWriterStorage {
            inner,
            marker: PhantomData,
        };

        self.module_event_data
            .entry(event_ident.into())
            .or_default()
            .insert(system_name.into(), event_buffer);
    }

    /// Used during frame processing to read module and platform events across
    /// *all* storage buffers for the given event type.
    pub fn event_storage<I, F>(&self, ident: &I, mut f: F)
    where
        CString: Borrow<I>,
        I: Hash + Eq + ?Sized,
        F: FnMut(&EventWriterStorage<P>),
    {
        if let Some(storage) = self.platform_event_data.get(ident) {
            f(storage);
        }

        if let Some(storages) = self.module_event_data.get(ident) {
            for storage in storages.values() {
                f(storage);
            }
        }
    }

    /// Used during frame processing to get a *specific* system's event writer
    /// storage for the given event type.
    pub fn module_event_storage<E, S>(
        &self,
        event_ident: &E,
        writing_system_name: &S,
    ) -> Option<&EventWriterStorage<P>>
    where
        CString: Borrow<E>,
        String: Borrow<S>,
        E: Hash + Eq + ?Sized,
        S: Hash + Eq + ?Sized,
    {
        self.module_event_data
            .get(event_ident)
            .and_then(|storages| storages.get(writing_system_name))
    }

    /// Used to enqueue platform events.
    pub fn platform_event_delegate(&mut self) -> PlatformEventDelegate<'_, P> {
        PlatformEventDelegate {
            platform_event_data: &mut self.platform_event_data,
        }
    }

    pub fn command_add_components<'a, F>(&self, entity_id: EntityId, components_len: usize, f: F)
    where
        F: Fn(usize) -> ComponentData<'a>,
    {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        buffer.push(MaybeUninit::new(CommandTag::AddComponents as u8));
        write_add_components_in_place(buffer, entity_id, components_len, f);
    }

    pub fn command_despawn(&self, entity_id: EntityId) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let event = Despawn::new(NonZero::from(entity_id).get());

        buffer.push(MaybeUninit::new(CommandTag::Despawn as u8));
        write_struct_event_in_place(buffer, event);
    }

    pub fn command_remove_components(&self, entity_id: EntityId, component_ids: &[ComponentId]) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let mut fbb = FlatBufferBuilder::new();
        let component_ids = fbb.create_vector_from_iter(component_ids.iter().map(|c| c.get()));
        let mut builder = RemoveComponentsBuilder::new(&mut fbb);
        builder.add_entity_id(NonZero::from(entity_id).get());
        builder.add_component_ids(component_ids);
        let offset = builder.finish();
        fbb.finish_minimal(offset);

        buffer.push(MaybeUninit::new(CommandTag::RemoveComponents as u8));
        write_table_event_bytes_in_place(buffer, fbb.finished_data());
    }

    pub fn command_set_system_enabled(&self, system_name: &str, enabled: bool) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let mut fbb = FlatBufferBuilder::new();
        let system_name = fbb.create_string(system_name);
        let mut builder = SetSystemEnabledBuilder::new(&mut fbb);
        builder.add_system_name(system_name);
        builder.add_enabled(enabled);
        let offset = builder.finish();
        fbb.finish_minimal(offset);

        buffer.push(MaybeUninit::new(CommandTag::SetSystemEnabled as u8));
        write_table_event_bytes_in_place(buffer, fbb.finished_data());
    }

    /// Associates an entity a some string label.
    pub fn command_set_entity_label(&self, entity_id: EntityId, label: Option<&CStr>) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let mut fbb = FlatBufferBuilder::new();
        let label = label.map(|label| fbb.create_string(&label.to_string_lossy()));
        let mut builder = SetEntityLabelBuilder::new(&mut fbb);
        builder.add_entity_id(NonZero::from(entity_id).get());
        if let Some(label) = label {
            builder.add_label(label);
        }
        let offset = builder.finish();
        fbb.finish_minimal(offset);

        buffer.push(MaybeUninit::new(CommandTag::SetEntityLabel as u8));
        write_table_event_bytes_in_place(buffer, fbb.finished_data());
    }

    /// Enqueues a `set_parent` event into the event queue. This event will be processed at the end of
    /// the frame.
    pub fn command_set_parent(
        &self,
        entity_id: EntityId,
        parent_id: Option<EntityId>,
        keep_world_space_transform: bool,
    ) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let event = SetParent::new(
            NonZero::from(entity_id).get(),
            parent_id.map_or(0, |id| NonZero::from(id).get()),
            keep_world_space_transform,
        );

        buffer.push(MaybeUninit::new(CommandTag::SetParent as u8));
        write_struct_event_in_place(buffer, event);
    }

    pub fn command_load_scene(&self, scene_str: &CStr) {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        let mut fbb = FlatBufferBuilder::new();
        let scene_str = fbb.create_string(&scene_str.to_string_lossy());
        let mut builder = LoadSceneBuilder::new(&mut fbb);
        builder.add_scene_json(scene_str);
        let offset = builder.finish();
        fbb.finish_minimal(offset);

        buffer.push(MaybeUninit::new(CommandTag::LoadScene as u8));
        write_table_event_bytes_in_place(buffer, fbb.finished_data());
    }

    pub fn command_spawn<'a, F>(&self, entity_id: EntityId, components_len: usize, f: F)
    where
        F: Fn(usize) -> ComponentData<'a>,
    {
        let buffer = unsafe { &mut *self.command_data[P::Executor::thread_index()].get() };

        buffer.push(MaybeUninit::new(CommandTag::Spawn as u8));
        write_add_components_in_place(buffer, entity_id, components_len, f);
    }

    pub fn drain_commands<F>(&mut self, mut f: F)
    where
        F: FnMut(CommandRef<'_>),
    {
        for thread_buffer in self.command_data.as_mut_slice() {
            let thread_buffer = thread_buffer.get_mut();
            let mut buffer = thread_buffer.as_slice();

            while !buffer.is_empty() {
                let event_tag: CommandTag = unsafe {
                    buffer[0]
                        .assume_init()
                        .try_into()
                        .expect("failed `u8` cast to `EventTag`, command events corrupted")
                };

                buffer = &buffer[1..];

                buffer = match event_tag {
                    CommandTag::AddComponents => {
                        let (buffer_remainder, event) = unsafe { decode_add_components(buffer) };
                        f(CommandRef::AddComponents(event));
                        buffer_remainder
                    }
                    CommandTag::Despawn => {
                        let (buffer_remainder, event) =
                            unsafe { decode_struct_event::<Despawn>(buffer) };
                        f(CommandRef::Despawn(event));
                        buffer_remainder
                    }
                    CommandTag::LoadScene => {
                        let (buffer_remainder, event) =
                            unsafe { decode_table_event::<LoadScene<'_>>(buffer) };
                        f(CommandRef::LoadScene(event));
                        buffer_remainder
                    }
                    CommandTag::RemoveComponents => {
                        let (buffer_remainder, event) =
                            unsafe { decode_table_event::<RemoveComponents<'_>>(buffer) };
                        f(CommandRef::RemoveComponents(event));
                        buffer_remainder
                    }
                    CommandTag::SetEntityLabel => {
                        let (buffer_remainder, event) =
                            unsafe { decode_table_event::<SetEntityLabel<'_>>(buffer) };
                        f(CommandRef::SetEntityLabel(event));
                        buffer_remainder
                    }
                    CommandTag::SetParent => {
                        let (buffer_remainder, event) =
                            unsafe { decode_struct_event::<SetParent>(buffer) };
                        f(CommandRef::SetParent(event));
                        buffer_remainder
                    }
                    CommandTag::SetSystemEnabled => {
                        let (buffer_remainder, event) =
                            unsafe { decode_table_event::<SetSystemEnabled<'_>>(buffer) };
                        f(CommandRef::SetSystemEnabled(event));
                        buffer_remainder
                    }
                    CommandTag::Spawn => {
                        let (buffer_remainder, event) = unsafe { decode_add_components(buffer) };
                        f(CommandRef::Spawn(event));
                        buffer_remainder
                    }
                }
            }

            thread_buffer.clear();
        }
    }
}

impl<P: Platform> PlatformEventDelegate<'_, P> {
    /// # Safety
    ///
    /// Event data must be valid.
    pub unsafe fn send(&mut self, ident: &CStr, data: &[u8]) {
        // just get the first buffer and use it for writing

        let storage = self
            .platform_event_data
            .entry(ident.into())
            .or_insert_with(|| {
                let per_thread_buffers = (0..P::Executor::available_parallelism().get())
                    .map(|_| {
                        ThreadBuffer {
                            data: Vec::new(),
                            event_count: 0,
                        }
                        .into()
                    })
                    .collect();

                let inner = EventWriterStorageInner { per_thread_buffers }.into();

                EventWriterStorage {
                    inner,
                    marker: PhantomData,
                }
            });

        let buffer = storage.inner.get_mut().per_thread_buffers[0].get_mut();

        unsafe { write_event(data, buffer) };
    }

    pub fn clear(&mut self) {
        for data in self.platform_event_data.values_mut() {
            data.inner.get_mut().per_thread_buffers[0].get_mut().clear();
        }
    }

    pub fn storage(&self, ident: &CStr) -> Option<EventWriterStorageRef<'_>> {
        self.platform_event_data
            .get(ident)
            .map(|storage| storage.borrow())
    }
}

pub struct AddComponents<'a> {
    pub entity_id: &'a EntityId,
    component_ids: &'a [ComponentId],
    component_data: &'a [MaybeUninit<u8>],
}

/// A bundle of components associated with a spawn event.
pub trait SpawnComponentData {
    /// Returns all IDs for all components in this bundle, in sorted order.
    fn sorted_component_ids(&self) -> &[ComponentId];

    /// Returns raw component data for the given `component_id`. If the
    /// component does not exist in this bundle, `None` is returned.
    fn component_data(&self, component_id: ComponentId) -> Option<&[MaybeUninit<u8>]>;
}

impl SpawnComponentData for AddComponents<'_> {
    fn sorted_component_ids(&self) -> &[ComponentId] {
        self.component_ids
    }

    fn component_data(&self, component_id: ComponentId) -> Option<&[MaybeUninit<u8>]> {
        let index = self
            .component_ids
            .iter()
            .position(|&cid| cid == component_id)?;

        let mut i = 0;
        for _ in 0..index {
            let mut bytes = [MaybeUninit::uninit(); 2];
            bytes.copy_from_slice(&self.component_data[i..i + 2]);
            let len = usize::from(u16::from_ne_bytes(
                bytes.map(|a| unsafe { a.assume_init() }),
            ));
            i += size_of::<u16>() + len;
            i = i.next_multiple_of(align_of::<u16>());
        }

        let mut bytes = [MaybeUninit::uninit(); 2];
        bytes.copy_from_slice(&self.component_data[i..i + 2]);
        let len = usize::from(u16::from_ne_bytes(
            bytes.map(|a| unsafe { a.assume_init() }),
        ));
        i += size_of::<u16>();

        Some(&self.component_data[i..i + len])
    }
}

pub struct SetSystemEnabledEvent {
    pub system_name: String,
    pub enabled: bool,
}

#[derive(Debug)]
pub struct ComponentData<'a> {
    pub component_id: ComponentId,
    pub component_data: &'a [MaybeUninit<u8>],
}

/// Stores events of a specific type from a specific writer
pub struct EventWriterStorage<P: Platform> {
    inner: AtomicRefCell<EventWriterStorageInner>,
    marker: PhantomData<P>,
}

pub struct EventWriterStorageInner {
    // todo(optimization): thread count is fixed, make the Vec a DST
    per_thread_buffers: Vec<UnsafeCell<ThreadBuffer>>,
}

struct ThreadBuffer {
    // we're assuming flatbuffer data alignment is at most 8 (u64). Is this always true?
    data: Vec<u64>,
    event_count: usize,
}

impl ThreadBuffer {
    fn clear(&mut self) {
        self.data.clear();
        self.event_count = 0;
    }
}

/// SAFETY: access is checked via `AtomicRefCell`, and when writing threads only access their thread buffer
unsafe impl Sync for EventWriterStorageInner {}

pub struct EventWriterStorageRef<'a> {
    inner: AtomicRef<'a, EventWriterStorageInner>,
}

pub struct EventWriterStorageMut<'a, P: Platform> {
    inner: AtomicRefMut<'a, EventWriterStorageInner>,
    marker: PhantomData<P>,
}

impl<P: Platform> EventWriterStorage<P> {
    pub fn borrow(&self) -> EventWriterStorageRef<'_> {
        EventWriterStorageRef {
            inner: self.inner.borrow(),
        }
    }

    pub fn borrow_mut(&self) -> EventWriterStorageMut<'_, P> {
        EventWriterStorageMut {
            inner: self.inner.borrow_mut(),
            marker: PhantomData,
        }
    }
}

impl EventWriterStorageRef<'_> {
    /// This function returns a pointer to the event data length (u64), followed by the event data 8 bytes later.
    /// If the index is out of bounds, it will return null.
    pub fn read_event(&self, index: usize) -> *const u64 {
        let mut i = 0;

        for buffer in &self.inner.per_thread_buffers {
            let mut buffer = unsafe { (*buffer.get()).data.as_slice() };

            while !buffer.is_empty() {
                if i == index {
                    return buffer.as_ptr();
                }

                let len = (buffer[0] as usize).div_ceil(size_of::<u64>());
                buffer = &buffer[len + 1..]; // + 1 to account for u64-encoded len
                i += 1;
            }
        }

        ptr::null()
    }

    pub fn count(&self) -> usize {
        self.inner
            .per_thread_buffers
            .iter()
            .map(|buffer| unsafe { (*buffer.get()).event_count })
            .sum()
    }
}

impl<P: Platform> EventWriterStorageMut<'_, P> {
    /// # Safety
    ///
    /// Event data must be valid.
    #[inline]
    pub unsafe fn write_module_event(&self, data: &[u8]) {
        let buffer =
            unsafe { &mut *self.inner.per_thread_buffers[P::Executor::thread_index()].get() };

        unsafe { write_event(data, buffer) };
    }

    pub fn clear(&mut self) {
        for buffer in &mut self.inner.per_thread_buffers {
            buffer.get_mut().clear();
        }
    }
}

unsafe fn write_event(data: &[u8], buffer: &mut ThreadBuffer) {
    // write the event data len converted into a u64, followed by the event data 8 bytes later
    // this ensures 8-byte alignment for the event data

    buffer.data.push(data.len().try_into().unwrap());

    // finagle the u8 slice into a u64 iterator

    buffer.data.extend(
        data.chunks(size_of::<u64>())
            .map(|chunk| u64::from_ne_bytes(from_fn(|i| *chunk.get(i).unwrap_or(&0)))),
    );

    buffer.event_count += 1;
}

fn write_struct_event_in_place<T: Push>(buffer: &mut AVec<MaybeUninit<u8>>, event: T) {
    let offset = buffer.len();
    buffer.resize(offset + T::size(), MaybeUninit::new(0));

    unsafe {
        let slice = slice_assume_init_mut(&mut buffer[offset..]);
        event.push(slice, T::size());
    }
}

/// # Safety
///
/// Caller must ensure that `buffer` was initialized as the expected event type.
unsafe fn decode_struct_event<'a, T>(
    buffer: &'a [MaybeUninit<u8>],
) -> (&'a [MaybeUninit<u8>], T::Inner)
where
    T: Follow<'a> + Push,
{
    let size = size_of::<T>();
    let (event_data, remainder) = buffer.split_at(size);
    unsafe {
        let event_data = slice_assume_init_ref(event_data);
        (remainder, T::follow(event_data, 0))
    }
}

fn write_table_event_bytes_in_place(buffer: &mut AVec<MaybeUninit<u8>>, event_bytes: &[u8]) {
    // write the event length, since table events have variable sizes
    write_struct_event_in_place(buffer, event_bytes.len() as u32);

    let offset = buffer.len();
    buffer.resize(offset + event_bytes.len(), MaybeUninit::new(0));

    unsafe {
        let slice = slice_assume_init_mut(&mut buffer[offset..]);
        slice.copy_from_slice(event_bytes);
    }
}

/// # Safety
///
/// Caller must ensure that `buffer` was initialized as the expected event type.
unsafe fn decode_table_event<'a, T>(
    buffer: &'a [MaybeUninit<u8>],
) -> (&'a [MaybeUninit<u8>], T::Inner)
where
    T: Follow<'a> + 'a,
{
    unsafe {
        let (remainder, size) = decode_struct_event::<u32>(buffer);
        let (event_data, remainder) = remainder.split_at(size as usize);
        let event_data = slice_assume_init_ref(event_data);
        (remainder, root_unchecked::<T>(event_data))
    }
}

fn write_add_components_in_place<'a, F>(
    buffer: &mut AVec<MaybeUninit<u8>>,
    entity_id: EntityId,
    components_len: usize,
    f: F,
) where
    F: Fn(usize) -> ComponentData<'a>,
{
    fn write_aligned_as<T: Copy + 'static>(val: &T, data: &mut AVec<MaybeUninit<u8>>) {
        unsafe {
            let offset = data.as_ptr().add(data.len()).align_offset(align_of::<T>());
            let new_len = data.len() + offset;
            data.resize(new_len, MaybeUninit::uninit());
            data.extend_from_slice(slice::from_raw_parts(
                (val as *const T).cast::<MaybeUninit<u8>>(),
                size_of::<T>(),
            ));
        }
    }

    fn write_aligned_as_preallocated<'a, T: Copy>(
        val: &T,
        data: &'a mut [MaybeUninit<u8>],
    ) -> &'a mut [MaybeUninit<u8>] {
        let offset = data.as_ptr().align_offset(align_of::<T>());
        let write_len = offset + size_of::<T>();

        assert!(
            write_len <= data.len(),
            "spawn: ran out of preallocated space, check module component data passed to spawn()"
        );

        unsafe {
            ptr::write(data.as_mut_ptr().add(offset).cast::<T>(), *val);
        }

        &mut data[write_len..]
    }

    // write entity id and the number of components

    write_aligned_as::<EntityId>(&entity_id, buffer);
    write_aligned_as::<usize>(&components_len, buffer);

    // Write component IDs, and then sum the total component data section byte
    // size. The data for each component contains a length (u16) + the component
    // data itself.

    let component_ids_start = buffer.len().next_multiple_of(size_of::<ComponentId>());
    let component_data_start = component_ids_start + components_len * size_of::<ComponentId>();
    let mut component_data_len: usize = 0;

    for i in 0..components_len {
        let component = f(i);
        write_aligned_as::<ComponentId>(&component.component_id, buffer);

        // byte size needed for component = length (16-byte-aligned) + data
        component_data_len += size_of::<u16>() + component.component_data.len();
        component_data_len = component_data_len.next_multiple_of(size_of::<u16>());
    }

    // reserve space for component data entries

    buffer.resize(
        component_data_start + component_data_len,
        MaybeUninit::uninit(),
    );

    // split data buffer into component_ids section and uninitialized component_data section

    let (_, ids_and_data) = buffer.split_at_mut(component_ids_start);

    let (component_ids, mut component_data) =
        ids_and_data.split_at_mut(components_len * size_of::<ComponentId>());

    // sort component IDs

    let component_ids = unsafe {
        slice::from_raw_parts_mut(
            component_ids.as_mut_ptr().cast::<ComponentId>(),
            components_len,
        )
    };

    component_ids.sort_unstable();

    // for each component, write component data length + data blob

    for component_id in &*component_ids {
        let component = (0..components_len)
            .find_map(|i| {
                let component = f(i);
                if component.component_id == *component_id {
                    Some(component)
                } else {
                    None
                }
            })
            .unwrap();

        // write length
        component_data = write_aligned_as_preallocated::<u16>(
            &component.component_data.len().try_into().unwrap(),
            component_data,
        );

        assert!(
            component.component_data.len() <= component_data.len(),
            "spawn: ran out of preallocated space, check module component data passed to spawn()"
        );

        // write data
        component_data[..component.component_data.len()].copy_from_slice(component.component_data);
        component_data = &mut component_data[component.component_data.len()..];
    }
}

/// # Safety
///
/// Caller must ensure that `buffer` was initialized as the expected event type.
unsafe fn decode_add_components(
    buffer: &[MaybeUninit<u8>],
) -> (&[MaybeUninit<u8>], AddComponents<'_>) {
    unsafe fn read_aligned_as<T>(data: &[MaybeUninit<u8>]) -> (&T, &[MaybeUninit<u8>]) {
        let offset = data.as_ptr().align_offset(align_of::<T>());
        let val = unsafe { &*data.as_ptr().add(offset).cast::<T>() };

        (val, &data[offset + size_of::<T>()..])
    }

    unsafe fn read_slice_aligned_as<T>(data: &[MaybeUninit<u8>]) -> (&[T], &[MaybeUninit<u8>]) {
        let (len, data) = unsafe { read_aligned_as::<usize>(data) };

        let offset = data.as_ptr().align_offset(align_of::<T>());
        let val = unsafe { slice::from_raw_parts(data.as_ptr().add(offset).cast::<T>(), *len) };

        (val, &data[offset + len * size_of::<T>()..])
    }

    unsafe {
        let (entity_id, buffer) = read_aligned_as::<EntityId>(buffer);
        let (component_ids, mut buffer) = read_slice_aligned_as::<ComponentId>(buffer);

        let align_offset = buffer.as_ptr().align_offset(align_of::<u16>());
        buffer = &buffer[align_offset..];

        let data_ptr = buffer.as_ptr();

        let mut i: usize = 0;
        for _ in 0..component_ids.len() {
            i = i.next_multiple_of(align_of::<u16>());
            let size = *data_ptr.add(i).cast::<u16>();
            i += size_of::<u16>() + usize::from(size);
        }

        let component_data = slice::from_raw_parts(data_ptr, i);

        // round up `i` to next multiple of 2, because the spawn writer always
        // writes a multiple of 2 bytes
        let remainder = &buffer[i.next_multiple_of(align_of::<u16>())..];

        let event = AddComponents {
            entity_id,
            component_ids,
            component_data,
        };

        (remainder, event)
    }
}

/// This function is copied from the unstable standard library function of the
/// same name.
///
/// # Safety
///
/// The caller must ensure that the slice is initialized.
const unsafe fn slice_assume_init_ref(slice: &[MaybeUninit<u8>]) -> &[u8] {
    // SAFETY: casting `slice` to a `*const [T]` is safe since the caller guarantees that
    // `slice` is initialized, and `MaybeUninit` is guaranteed to have the same layout as `T`.
    // The pointer obtained is valid since it refers to memory owned by `slice` which is a
    // reference and thus guaranteed to be valid for reads.
    unsafe { &*(slice as *const [MaybeUninit<u8>] as *const [u8]) }
}

/// This function is copied from the unstable standard library function of the
/// same name.
///
/// # Safety
///
/// The caller must ensure that the slice is initialized.
const unsafe fn slice_assume_init_mut(slice: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    // SAFETY: casting `slice` to a `*const [T]` is safe since the caller guarantees that
    // `slice` is initialized, and `MaybeUninit` is guaranteed to have the same layout as `T`.
    // The pointer obtained is valid since it refers to memory owned by `slice` which is a
    // reference and thus guaranteed to be valid for reads.
    unsafe { &mut *(slice as *mut [MaybeUninit<u8>] as *mut [u8]) }
}

#[cfg(test)]
mod test {
    use std::slice;

    use platform::test::TestPlatform;

    use super::*;

    const EVENT_IDENT: &CStr = c"event";
    const SYSTEM_NAME: &str = "test_system";

    #[test]
    fn write_7() {
        let mut event_manager = EventManager::<TestPlatform>::default();
        event_manager.register_module_event_writer(EVENT_IDENT, SYSTEM_NAME.to_owned());

        // write

        unsafe {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            borrow.write_module_event(&[0, 1, 2, 3, 4, 5, 6]);
        }

        // check storage length

        {
            let mut borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            assert_eq!(borrow.inner.per_thread_buffers[0].get_mut().data.len(), 2);
        }

        // check event count

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();
            assert_eq!(borrow.count(), 1);
        }

        // read back event

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();

            let data = unsafe {
                let ptr = borrow.read_event(0);
                let len = ptr.read() as usize;
                slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len)
            };

            assert_eq!(data, &[0, 1, 2, 3, 4, 5, 6]);
        }
    }

    #[test]
    fn write_8() {
        let mut event_manager = EventManager::<TestPlatform>::default();
        event_manager.register_module_event_writer(EVENT_IDENT, SYSTEM_NAME.to_owned());

        // write

        unsafe {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            borrow.write_module_event(&[0, 1, 2, 3, 4, 5, 6, 7]);
        }

        // check storage length

        {
            let mut borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            assert_eq!(borrow.inner.per_thread_buffers[0].get_mut().data.len(), 2);
        }

        // check event count

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();
            assert_eq!(borrow.count(), 1);
        }

        // read back event

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();

            let data = unsafe {
                let ptr = borrow.read_event(0);
                let len = ptr.read() as usize;
                slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len)
            };

            assert_eq!(data, &[0, 1, 2, 3, 4, 5, 6, 7]);
        }
    }

    #[test]
    fn write_7_twice() {
        let mut event_manager = EventManager::<TestPlatform>::default();
        event_manager.register_module_event_writer(EVENT_IDENT, SYSTEM_NAME.to_owned());

        // write

        unsafe {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            borrow.write_module_event(&[0, 1, 2, 3, 4, 5, 6]);
            borrow.write_module_event(&[6, 5, 4, 3, 2, 1, 0]);
        }

        // check storage length

        {
            let mut borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            assert_eq!(borrow.inner.per_thread_buffers[0].get_mut().data.len(), 4);
        }

        // check event count

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();
            assert_eq!(borrow.count(), 2);
        }

        // read back events

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();

            let data = unsafe {
                let ptr = borrow.read_event(0);
                let len = ptr.read() as usize;
                slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len)
            };

            assert_eq!(data, &[0, 1, 2, 3, 4, 5, 6]);

            let data = unsafe {
                let ptr = borrow.read_event(1);
                let len = ptr.read() as usize;
                slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len)
            };

            assert_eq!(data, &[6, 5, 4, 3, 2, 1, 0]);
        }
    }

    #[test]
    fn write_15() {
        let mut event_manager = EventManager::<TestPlatform>::default();
        event_manager.register_module_event_writer(EVENT_IDENT, SYSTEM_NAME.to_owned());

        // write

        unsafe {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            borrow.write_module_event(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        }

        // check storage length

        {
            let mut borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow_mut();
            assert_eq!(borrow.inner.per_thread_buffers[0].get_mut().data.len(), 3);
        }

        // check event count

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();
            assert_eq!(borrow.count(), 1);
        }

        // read back event

        {
            let borrow = event_manager
                .module_event_storage(EVENT_IDENT, SYSTEM_NAME)
                .unwrap()
                .borrow();

            let data = unsafe {
                let ptr = borrow.read_event(0);
                let len = ptr.read() as usize;
                slice::from_raw_parts(ptr.offset(1).cast::<u8>(), len)
            };

            assert_eq!(data, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        }
    }
}
