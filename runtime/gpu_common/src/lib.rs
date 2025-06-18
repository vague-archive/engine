use std::{
    cell::UnsafeCell,
    fmt::Debug,
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use game_ecs::{ComponentBundle, ComponentRegistry, CpuFrameData, GpuFrameData, PartitionIndex};
use platform::{EcsModule, Platform};
use void_public::{ComponentId, graphics::TextureId};

pub type GpuResourceId = u16;

pub trait Gpu: GpuFrameData + std::fmt::Debug + Sized {
    type Error: Debug;

    const MULTI_BUFFERED: bool;

    fn multi_buffer_count(&self) -> NonZeroUsize;

    fn window_resized(
        &mut self,
        width: u32,
        height: u32,
        component_registry: &ComponentRegistry,
        cpu_data: &mut CpuFrameData,
    );

    // startup

    /// Loads a texture from the file byte array `data`. Returns the dimensions of the texture.
    #[allow(clippy::too_many_arguments)]
    fn register_preloaded_texture(
        &mut self,
        cpu_data: &mut CpuFrameData,
        component_registry: &ComponentRegistry,
        texture_id: TextureId,
        path: String,
        data: Vec<u8>,
        width_and_height: (u32, u32),
        use_atlas: bool,
    ) -> (u32, u32);

    fn register_components(&mut self, component_registry: &mut ComponentRegistry);

    fn register_resources(
        &mut self,
        cpu_data: &mut CpuFrameData,
        component_registry: &mut ComponentRegistry,
    );

    /// Components in a group will be arranged contiguously in the same buffer, ordered and padded
    /// to the C ABI standard. This allows GPU systems to group components according to how they are
    /// accessed together in a shader.
    fn component_groupings(&mut self) -> Vec<Vec<ComponentId>>;

    /// Specifies additional components to add to an entity when source components are added to an entity.
    fn component_bundles(&mut self) -> Vec<ComponentBundle>;

    /// Specifies components which will always be stored contiguously in a single, global, homogenous buffer.
    /// Components belonging to different archetypes will be forced to be stored side-by-side. This is
    /// useful for forcing data used across multiple draw calls, i.e. light positions, to be bound only once.
    fn single_buffer_components(&mut self) -> Vec<ComponentId>;

    /// This is a placeholder for entity relationships. The engine will separate components so that the values
    /// of these components are homogenous in a single archetype. This is useful for i.e. separating different
    /// mesh assets into different archetypes. Components used as keys may not be freely mutable!
    fn component_archetype_keys(&mut self) -> Vec<ComponentId>;

    fn ecs_module<P: Platform>(&self) -> Box<dyn EcsModule>;

    // frame

    fn begin_frame(&mut self, cpu_data: &mut CpuFrameData);

    fn submit_frame(&mut self, cpu_data: &mut CpuFrameData);

    fn destroy(self, cpu_data: &mut CpuFrameData);
}

const HIGH_BIT: usize = !(usize::MAX >> 1);

pub struct DataBufferCell<T> {
    data_buffer: UnsafeCell<T>,
    partition_locks: Vec<Arc<AtomicUsize>>,
}

unsafe impl<T: Send> Send for DataBufferCell<T> {}
unsafe impl<T: Sync> Sync for DataBufferCell<T> {}

impl<T> From<T> for DataBufferCell<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> DataBufferCell<T> {
    pub fn new(val: T) -> Self {
        Self {
            data_buffer: val.into(),
            partition_locks: Vec::from([Default::default()]),
        }
    }

    pub fn allocate_partition(&mut self) {
        self.partition_locks.push(Default::default());
    }

    #[track_caller]
    pub fn borrow(&self, partition: PartitionIndex) -> DataBufferCellRef<T> {
        let borrow = self.partition_locks[usize::from(partition)].clone();
        let old = borrow.fetch_add(1, Ordering::Acquire);

        if old & HIGH_BIT != 0 {
            panic!("already mutably borrowed");
        } else {
            DataBufferCellRef {
                data_buffer: unsafe { NonNull::new_unchecked(self.data_buffer.get()) },
                borrow,
            }
        }
    }

    #[track_caller]
    pub fn borrow_mut(&self, partition: PartitionIndex) -> DataBufferCellRefMut<T> {
        let borrow = self.partition_locks[usize::from(partition)].clone();

        let old = match borrow.compare_exchange(0, HIGH_BIT, Ordering::Acquire, Ordering::Relaxed) {
            Ok(x) | Err(x) => x,
        };

        if old == 0 {
            DataBufferCellRefMut {
                data_buffer: unsafe { NonNull::new_unchecked(self.data_buffer.get()) },
                borrow,
            }
        } else if old & HIGH_BIT == 0 {
            panic!("already immutably borrowed");
        } else {
            panic!("already mutably borrowed");
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.data_buffer.get_mut()
    }
}

pub struct DataBufferCellRef<T> {
    data_buffer: NonNull<T>,
    borrow: Arc<AtomicUsize>,
}

unsafe impl<T: Send> Send for DataBufferCellRef<T> {}
unsafe impl<T: Sync> Sync for DataBufferCellRef<T> {}

impl<T> Deref for DataBufferCellRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data_buffer.as_ptr() }
    }
}

impl<T> Drop for DataBufferCellRef<T> {
    fn drop(&mut self) {
        self.borrow.fetch_sub(1, Ordering::Release);
    }
}

pub struct DataBufferCellRefMut<T> {
    data_buffer: NonNull<T>,
    borrow: Arc<AtomicUsize>,
}

unsafe impl<T: Send> Send for DataBufferCellRefMut<T> {}
unsafe impl<T: Sync> Sync for DataBufferCellRefMut<T> {}

impl<T> Deref for DataBufferCellRefMut<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data_buffer.as_ptr() }
    }
}

impl<T> DerefMut for DataBufferCellRefMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.data_buffer.as_ptr() }
    }
}

impl<T> Drop for DataBufferCellRefMut<T> {
    fn drop(&mut self) {
        self.borrow.store(0, Ordering::Release);
    }
}
