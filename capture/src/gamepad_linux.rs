use evdev::{
    uinput::VirtualDevice, AbsInfo, AbsoluteAxisType, AttributeSet, EventType, InputEvent, Key,
    UinputAbsSetup,
};

use crate::KeyEvent;

pub struct GamepadEmulator {
    dev: VirtualDevice,
}

impl GamepadEmulator {
    pub fn new() -> Self {
        let mut keys = AttributeSet::<Key>::new();

        // Action pad
        keys.insert(Key::BTN_NORTH);
        keys.insert(Key::BTN_SOUTH);
        keys.insert(Key::BTN_EAST);
        keys.insert(Key::BTN_WEST);

        // D pad
        keys.insert(Key::BTN_DPAD_LEFT);
        keys.insert(Key::BTN_DPAD_RIGHT);
        keys.insert(Key::BTN_DPAD_DOWN);
        keys.insert(Key::BTN_DPAD_UP);

        // Menu pad
        keys.insert(Key::BTN_SELECT);
        keys.insert(Key::BTN_START);
        keys.insert(Key::BTN_MODE);

        // Triggers
        keys.insert(Key::BTN_TR);
        keys.insert(Key::BTN_TL);
        keys.insert(Key::BTN_TR2);
        keys.insert(Key::BTN_TL2);

        // Stick buttons
        keys.insert(Key::BTN_THUMBR);
        keys.insert(Key::BTN_THUMBL);

        let dev = evdev::uinput::VirtualDeviceBuilder::new()
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_X,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_Y,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_RX,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_RY,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_HAT1Y,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisType::ABS_HAT1X,
                AbsInfo::new(32768, 0, 65536, 0, 0, 1),
            ))
            .unwrap()
            .with_keys(&keys)
            .unwrap()
            .name("prospectivegopher")
            .build()
            .unwrap();

        Self { dev }
    }

    pub fn send_gamepad_event(&mut self, ev: KeyEvent) {
        match ev {
            KeyEvent::GamepadButton { button, state } => {
                if let gilrs::Button::Unknown = button {
                    return;
                }

                let but = match button {
                    gilrs::Button::North => Key::BTN_NORTH,
                    gilrs::Button::South => Key::BTN_SOUTH,
                    gilrs::Button::East => Key::BTN_EAST,
                    gilrs::Button::West => Key::BTN_WEST,

                    gilrs::Button::DPadLeft => Key::BTN_DPAD_LEFT,
                    gilrs::Button::DPadRight => Key::BTN_DPAD_RIGHT,
                    gilrs::Button::DPadDown => Key::BTN_DPAD_DOWN,
                    gilrs::Button::DPadUp => Key::BTN_DPAD_UP,

                    gilrs::Button::Select => Key::BTN_SELECT,
                    gilrs::Button::Start => Key::BTN_START,
                    gilrs::Button::Mode => Key::BTN_MODE,

                    gilrs::Button::RightTrigger => Key::BTN_TR,
                    gilrs::Button::LeftTrigger => Key::BTN_TL,
                    gilrs::Button::RightTrigger2 => Key::BTN_TR2,
                    gilrs::Button::LeftTrigger2 => Key::BTN_TL2,

                    gilrs::Button::RightThumb => Key::BTN_THUMBR,
                    gilrs::Button::LeftThumb => Key::BTN_THUMBL,

                    _ => panic!("unknown button: {:?}", button),
                };

                self.dev
                    .emit(&[InputEvent::new(EventType::KEY, but.code(), state as i32)])
                    .unwrap();
            }
            KeyEvent::GamepadAxis { axis, mut state } => {
                if let gilrs::Axis::Unknown = axis {
                    return;
                }

                let evdev_axis = match axis {
                    gilrs::Axis::LeftStickX => AbsoluteAxisType::ABS_X,
                    gilrs::Axis::LeftStickY => AbsoluteAxisType::ABS_Y,

                    gilrs::Axis::RightStickX => AbsoluteAxisType::ABS_RX,
                    gilrs::Axis::RightStickY => AbsoluteAxisType::ABS_RY,

                    gilrs::Axis::LeftZ => AbsoluteAxisType::ABS_HAT1Y,
                    gilrs::Axis::RightZ => AbsoluteAxisType::ABS_HAT1X,

                    _ => panic!("unknown axis: {:?}", axis),
                };

                if let gilrs::Axis::LeftStickY = axis {
                    state *= -1.;
                }
                if let gilrs::Axis::RightStickY = axis {
                    state *= -1.;
                }

                self.dev
                    .emit(&[InputEvent::new(
                        EventType::ABSOLUTE,
                        evdev_axis.0,
                        ((state * 32768.) + 32768.) as i32,
                    )])
                    .unwrap();
            }
            _ => {}
        }
    }
}
