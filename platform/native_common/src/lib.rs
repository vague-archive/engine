#![allow(clippy::macro_metavars_in_unsafe)] // send_struct_event false positive.

//! This crate provides common functionality across all native platform
//! entrypoints. It is not a formal platform in itself, as it does not contain a
//! `main` function. It is intended to be used by native entrypoints, such as
//! `platform-native` and `tauri-shell`, to encourage code reuse.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    fs::read,
    io,
    mem::{MaybeUninit, transmute},
    ops::{Deref, DerefMut},
    path::Path,
    pin::Pin,
    slice,
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    time::Instant,
};

use ecs_module::register_ecs_modules;
pub use game_engine;
use game_engine::{
    platform,
    void_public::{
        self,
        callable::TaskId,
        event::input::{
            GamepadAxis, GamepadButton, GamepadConnected, GamepadConnectedArgs, GamepadDisconnected,
        },
        event_name,
    },
};
use gilrs::{Axis, Button, EventType, Gilrs};
pub use gpu_web::{GpuWeb, RenderViewport};
pub use js::JsOptions;
use js::{SyncIsolate, register_js_ecs_modules};
use libloading::{Library, Symbol};
use platform_library::register_platform_libraries;
use pollster::FutureExt;
use tokio::runtime::Runtime as TokioRuntime;

mod deno_op;
pub mod ecs_module;
mod js;
mod platform_library;
mod typescript_loader;

/// Sends a simple struct-type flatbuffers event.
#[macro_export]
macro_rules! send_struct_event {
    ($engine:expr, $type:ident, $event:expr) => {
        let mut builder = ::flatbuffers::FlatBufferBuilder::new();
        let offset: ::flatbuffers::WIPOffset<$type> = builder.push($event);
        builder.finish_minimal(offset);

        // Safe because we specify `WIPOffset<$type>`.
        unsafe {
            $engine
                .platform_event_delegate()
                .send(void_public::event_name!($type), builder.finished_data());
        }
    };
}

pub type GameEngine = game_engine::GameEngine<Platform, GpuWeb>;

pub struct NativeGameEngine {
    engine: GameEngine,
    js_isolate: Arc<SyncIsolate>,
    tokio_runtime: TokioRuntime,
    async_completions_receiver: Receiver<AsyncCompletion>,
    platform_events_receiver: Receiver<PlatformEvent>,
    gilrs: Gilrs,
    prev_frame_instant: Instant,
}

impl NativeGameEngine {
    pub fn new(gpu: GpuWeb, width: u32, height: u32, js_options: &JsOptions) -> Self {
        let (sender, async_completions_receiver) = channel();
        *ASYNC_COMPLETION_QUEUE.lock().unwrap() = Some(sender);

        let (sender, platform_events_receiver) = channel();
        *PLATFORM_EVENT_QUEUE.lock().unwrap() = Some(sender);

        let mut engine = GameEngine::new(Executor, width, height, gpu);

        let tokio_runtime = TokioRuntime::new().unwrap();
        let js_isolate = Arc::new(SyncIsolate::new(js_options, &tokio_runtime));

        register_platform_libraries(&mut engine);
        register_ecs_modules(&mut engine);
        register_js_ecs_modules(&mut engine, &js_isolate, &tokio_runtime, js_options);

        let gilrs = Gilrs::new().expect("could not initialize gamepad library");

        Self {
            engine,
            js_isolate,
            tokio_runtime,
            async_completions_receiver,
            platform_events_receiver,
            gilrs,
            prev_frame_instant: Instant::now(),
        }
    }

    pub fn frame(&mut self) {
        self.poll_controller_input();

        drain_async_completion_queue(&self.async_completions_receiver, &mut self.engine);
        drain_platform_event_queue(&self.platform_events_receiver, &mut self.engine);

        let now = Instant::now();
        let delta_time = now.duration_since(self.prev_frame_instant).as_secs_f32();

        self.engine.frame(delta_time);

        self.js_isolate.lock().run_event_loop(&self.tokio_runtime);

        self.prev_frame_instant = now;
    }

    fn poll_controller_input(&mut self) {
        while let Some(event) = self.gilrs.next_event() {
            match &event.event {
                EventType::ButtonChanged(button, value, _) => {
                    let Some(index) = to_web_standard_layout(button) else {
                        log::warn!("unsupported gamepad button {:?}", button);
                        continue;
                    };

                    send_struct_event!(
                        self.engine,
                        GamepadButton,
                        GamepadButton::new(
                            usize::from(event.id).try_into().unwrap(),
                            index.try_into().unwrap(),
                            *value
                        )
                    );
                }
                EventType::AxisChanged(axis, value, _) => {
                    let Some((index, value)) = to_web_standard_axis_and_value(axis, *value) else {
                        log::warn!("unsupported gamepad axis: {:?}", axis);
                        continue;
                    };

                    send_struct_event!(
                        self.engine,
                        GamepadAxis,
                        GamepadAxis::new(
                            usize::from(event.id).try_into().unwrap(),
                            index.try_into().unwrap(),
                            value
                        )
                    );
                }
                EventType::Connected => {
                    // hardcode for standard layout controller
                    let gamepad = self.gilrs.gamepad(event.id);

                    let mut builder = flatbuffers::FlatBufferBuilder::new();

                    let name = builder.create_string(gamepad.name());

                    let event = GamepadConnected::create(
                        &mut builder,
                        &GamepadConnectedArgs {
                            id: usize::from(event.id).try_into().unwrap(),
                            button_count: 17,
                            axis_count: 4,
                            name: Some(name),
                        },
                    );

                    builder.finish_minimal(event);

                    unsafe {
                        self.engine
                            .platform_event_delegate()
                            .send(event_name!(GamepadConnected), builder.finished_data());
                    }
                }
                EventType::Disconnected => {
                    send_struct_event!(
                        self.engine,
                        GamepadDisconnected,
                        GamepadDisconnected::new(usize::from(event.id).try_into().unwrap(),)
                    );
                }
                _ => {}
            }
        }
    }
}

impl Deref for NativeGameEngine {
    type Target = GameEngine;

    fn deref(&self) -> &Self::Target {
        &self.engine
    }
}

impl DerefMut for NativeGameEngine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.engine
    }
}

pub struct Platform;

impl platform::Platform for Platform {
    type Executor = Executor;
    type Filesystem = Filesystem;
}

/// Maps a gamepad button to its standard button index
/// <https://w3c.github.io/gamepad/#remapping/>
fn to_web_standard_layout(button: &Button) -> Option<usize> {
    let index = match button {
        Button::South => 0,
        Button::East => 1,
        Button::North => 3,
        Button::West => 2,
        Button::LeftTrigger => 4,
        Button::LeftTrigger2 => 6,
        Button::RightTrigger => 5,
        Button::RightTrigger2 => 7,
        Button::Select => 8,
        Button::Start => 9,
        Button::Mode => 16,
        Button::LeftThumb => 10,
        Button::RightThumb => 11,
        Button::DPadUp => 12,
        Button::DPadDown => 13,
        Button::DPadLeft => 14,
        Button::DPadRight => 15,
        _ => {
            return None;
        }
    };

    Some(index)
}

/// Maps a gamepad axis to its standard axis index, and inverts Y values
fn to_web_standard_axis_and_value(axis: &Axis, value: f32) -> Option<(usize, f32)> {
    let index = match axis {
        Axis::LeftStickX => (0, value),
        Axis::LeftStickY => (1, -value),
        Axis::RightStickX => (2, value),
        Axis::RightStickY => (3, -value),
        _ => {
            return None;
        }
    };

    Some(index)
}

pub struct Executor;

impl platform::Executor for Executor {
    fn available_parallelism() -> std::num::NonZero<usize> {
        std::num::NonZero::new(1).unwrap()
    }

    #[inline]
    fn thread_index() -> usize {
        0
    }

    fn parallel_iter<F>(len: usize, f: F)
    where
        F: Fn(usize, usize) + Send + Sync,
    {
        for i in 0..len {
            f(i, 0);
        }
    }

    fn execute_blocking(&mut self, future: Pin<&mut (dyn Future<Output = ()> + Send)>) {
        future.block_on();
    }
}

pub struct Filesystem;

impl platform::Filesystem for Filesystem {
    fn read_async<P, T>(path: P, user_data: Arc<T>, completion: fn(Arc<T>, io::Result<Vec<u8>>))
    where
        P: AsRef<Path>,
        Arc<T>: Send,
    {
        // todo: move to background thread
        completion(user_data, read(path));
    }
}

/// Fetches a symbol and transmutes the lifetime to be 'static. This lifetime
/// transmutation is safe, as long as the library is not dropped during the
/// lifetime of the symbol. We ensure this by storing the library in any struct
/// which store procedures with a "static" lifetime.
unsafe fn get_procedure<T>(library: &Library, procedure_name: &CStr) -> Symbol<'static, T> {
    match unsafe { library.get::<T>(procedure_name.to_bytes_with_nul()) } {
        Ok(procedure) => unsafe { transmute::<Symbol<'_, T>, Symbol<'static, T>>(procedure) },
        Err(err) => {
            panic!("{err}: Failed to load procedure {procedure_name:?} from {library:?}",);
        }
    }
}

/// This queue tracks platform library completion events, which are applied at
/// the start of the next frame.
static ASYNC_COMPLETION_QUEUE: Mutex<Option<Sender<AsyncCompletion>>> = Mutex::new(None);

/// This queue tracks platform events which are received during frame processing,
/// and are deferred until the start of the next frame.
static PLATFORM_EVENT_QUEUE: Mutex<Option<Sender<PlatformEvent>>> = Mutex::new(None);

struct AsyncCompletion {
    task_id: TaskId,
    return_value: Box<[MaybeUninit<u8>]>,
}

struct PlatformEvent {
    ident: CString,
    data: Box<[u8]>,
}

/// Exposed to platform libraries. Enqueues async task completions for deferred submission to the engine.
///
/// # Safety
///
/// Casting slice from raw parts input could be unsafe because we don't have any guarantees about the pointer
unsafe extern "C" fn async_task_complete_callback(
    task_id: TaskId,
    return_value_ptr: *const c_void,
    return_value_len: usize,
) {
    let return_value =
        unsafe { slice::from_raw_parts(return_value_ptr.cast(), return_value_len).into() };

    let completion = AsyncCompletion {
        task_id,
        return_value,
    };

    ASYNC_COMPLETION_QUEUE
        .lock()
        .unwrap()
        .as_ref()
        .expect("async completion queue not initialized")
        .send(completion)
        .expect("could not enqueue async completion");
}

/// Exposed to platform libraries. Enqueues platform events for deferred submission to the engine.
///
/// # Safety
///
/// Casting slice from raw parts input could be unsafe because we don't have any guarantees about the pointer
unsafe extern "C" fn platform_event_callback(
    event_identifier: *const c_char,
    event_data_ptr: *const c_void,
    event_data_len: usize,
) {
    let ident = unsafe { CStr::from_ptr(event_identifier).to_owned() };
    let data = unsafe { slice::from_raw_parts(event_data_ptr.cast(), event_data_len).into() };

    let event = PlatformEvent { ident, data };

    PLATFORM_EVENT_QUEUE
        .lock()
        .unwrap()
        .as_ref()
        .expect("platform event queue not initialized")
        .send(event)
        .expect("could not enqueue platform event");
}

fn drain_async_completion_queue(receiver: &Receiver<AsyncCompletion>, engine: &mut GameEngine) {
    for completion in receiver.try_iter() {
        unsafe {
            engine.complete_async_task(completion.task_id, completion.return_value);
        }
    }
}

fn drain_platform_event_queue(receiver: &Receiver<PlatformEvent>, engine: &mut GameEngine) {
    for event in receiver.try_iter() {
        unsafe {
            engine
                .platform_event_delegate()
                .send(&event.ident, &event.data);
        }
    }
}
