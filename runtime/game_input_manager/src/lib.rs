use binary_writer::BinaryWriter;
use event::{PlatformEventDelegate, platform_event_iter};
use log::{error, warn};
use platform::Platform;
use void_public::{
    event::input::{
        ElementState, GamepadAxis, GamepadButton, GamepadConnected, GamepadDisconnected,
        KeyboardInput, MouseButtonInput, MousePosition, MouseScroll, WindowUnfocused,
    },
    input::InputState,
};

mod binary_writer;

#[cfg(feature = "state_snapshots")]
mod serialize;

#[derive(Default)]
pub struct InputManager {
    buffer: Vec<u8>,
    gamepads: Vec<Gamepad>,
}

#[cfg_attr(feature = "state_snapshots", derive(snapshot::Deserialize))]
#[cfg_attr(feature = "state_snapshots", derive(snapshot::Serialize))]
struct Gamepad {
    id: u8,
    _name: String,
    buttons: Vec<f32>,
    buttons_prev: Vec<f32>,
    axes: Vec<f32>,
}

impl InputManager {
    pub fn read_events<P: Platform>(
        &mut self,
        event_delegate: &PlatformEventDelegate<'_, P>,
        input_state: &mut InputState,
    ) {
        self.prepare_frame(input_state);

        platform_event_iter!(event_delegate, MousePosition, |event| {
            input_state.mouse.cursor_position.x = event.x();
            input_state.mouse.cursor_position.y = event.y();
        });

        platform_event_iter!(event_delegate, MouseButtonInput, |event| {
            let pressed = match event.state() {
                ElementState::Pressed => true,
                ElementState::Released => false,
                _ => unreachable!(),
            };

            input_state.mouse.buttons[event.button()].set_pressed(pressed);
        });

        platform_event_iter!(event_delegate, MouseScroll, |event| {
            input_state.mouse.scroll_delta.x = event.x();
            input_state.mouse.scroll_delta.y = event.y();
        });

        platform_event_iter!(event_delegate, KeyboardInput, |event| {
            let pressed = match event.state() {
                ElementState::Pressed => true,
                ElementState::Released => false,
                _ => unreachable!(),
            };

            input_state.keys[event.key_code()].set_pressed(pressed);
        });

        platform_event_iter!(event_delegate, GamepadConnected, |event| {
            let Ok(id) = event.id().try_into() else {
                error!("gamepad id greater than 255: {}", event.id());
                return;
            };

            if event.button_count() > u8::MAX.into() {
                warn!("gamepad button count > 255");
            }

            if event.axis_count() > u8::MAX.into() {
                warn!("gamepad axis count > 255");
            }

            self.gamepads.push(Gamepad {
                id,
                _name: event.name().unwrap().into(),
                buttons: (0..event.button_count()).map(|_| 0.0).collect(),
                buttons_prev: (0..event.button_count()).map(|_| 0.0).collect(),
                axes: (0..event.axis_count()).map(|_| 0.0).collect(),
            });
        });

        platform_event_iter!(event_delegate, GamepadDisconnected, |event| {
            let Ok(id): Result<u8, _> = event.id().try_into() else {
                error!("gamepad id greater than 255: {}", event.id());
                return;
            };

            let Some(index) = self
                .gamepads
                .iter_mut()
                .position(|gamepad| gamepad.id == id)
            else {
                error!("gamepad id ({id}) not found in connected controllers");
                return;
            };

            self.gamepads.remove(index);
        });

        platform_event_iter!(event_delegate, GamepadButton, |event| {
            let Ok(id): Result<u8, _> = event.id().try_into() else {
                error!("gamepad id greater than 255: {}", event.id());
                return;
            };

            let Some(gamepad) = self.gamepads.iter_mut().find(|gamepad| gamepad.id == id) else {
                error!("gamepad id ({id}) not found in connected controllers");
                return;
            };

            gamepad.buttons[event.index() as usize] = event.value();
        });

        platform_event_iter!(event_delegate, GamepadAxis, |event| {
            let Ok(id): Result<u8, _> = event.id().try_into() else {
                error!("gamepad id greater than 255: {}", event.id());
                return;
            };

            let Some(gamepad) = self.gamepads.iter_mut().find(|gamepad| gamepad.id == id) else {
                error!("gamepad id ({id}) not found in connected controllers");
                return;
            };

            gamepad.axes[event.index() as usize] = event.value();
        });

        platform_event_iter!(event_delegate, WindowUnfocused, |_| {
            // set all buttons to not pressed

            for button_state in &mut input_state.keys.0 {
                button_state.set_pressed(false);
            }

            for button_state in &mut input_state.mouse.buttons.0 {
                button_state.set_pressed(false);
            }
        });

        self.write_binary_buffer();
    }

    fn write_binary_buffer(&mut self) {
        let mut writer = BinaryWriter::new(&mut self.buffer);

        // gamepads

        writer.write_u8(self.gamepads.len() as u8);

        for gamepad in &self.gamepads {
            writer.write_u8(gamepad.id);

            writer.write_u8(gamepad.buttons.len() as u8);
            writer.write_u8(gamepad.axes.len() as u8);

            let mut i = 1;
            for value in &gamepad.buttons {
                // the js side expects the button position to be provided
                // some gamepads report buttons in a non-standard order
                // so we'll probably want to provide a mapping of button positions to values
                // see https://github.com/vaguevoid/sdk/blob/main/src/platforms/gamepadDetect.ts#L274 as an example - the order that the buttons are specified is the order of the indices reported by the browser.
                writer.write_u8(i);
                i += 1;
                writer.write_f64((*value).into());
            }

            i = 0;
            for value in &gamepad.axes {
                let hand = match i {
                    0 => 1, // left
                    2 => 2, // right
                    _ => 0, // analogue triggers
                };
                writer.write_u8(hand);
                i += 1;
                writer.write_f64((*value).into());
            }

            let pressed_count = gamepad.buttons.iter().filter(|&&val| val > 0.0).count();

            let released_count = gamepad
                .buttons
                .iter()
                .zip(&gamepad.buttons_prev)
                .filter(|(curr, prev)| **curr == 0.0 && **prev > 0.0)
                .count();

            writer.write_u8(pressed_count as u8);
            writer.write_u8(released_count as u8);

            // write pressed button positions

            gamepad
                .buttons
                .iter()
                .enumerate()
                .filter(|(_, val)| **val > 0.0)
                .for_each(|(i, _)| writer.write_u8((i + 1) as u8));

            // write released button positions

            gamepad
                .buttons
                .iter()
                .zip(&gamepad.buttons_prev)
                .enumerate()
                .filter(|(_, (curr, prev))| **curr == 0.0 && **prev > 0.0)
                .for_each(|(i, _)| writer.write_u8((i + 1) as u8));
        }

        // rtc messages

        writer.write_u8(0);
    }

    pub fn binary_buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Flushes transient state and prepares `InputState` to receive inputs for a new frame.
    fn prepare_frame(&mut self, input_state: &mut InputState) {
        // shift button inputs for this frame, assuming the same state as last frame

        for button_state in &mut input_state.keys.0 {
            button_state.0 = (button_state.0 << 1) | (button_state.0 & 1);
        }

        for button_state in &mut input_state.mouse.buttons.0 {
            button_state.0 = (button_state.0 << 1) | (button_state.0 & 1);
        }

        // reset scroll delta

        input_state.mouse.scroll_delta = Default::default();

        // prepare gamepads

        for gamepad in &mut self.gamepads {
            gamepad.buttons_prev.copy_from_slice(&gamepad.buttons);
        }

        self.buffer.clear();
    }
}
