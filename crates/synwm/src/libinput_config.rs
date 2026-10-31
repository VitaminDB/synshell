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

/// Рабочий диапазон давления пера из свойства udev устройства `SYNSHELL_PEN_PRESSURE_RANGE=мин:макс`
/// (доли 0..1 аппаратного диапазона; правило — в оверлее устройства). Перо, которое шлёт давление
/// уже в воздухе (ONYX Note Air 4C: 950–1270 из 4095 в ~5 мм от экрана, вместе с BTN_TOUCH), libinput
/// иначе считает прижатым — штрих начинается до касания, а нажим почти не меняет толщину. Давление
/// `мин` становится нулём (касание — чуть выше), `макс` — полным нажимом. libinput применяет новый
/// диапазон со следующего поднесения пера.
pub fn pen_pressure_range(event: &mut smithay::reexports::input::event::tablet_tool::TabletToolProximityEvent, pen: &synshell_common::config::Pen) {
    use smithay::reexports::input::event::tablet_tool::TabletToolEventTrait;
    use smithay::reexports::input::{event::EventTrait, AsRaw};
    extern "C" {
        fn libinput_tablet_tool_config_pressure_range_is_available(tool: *mut std::ffi::c_void) -> i32;
        fn libinput_tablet_tool_config_pressure_range_set(tool: *mut std::ffi::c_void, min: f64, max: f64) -> i32;
        fn libinput_tablet_tool_config_pressure_range_get_minimum(tool: *mut std::ffi::c_void) -> f64;
        fn libinput_tablet_tool_config_pressure_range_get_maximum(tool: *mut std::ffi::c_void) -> f64;
    }
    // Умолчание устройства — из udev, настройки `[input.pen]` (не 0) — поверх.
    let dev = (unsafe { event.device().udev_device() })
        .and_then(|d| d.property_value("SYNSHELL_PEN_PRESSURE_RANGE").map(|v| v.to_string_lossy().into_owned()))
        .and_then(|r| r.split_once(':').and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))));
    let (dmin, dmax) = dev.unwrap_or((0.0, 1.0));
    let min = if pen.touch_threshold > 0.0 { pen.touch_threshold } else { dmin }.clamp(0.0, 0.9);
    let max = if pen.full_pressure > 0.0 { pen.full_pressure } else { dmax }.clamp(min + 0.05, 1.0);
    if dev.is_none() && pen.touch_threshold <= 0.0 && pen.full_pressure <= 0.0 {
        return;
    }
    let tool = event.tool();
    let raw = tool.as_raw_mut() as *mut std::ffi::c_void;
    unsafe {
        if libinput_tablet_tool_config_pressure_range_is_available(raw) == 0 {
            return;
        }
        let (cur_min, cur_max) = (libinput_tablet_tool_config_pressure_range_get_minimum(raw), libinput_tablet_tool_config_pressure_range_get_maximum(raw));
        if (cur_min - min).abs() < 1e-6 && (cur_max - max).abs() < 1e-6 {
            return;
        }
        let status = libinput_tablet_tool_config_pressure_range_set(raw, min, max);
        tracing::info!(min, max, status, "перо: рабочий диапазон давления");
    }
}
