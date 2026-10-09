//! Подсветка экрана (`/sys/class/backlight`): проценты и ниты.
//!
//! Ниты ядро не сообщает: пересчёт идёт от максимальной яркости панели
//! (`[[output]] max_nits` в конфиге или паспорт устройства). Зависимость
//! яркости от кода у панелей близка к линейной (OLED с DC-диммингом, LCD
//! с ШИМ), поэтому ниты ≈ доля × максимум.

use crate::Sys;
use synshell_tr::t;

#[derive(Debug, Clone, PartialEq)]
pub struct Backlight {
    pub name: String,
    pub brightness: u32,
    pub max: u32,
    /// `raw`, `platform`, `firmware`.
    pub kind: String,
}

impl Backlight {
    pub fn percent(&self) -> f32 {
        if self.max == 0 {
            return 0.0;
        }
        self.brightness as f32 / self.max as f32 * 100.0
    }

    /// Яркость в нитах при максимуме панели `max_nits`.
    pub fn nits(&self, max_nits: f32) -> f32 {
        self.percent() / 100.0 * max_nits
    }

    /// Код для процента (не ниже 1: ноль у многих панелей — «выключить»).
    pub fn value_for_percent(&self, pct: f32) -> u32 {
        ((pct.clamp(0.0, 100.0) / 100.0 * self.max as f32).round() as u32).clamp(1, self.max.max(1))
    }
}

const BL: &str = "/sys/class/backlight";

/// Все подсветки; первой — предпочтительная (`firmware` > `platform` > `raw`,
/// как у systemd-backlight, но `raw` панели телефона — единственная).
pub fn list(sys: &Sys) -> Vec<Backlight> {
    let mut v: Vec<Backlight> = sys
        .list(BL)
        .into_iter()
        .filter_map(|name| {
            Some(Backlight {
                brightness: sys.read_num(format!("{BL}/{name}/brightness"))?,
                max: sys.read_num(format!("{BL}/{name}/max_brightness"))?,
                kind: sys.read(format!("{BL}/{name}/type")).unwrap_or_else(|| "raw".into()),
                name,
            })
        })
        .collect();
    let rank = |k: &str| match k {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    };
    v.sort_by_key(|b| rank(&b.kind));
    v
}

pub fn primary(sys: &Sys) -> Option<Backlight> {
    list(sys).into_iter().next()
}

/// Выставить яркость (проценты). Нужны права на запись (root на телефоне,
/// udev-правило или группа video на десктопе); без них — `brightnessctl`.
pub fn set_percent(sys: &Sys, pct: f32) -> std::io::Result<Backlight> {
    let mut b = primary(sys).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, t!("нет подсветки")))?;
    let v = b.value_for_percent(pct);
    let path = sys.path(format!("{BL}/{}/brightness", b.name));
    if let Err(e) = std::fs::write(&path, v.to_string()) {
        if crate::util::which("brightnessctl") {
            let arg = format!("{v}");
            crate::util::output("brightnessctl", &["-q", "-d", &b.name, "set", &arg]).ok_or(e)?;
        } else {
            return Err(e);
        }
    }
    b.brightness = v;
    Ok(b)
}

/// Изменить на `delta` процентов; результат — новая подсветка.
pub fn change(sys: &Sys, delta: f32) -> std::io::Result<Backlight> {
    let cur = primary(sys).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, t!("нет подсветки")))?;
    set_percent(sys, cur.percent() + delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn phone_panel() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("sys/class/backlight/panel0-backlight");
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("brightness"), "3000").unwrap();
        fs::write(b.join("max_brightness"), "4095").unwrap();
        fs::write(b.join("type"), "raw").unwrap();
        let sys = Sys::at(dir.path());
        let p = primary(&sys).unwrap();
        assert!((p.nits(1000.0) - 732.6).abs() < 0.5);
        let n = set_percent(&sys, 50.0).unwrap();
        assert_eq!(n.brightness, 2048);
        assert_eq!(fs::read_to_string(b.join("brightness")).unwrap(), "2048");
        assert_eq!(p.value_for_percent(0.0), 1);
    }
}
