//! Настройки устройств ввода libinput из `[input.mouse]` / `[input.touchpad]`.

use smithay::reexports::input::{AccelProfile as LiAccel, ClickMethod, Device, DeviceCapability, ScrollMethod};
use synshell_common::config::{AccelProfile, Input};

pub fn apply(device: &mut Device, input: &Input) {
    if !device.has_capability(DeviceCapability::Pointer) {
        return;
    }
    let is_touchpad = device.config_tap_finger_count() > 0;
    if is_touchpad {
        let t = &input.touchpad;
        let _ = device.config_tap_set_enabled(t.tap);
        let _ = device.config_tap_set_drag_enabled(t.tap_drag);
        let _ = device.config_dwt_set_enabled(t.disable_while_typing);
        let _ = device.config_scroll_set_natural_scroll_enabled(t.natural_scroll);
        let _ = device.config_left_handed_set(t.left_handed);
        let _ = device.config_accel_set_speed(t.accel_speed.clamp(-1.0, 1.0));
        let _ = device.config_accel_set_profile(profile(t.accel_profile));
        let _ = device.config_middle_emulation_set_enabled(t.middle_emulation);
        let scroll = match t.scroll_method.as_str() {
            "edge" => ScrollMethod::Edge,
            "none" => ScrollMethod::NoScroll,
            _ => ScrollMethod::TwoFinger,
        };
        let _ = device.config_scroll_set_method(scroll);
        let click = match t.click_method.as_str() {
            "button-areas" => ClickMethod::ButtonAreas,
            _ => ClickMethod::Clickfinger,
        };
        let _ = device.config_click_set_method(click);
    } else {
        let m = &input.mouse;
        let _ = device.config_scroll_set_natural_scroll_enabled(m.natural_scroll);
        let _ = device.config_left_handed_set(m.left_handed);
        let _ = device.config_accel_set_speed(m.accel_speed.clamp(-1.0, 1.0));
        let _ = device.config_accel_set_profile(profile(m.accel_profile));
        let _ = device.config_middle_emulation_set_enabled(m.middle_emulation);
    }
}

fn profile(p: AccelProfile) -> LiAccel {
    match p {
        AccelProfile::Flat => LiAccel::Flat,
        AccelProfile::Adaptive => LiAccel::Adaptive,
    }
}
