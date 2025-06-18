//! Game executable with tooling IPC.

use clap::Parser;
use log::LevelFilter;
use native_common::{
    GpuWeb, JsOptions, NativeGameEngine,
    game_engine::void_public::{
        self,
        event::input::{
            KeyboardInput, MouseButtonInput, MousePosition, MouseScroll, WindowResized,
        },
    },
    send_struct_event,
};
use native_ipc::tooling::ToolingIpc;
use platform_native::{EnvArgs, to_engine_keyboard_input};
use pollster::FutureExt;
use winit::{
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};

fn main() {
    #[cfg(debug_assertions)]
    env_logger::builder().filter_level(LevelFilter::Info).init();

    #[cfg(not(debug_assertions))]
    env_logger::builder().filter_level(LevelFilter::Warn).init();

    let env_args = EnvArgs::parse();

    let mut tooling_ipc = ToolingIpc::on_port(9002).unwrap();

    let event_loop = EventLoop::new().unwrap();
    let window = WindowBuilder::new()
        .with_title(&env_args.window_title)
        .build(&event_loop)
        .unwrap();

    let width = window.inner_size().width;
    let height = window.inner_size().height;

    let js_options = JsOptions {
        start_js_inspector: env_args.start_js_inspector,
        js_inspector_port: env_args.js_inspector_port,
        modules_dir: env_args.modules_dir,
    };

    let gpu = GpuWeb::new(width, height, window).block_on().unwrap();

    let mut engine = NativeGameEngine::new(gpu, width, height, &js_options);

    event_loop.set_control_flow(ControlFlow::Poll);

    event_loop
        .run(move |event, window| match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                window.exit();
            }
            Event::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. },
                ..
            } => {
                send_struct_event!(
                    engine,
                    MousePosition,
                    MousePosition::new(position.x as f32, position.y as f32)
                );
            }
            Event::WindowEvent {
                event: WindowEvent::MouseInput { state, button, .. },
                ..
            } => {
                let button = match button {
                    MouseButton::Left => void_public::event::input::MouseButton::Left,
                    MouseButton::Right => void_public::event::input::MouseButton::Right,
                    MouseButton::Middle => void_public::event::input::MouseButton::Middle,
                    _ => return,
                };

                let state = match state {
                    ElementState::Pressed => void_public::event::input::ElementState::Pressed,
                    ElementState::Released => void_public::event::input::ElementState::Released,
                };

                send_struct_event!(
                    engine,
                    MouseButtonInput,
                    MouseButtonInput::new(button, state)
                );
            }
            Event::WindowEvent {
                event: WindowEvent::MouseWheel { delta, .. },
                ..
            } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(lines_x, lines_y) => {
                        (lines_x as f64 * 50., lines_y as f64 * 50.)
                    }
                    MouseScrollDelta::PixelDelta(pixels) => (pixels.x, pixels.y),
                };

                send_struct_event!(engine, MouseScroll, MouseScroll::new(x as f32, y as f32));
            }
            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event, .. },
                ..
            } => {
                if let Ok(input) = to_engine_keyboard_input(&event) {
                    send_struct_event!(engine, KeyboardInput, input);
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                send_struct_event!(
                    engine,
                    WindowResized,
                    WindowResized::new(size.width, size.height, true)
                );
            }
            Event::AboutToWait => {
                let should_run_frame = tooling_ipc.process_ipc_messages(&mut engine);
                if should_run_frame {
                    engine.frame();
                }
            }
            _ => {}
        })
        .unwrap();
}
