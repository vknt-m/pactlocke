//! A gamepad as a second way to press the GBA's buttons: A and B, the
//! shoulders as L and R, Start, Back (or Select) as Select, and the D-pad or
//! the left stick for directions, with the right trigger held to
//! fast-forward. Any connected pad counts.

use gilrs::{Axis, Button, Gilrs};
use pactlocke_emu::Keys;

/// How far the stick must lean to count as a direction.
const STICK_DEADZONE: f32 = 0.5;

/// The gamepads, read once per frame.
pub struct Pads {
    gilrs: Option<Gilrs>,
}

impl Pads {
    /// Starts reading gamepads; on a system without support, there are none.
    pub fn new() -> Self {
        let gilrs = Gilrs::new()
            .map_err(|e| log::info!("No gamepad support here: {e}"))
            .ok();
        Self { gilrs }
    }

    /// The GBA buttons held on any gamepad now.
    pub fn held(&mut self) -> Keys {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return Keys::NONE;
        };
        // Events must be drained for gilrs to update its state.
        while gilrs.next_event().is_some() {}
        let mut held = Keys::NONE;
        for (_, pad) in gilrs.gamepads() {
            let map = [
                (Button::South, Keys::A),
                (Button::East, Keys::B),
                (Button::LeftTrigger, Keys::L),
                (Button::RightTrigger, Keys::R),
                (Button::Start, Keys::START),
                (Button::Select, Keys::SELECT),
                (Button::DPadUp, Keys::UP),
                (Button::DPadDown, Keys::DOWN),
                (Button::DPadLeft, Keys::LEFT),
                (Button::DPadRight, Keys::RIGHT),
            ];
            for (button, key) in map {
                if pad.is_pressed(button) {
                    held |= key;
                }
            }
            let (x, y) = (pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY));
            if x > STICK_DEADZONE {
                held |= Keys::RIGHT;
            } else if x < -STICK_DEADZONE {
                held |= Keys::LEFT;
            }
            if y > STICK_DEADZONE {
                held |= Keys::UP;
            } else if y < -STICK_DEADZONE {
                held |= Keys::DOWN;
            }
        }
        held
    }

    /// Whether any gamepad's right trigger is held: fast-forward. Call after
    /// [`Pads::held`], which reads the pads' state.
    pub fn fast_forward(&self) -> bool {
        self.gilrs
            .as_ref()
            .is_some_and(|g| g.gamepads().any(|(_, pad)| pad.is_pressed(Button::RightTrigger2)))
    }

    /// The first connected gamepad's name, if any.
    pub fn connected(&self) -> Option<String> {
        let gilrs = self.gilrs.as_ref()?;
        gilrs.gamepads().next().map(|(_, pad)| pad.name().to_owned())
    }
}
