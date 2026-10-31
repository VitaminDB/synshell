//! Подсветка экрана (`/sys/class/backlight`): проценты и ниты.
//!
//! Ниты ядро не сообщает: пересчёт идёт от максимальной яркости панели
//! (`[[output]] max_nits` в конфиге или паспорт устройства). Зависимость
//! яркости от кода у панелей близка к линейной (OLED с DC-диммингом, LCD
//! с ШИМ), поэтому ниты ≈ доля × максимум.
//!
//! Передняя подсветка из двух каналов (E Ink: холодный белый и тёплый
//! жёлтый, `frontlight-white` + `frontlight-warm`) — [`FrontLight`]: общая
//! яркость и тепло смешиванием каналов. При такой паре [`primary`] и
//! [`set_percent`] работают с общей яркостью, тепло сохраняется.

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

/// Экспоненциальная шкала кода фронтлайта: свет ∝ e^(K·код). TI LM3630A (ONYX BOOX и др.):
/// I = I_full · 0.85^(44 − (код + 1)/5.8181) → K = −ln 0.85 / 5.8181 ≈ 0.02793 (×1,028 на код).
const EXP_PER_CODE: f32 = 0.027_93;

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

/// Основная подсветка. У пары «белая + тёплая» — общая яркость обоих
/// каналов (имя `белая+тёплая`, тип `front`, код — сумма долей × максимум).
pub fn primary(sys: &Sys) -> Option<Backlight> {
    let all = list(sys);
    if let Some(fl) = pair(&all) {
        return Some(fl.combined());
    }
    all.into_iter().next()
}

fn write_code(sys: &Sys, name: &str, v: u32) -> std::io::Result<()> {
    let path = sys.path(format!("{BL}/{name}/brightness"));
    if let Err(e) = std::fs::write(&path, v.to_string()) {
        if crate::util::which("brightnessctl") {
            let arg = format!("{v}");
            crate::util::output("brightnessctl", &["-q", "-d", name, "set", &arg]).ok_or(e)?;
        } else {
            return Err(e);
        }
    }
    Ok(())
}

fn not_found() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, t!("нет подсветки"))
}

/// Выставить яркость (проценты). Нужны права на запись (root на телефоне,
/// udev-правило или группа video на десктопе); без них — `brightnessctl`.
/// У пары «белая + тёплая» — общая яркость с прежним теплом.
pub fn set_percent(sys: &Sys, pct: f32) -> std::io::Result<Backlight> {
    let all = list(sys);
    if let Some(fl) = pair(&all) {
        let (_, warmth) = fl.state(sys);
        // не ниже одного кода в сумме: ноль — «выключить», как и у одиночной подсветки
        let min = 100.0 / fl.white.max.max(fl.warm.max).max(1) as f32;
        let fl = set_pair(sys, fl, pct.clamp(min, 100.0), warmth)?;
        return Ok(fl.combined());
    }
    let mut b = all.into_iter().next().ok_or_else(not_found)?;
    let v = b.value_for_percent(pct);
    write_code(sys, &b.name, v)?;
    b.brightness = v;
    Ok(b)
}

// ─── Пара «белая + тёплая» ───────────────────────────────────────────────────

/// Передняя подсветка из двух каналов: холодный белый и тёплый.
/// Смешивание в кодах: белый = B·(1−w), тёплый = B·w (доли от максимума
/// каждого канала); обратно — B = белый + тёплый, w = тёплый / (белый + тёплый).
#[derive(Debug, Clone, PartialEq)]
pub struct FrontLight {
    pub white: Backlight,
    pub warm: Backlight,
}

fn is_white(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["white", "cold", "cool"].iter().any(|k| n.contains(k))
}

fn is_warm(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["warm", "yellow"].iter().any(|k| n.contains(k))
}

fn pair(all: &[Backlight]) -> Option<FrontLight> {
    let white = all.iter().find(|b| is_white(&b.name) && !is_warm(&b.name))?;
    let warm = all.iter().find(|b| is_warm(&b.name) && !is_white(&b.name))?;
    Some(FrontLight { white: white.clone(), warm: warm.clone() })
}

/// Пара «белая + тёплая» среди подсветок (по именам `*white*`/`*cold*`/`*cool*`
/// и `*warm*`/`*yellow*`), если есть.
pub fn front_light(sys: &Sys) -> Option<FrontLight> {
    pair(&list(sys))
}

/// Последнее, что записали сами, по пути белого канала: (коды белый/тёплый, тепло %).
/// Пока в sysfs те же коды — тепло берётся отсюда: обратный расчёт по
/// округлённым кодам на малой яркости неточен, и шаги яркости сбивали бы тепло.
static LAST: std::sync::Mutex<Vec<(std::path::PathBuf, u32, u32, f32)>> = std::sync::Mutex::new(Vec::new());

impl FrontLight {
    /// Коды каналов для яркости и тепла (проценты).
    pub fn codes(&self, brightness_pct: f32, warmth_pct: f32) -> (u32, u32) {
        let b = brightness_pct.clamp(0.0, 100.0) / 100.0;
        let w = warmth_pct.clamp(0.0, 100.0) / 100.0;
        // Свет делится между каналами в линейных единицах: код у драйверов фронтлайта экспоненциальный
        // (свет ∝ e^(K·код)), поэтому доля света share — это сдвиг кода на ln(share)/K, а не умножение кода.
        let code = |share: f32, max: u32| -> u32 {
            if b <= 0.0 || share <= 0.0 {
                return 0;
            }
            let v = (b * max as f32 + share.ln() / EXP_PER_CODE).round();
            // при B > 0 канал с ненулевой долей не гаснет совсем
            (v.max(1.0) as u32).min(max)
        };
        (code(1.0 - w, self.white.max), code(w, self.warm.max))
    }

    /// Яркость и тепло (проценты) по кодам каналов; оба погашены — тепло `None`.
    pub fn from_codes(&self) -> (f32, Option<f32>) {
        let light = |b: &Backlight| if b.brightness == 0 { 0.0 } else { (EXP_PER_CODE * b.brightness as f32).exp() };
        let (lw, lk) = (light(&self.white), light(&self.warm));
        let sum = lw + lk;
        if sum <= 0.0 {
            return (0.0, None);
        }
        let max = self.white.max.max(self.warm.max).max(1) as f32;
        let b = (sum.ln() / EXP_PER_CODE / max * 100.0).clamp(0.0, 100.0);
        (b, Some(lk / sum * 100.0))
    }

    /// Яркость и тепло (проценты). Тепло — записанное самими, если коды
    /// не менялись; при погашенных каналах — последнее записанное или 0.
    pub fn state(&self, sys: &Sys) -> (f32, f32) {
        let (b, w) = self.from_codes();
        let key = sys.path(format!("{BL}/{}", self.white.name));
        let last = LAST.lock().unwrap().iter().find(|l| l.0 == key).cloned();
        match (last, w) {
            (Some((_, cw, ck, lw)), _) if cw == self.white.brightness && ck == self.warm.brightness => (b, lw),
            (_, Some(w)) => (b, w),
            (Some((.., lw)), None) => (b, lw),
            (None, None) => (b, 0.0),
        }
    }

    /// Пара как одна подсветка: код — общая яркость в шкале большего максимума.
    fn combined(&self) -> Backlight {
        let max = self.white.max.max(self.warm.max);
        let (b, _) = self.from_codes();
        Backlight {
            name: format!("{}+{}", self.white.name, self.warm.name),
            brightness: (b / 100.0 * max as f32).round() as u32,
            max,
            kind: "front".into(),
        }
    }
}

fn set_pair(sys: &Sys, mut fl: FrontLight, brightness_pct: f32, warmth_pct: f32) -> std::io::Result<FrontLight> {
    let warmth_pct = warmth_pct.clamp(0.0, 100.0);
    let (cw, ck) = fl.codes(brightness_pct, warmth_pct);
    write_code(sys, &fl.white.name, cw)?;
    write_code(sys, &fl.warm.name, ck)?;
    fl.white.brightness = cw;
    fl.warm.brightness = ck;
    let key = sys.path(format!("{BL}/{}", fl.white.name));
    let mut last = LAST.lock().unwrap();
    last.retain(|l| l.0 != key);
    last.push((key, cw, ck, warmth_pct));
    Ok(fl)
}

/// Выставить общую яркость и тепло (проценты: 0 — холодный белый, 100 — тёплый).
pub fn set_front_light(sys: &Sys, brightness_pct: f32, warmth_pct: f32) -> std::io::Result<FrontLight> {
    let fl = front_light(sys).ok_or_else(not_found)?;
    set_pair(sys, fl, brightness_pct, warmth_pct)
}

/// Только тепло при прежней яркости.
pub fn set_warmth(sys: &Sys, warmth_pct: f32) -> std::io::Result<FrontLight> {
    let fl = front_light(sys).ok_or_else(not_found)?;
    let (b, _) = fl.state(sys);
    set_pair(sys, fl, b, warmth_pct)
}

/// Яркость и тепло пары (проценты), если она есть.
pub fn front_light_state(sys: &Sys) -> Option<(f32, f32)> {
    front_light(sys).map(|fl| fl.state(sys))
}

/// Изменить на `delta` процентов; результат — новая подсветка.
pub fn change(sys: &Sys, delta: f32) -> std::io::Result<Backlight> {
    let cur = primary(sys).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, t!("нет подсветки")))?;
    set_percent(sys, cur.percent() + delta)
}

/// Вернуть подсветке запомненное оболочкой (`[brightness] level` и `warmth`, проценты).
///
/// `level` — общая яркость (у пары «белая + тёплая» — с теплом `warmth`); не задан —
/// яркость остаётся как есть (восстановленная systemd-backlight), у пары
/// пересмешивается только тепло. `only_if_dark` — трогать лишь погашенную
/// подсветку (включение экрана, экран входа): уровень, выставленный кем-то
/// после загрузки, не перебивается. Погашенная подсветка без `level`
/// остаётся погашенной. Возвращает, было ли что записано.
pub fn restore(sys: &Sys, level: Option<f32>, warmth: Option<f32>, only_if_dark: bool) -> std::io::Result<bool> {
    let level = level.filter(|l| *l > 0.0).map(|l| l.min(100.0));
    let all = list(sys);
    if let Some(fl) = pair(&all) {
        let (b, w) = fl.state(sys);
        if only_if_dark && b > 0.0 {
            return Ok(false);
        }
        let tb = level.unwrap_or(b);
        let tw = warmth.map(|w| w.clamp(0.0, 100.0)).unwrap_or(w);
        if tb <= 0.0 {
            return Ok(false);
        }
        // те же коды — не писать (обратный расчёт по округлённым кодам неточен)
        if fl.codes(tb, tw) == (fl.white.brightness, fl.warm.brightness) {
            return Ok(false);
        }
        set_pair(sys, fl, tb, tw)?;
        return Ok(true);
    }
    let Some(b) = all.into_iter().next() else { return Ok(false) };
    let Some(l) = level else { return Ok(false) };
    if (only_if_dark && b.brightness > 0) || b.value_for_percent(l) == b.brightness {
        return Ok(false);
    }
    set_percent(sys, l)?;
    Ok(true)
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
        assert!(front_light(&sys).is_none());
    }

    fn eink(white: u32, warm: u32) -> (tempfile::TempDir, Sys) {
        let dir = tempfile::tempdir().unwrap();
        for (n, v) in [("frontlight-white", white), ("frontlight-warm", warm)] {
            let b = dir.path().join("sys/class/backlight").join(n);
            fs::create_dir_all(&b).unwrap();
            fs::write(b.join("brightness"), v.to_string()).unwrap();
            fs::write(b.join("max_brightness"), "217").unwrap();
            fs::write(b.join("type"), "raw").unwrap();
        }
        let sys = Sys::at(dir.path());
        (dir, sys)
    }

    fn code(dir: &tempfile::TempDir, n: &str) -> u32 {
        fs::read_to_string(dir.path().join("sys/class/backlight").join(n).join("brightness")).unwrap().trim().parse().unwrap()
    }

    #[test]
    fn names() {
        assert!(is_white("frontlight-white") && is_white("cool-led") && is_white("bl_cold"));
        assert!(is_warm("frontlight-warm") && is_warm("YELLOW"));
        assert!(!is_white("panel0-backlight") && !is_warm("panel0-backlight"));
    }

    #[test]
    fn mixing() {
        let (dir, sys) = eink(0, 0);
        let fl = front_light(&sys).unwrap();
        assert_eq!(fl.white.name, "frontlight-white");
        assert_eq!(fl.codes(100.0, 0.0), (217, 0));
        assert_eq!(fl.codes(100.0, 100.0), (0, 217));
        // поровну света: каждый канал на ln(1/2)/K ≈ 25 кодов ниже общего
        assert_eq!(fl.codes(50.0, 50.0), (84, 84));
        assert_eq!(fl.codes(0.0, 30.0), (0, 0));
        // малая яркость: канал с ненулевой долей не гаснет
        assert_eq!(fl.codes(1.0, 10.0), (1, 1));
        set_front_light(&sys, 80.0, 25.0).unwrap();
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), (163, 124));
        let (b, w) = front_light_state(&sys).unwrap();
        assert!((b - 80.0).abs() < 0.5 && (w - 25.0).abs() < 0.01, "{b} {w}");
    }

    #[test]
    fn reverse() {
        let (_dir, sys) = eink(150, 50);
        let fl = front_light(&sys).unwrap();
        let (b, w) = fl.from_codes();
        assert!((b - 70.105).abs() < 0.01, "{b}");
        assert!((w.unwrap() - 5.770).abs() < 0.01);
        let (_d, s0) = eink(0, 0);
        assert_eq!(front_light(&s0).unwrap().from_codes(), (0.0, None));
    }

    #[test]
    fn primary_keeps_warmth() {
        let (dir, sys) = eink(100, 100);
        let p = primary(&sys).unwrap();
        assert_eq!(p.kind, "front");
        assert_eq!((p.brightness, p.max), (125, 217));
        set_percent(&sys, 40.0).unwrap();
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), (62, 62));
        // много мелких шагов вниз и обратно: тепло не уплывает
        for _ in 0..30 {
            change(&sys, -1.0).unwrap();
        }
        for _ in 0..30 {
            change(&sys, 1.0).unwrap();
        }
        let (_, w) = front_light_state(&sys).unwrap();
        assert!((w - 50.0).abs() < 0.01, "{w}");
        // общая яркость в процентах не опускается до нуля
        let b = set_percent(&sys, 0.0).unwrap();
        assert!(b.brightness >= 1);
        set_warmth(&sys, 100.0).unwrap();
        assert_eq!(code(&dir, "frontlight-white"), 0);
        assert!(code(&dir, "frontlight-warm") >= 1);
    }
    #[test]
    fn restore_pair() {
        // после загрузки оба канала погашены (systemd не восстановил)
        let (dir, sys) = eink(0, 0);
        assert!(!restore(&sys, None, Some(30.0), false).unwrap(), "без уровня погашенное не включается");
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), (0, 0));
        assert!(restore(&sys, Some(60.0), Some(30.0), false).unwrap());
        let fl = front_light(&sys).unwrap();
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), fl.codes(60.0, 30.0));
        let (b, w) = front_light_state(&sys).unwrap();
        assert!((b - 60.0).abs() < 0.5 && (w - 30.0).abs() < 0.01, "{b} {w}");
        // повтор — ничего не пишет
        assert!(!restore(&sys, Some(60.0), Some(30.0), false).unwrap());
        // включение экрана: горит — не трогать, погашено — вернуть
        assert!(!restore(&sys, Some(90.0), Some(30.0), true).unwrap());
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), fl.codes(60.0, 30.0));
        set_code(&dir, "frontlight-white", 0);
        set_code(&dir, "frontlight-warm", 0);
        assert!(restore(&sys, Some(90.0), Some(30.0), true).unwrap());
        assert_eq!((code(&dir, "frontlight-white"), code(&dir, "frontlight-warm")), fl.codes(90.0, 30.0));
    }

    #[test]
    fn restore_pair_warmth_only() {
        // systemd восстановил каналы, уровня в конфиге нет — только тепло, яркость прежняя
        let (dir, sys) = eink(150, 50);
        let (b0, _) = front_light_state(&sys).unwrap();
        assert!(restore(&sys, None, Some(50.0), false).unwrap());
        let (b, w) = front_light_state(&sys).unwrap();
        assert!((b - b0).abs() < 0.5 && (w - 50.0).abs() < 0.01, "{b0} {b} {w}");
        let (cw, ck) = (code(&dir, "frontlight-white"), code(&dir, "frontlight-warm"));
        assert_eq!(cw, ck);
    }

    fn set_code(dir: &tempfile::TempDir, n: &str, v: u32) {
        fs::write(dir.path().join("sys/class/backlight").join(n).join("brightness"), v.to_string()).unwrap();
    }

    #[test]
    fn restore_single() {
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("sys/class/backlight/panel0-backlight");
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("brightness"), "0").unwrap();
        fs::write(b.join("max_brightness"), "4095").unwrap();
        let sys = Sys::at(dir.path());
        let read = || fs::read_to_string(b.join("brightness")).unwrap();
        assert!(!restore(&sys, None, None, false).unwrap());
        assert!(!restore(&sys, Some(0.0), None, false).unwrap(), "ноль не уровень");
        assert_eq!(read(), "0");
        assert!(restore(&sys, Some(50.0), None, true).unwrap());
        assert_eq!(read(), "2048");
        assert!(!restore(&sys, Some(50.0), None, false).unwrap());
        assert!(!restore(&sys, Some(80.0), None, true).unwrap(), "горит — не трогать");
        assert!(restore(&sys, Some(80.0), None, false).unwrap(), "старт сеанса — уровень из конфига");
        assert_eq!(read(), "3276");
    }
}
