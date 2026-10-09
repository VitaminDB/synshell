//! Датчики компаса: источник платформы `/usr/lib/syndroid/sensors-source accel mag`
//! (Qualcomm SLPI через libssc, synmobile docs/15) — строки `A x y z` (м/с²), `M x y z`
//! (мкТл, без поправки SLPI `mag_cal`), `U x y z` (сырое поле), `B x y z` (поправка SLPI).
//! Оси — как у Android: X вправо, Y к верхнему краю, Z из экрана.
//!
//! Курс — как `SensorManager.getRotationMatrix` Android: восток = поле × гравитация,
//! север = гравитация × восток; курс верхнего края — `atan2(Ey, Ny)`. Телефон стоит
//! вертикально (наклон больше 60°) — курс задней камеры (`−Z`).
//!
//! Калибровка: если SLPI прислал поправку (`B`), поле уже исправлено; иначе — своя
//! «восьмёркой» ([`Calibration`]): сфера по точкам методом наименьших квадратов, центр —
//! смещение от магнитов корпуса (hard iron), хранится в `~/.config/synshell/compass.toml`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use syngui::prelude::*;

pub const SOURCE: &str = "/usr/lib/syndroid/sensors-source";

/// Откуда поправка поля.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cal {
    /// Поправка SLPI (`mag_cal`).
    Slpi,
    /// Своя калибровка «восьмёркой».
    Own,
    /// Без калибровки — курс может врать на десятки градусов.
    None,
}

/// Показания для экрана.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    /// Курс, градусы от магнитного севера по часовой стрелке (0…360).
    pub heading: f32,
    /// Наклон вперёд-назад и вбок, градусы.
    pub pitch: f32,
    pub roll: f32,
    /// Модуль поля, мкТл (Земля — 25…65).
    pub field: f32,
    pub cal: Cal,
    /// Курс — по задней камере (телефон вертикально).
    pub upright: bool,
}

/// Состояние датчиков.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    Ok(Reading),
    /// Датчиков нет (компьютер) или источник не запустился.
    Missing(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// Смещение поля (hard iron), мкТл.
    pub bias: [f32; 3],
}

fn calib_path() -> std::path::PathBuf {
    synshell_common::paths::config_dir().join("compass.toml")
}

impl Calibration {
    pub fn load() -> Option<Calibration> {
        std::fs::read_to_string(calib_path()).ok().and_then(|s| toml::from_str(&s).ok())
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = calib_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, toml::to_string(self).unwrap_or_default())
    }
}

type V3 = [f32; 3];

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Курс, наклоны по гравитации `g` и полю `m` (оси Android). `None` — поле почти вдоль
/// гравитации или телефон в свободном падении.
pub fn orientation(g: V3, m: V3) -> Option<(f32, f32, f32, bool)> {
    let gn = norm(g);
    if gn < 1.0 {
        return None;
    }
    let h = cross(m, g);
    let hn = norm(h);
    if hn < 0.1 {
        return None;
    }
    let e = scale(h, 1.0 / hn);
    let a = scale(g, 1.0 / gn);
    let n = cross(a, e);
    let pitch = (-a[1]).clamp(-1.0, 1.0).asin().to_degrees();
    let roll = (-a[0]).atan2(a[2]).to_degrees();
    // Вертикально — по задней камере (−Z), иначе — по верхнему краю (Y).
    let upright = pitch.abs() > 60.0;
    let az = if upright { (-e[2]).atan2(-n[2]) } else { e[1].atan2(n[1]) };
    let heading = (az.to_degrees() + 360.0) % 360.0;
    Some((heading, pitch, roll, upright))
}

/// Сфера по точкам методом наименьших квадратов: x²+y²+z² = 2ax+2by+2cz+d → центр (a, b, c)
/// и радиус. `None` — точек мало или они вырождены (лежат в плоскости).
pub fn fit_sphere(points: &[V3]) -> Option<(V3, f32)> {
    if points.len() < 12 {
        return None;
    }
    // Нормальные уравнения 4×4 для [2x 2y 2z 1]·[a b c d]ᵀ = x²+y²+z²
    let mut m = [[0f64; 5]; 4];
    for p in points {
        let row = [2.0 * p[0] as f64, 2.0 * p[1] as f64, 2.0 * p[2] as f64, 1.0];
        let rhs = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]) as f64;
        for i in 0..4 {
            for j in 0..4 {
                m[i][j] += row[i] * row[j];
            }
            m[i][4] += row[i] * rhs;
        }
    }
    // Гаусс с выбором главного элемента
    for c in 0..4 {
        let piv = (c..4).max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs()))?;
        if m[piv][c].abs() < 1e-9 {
            return None;
        }
        m.swap(c, piv);
        for r in 0..4 {
            if r != c {
                let k = m[r][c] / m[c][c];
                for j in c..5 {
                    m[r][j] -= k * m[c][j];
                }
            }
        }
    }
    let s: Vec<f64> = (0..4).map(|i| m[i][4] / m[i][i]).collect();
    let r2 = s[3] + s[0] * s[0] + s[1] * s[1] + s[2] * s[2];
    if r2 <= 0.0 {
        return None;
    }
    Some(([s[0] as f32, s[1] as f32, s[2] as f32], r2.sqrt() as f32))
}

/// Покрытие направлений при калибровке: 26 секторов (знаки −/0/+ по осям, кроме центра).
pub fn sector(d: V3) -> Option<usize> {
    let n = norm(d);
    if n < 1e-3 {
        return None;
    }
    let q = |v: f32| -> usize {
        let v = v / n;
        if v < -0.45 {
            0
        } else if v > 0.45 {
            2
        } else {
            1
        }
    };
    let i = q(d[0]) * 9 + q(d[1]) * 3 + q(d[2]);
    (i != 13).then_some(if i > 13 { i - 1 } else { i })
}

pub const SECTORS: usize = 26;

/// Сбор точек калибровки (сырое поле `U`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collect {
    pub points: Vec<V3>,
    pub seen: [bool; SECTORS],
}

impl Collect {
    /// Доля покрытых направлений (0…1) — относительно текущей оценки центра.
    pub fn coverage(&self) -> f32 {
        self.seen.iter().filter(|s| **s).count() as f32 / SECTORS as f32
    }

    fn add(&mut self, p: V3) {
        // реже 1 точки на 0,5 мкТл смещения — не копить одинаковые
        if self.points.last().is_some_and(|l| norm([l[0] - p[0], l[1] - p[1], l[2] - p[2]]) < 0.5) {
            return;
        }
        self.points.push(p);
        if self.points.len() > 2000 {
            self.points.remove(0);
        }
        let center = fit_sphere(&self.points).map(|c| c.0).unwrap_or_else(|| mean(&self.points));
        if let Some(s) = sector([p[0] - center[0], p[1] - center[1], p[2] - center[2]]) {
            self.seen[s] = true;
        }
    }
}

fn mean(ps: &[V3]) -> V3 {
    let n = ps.len().max(1) as f32;
    let s = ps.iter().fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
    [s[0] / n, s[1] / n, s[2] / n]
}

/// Сигналы: состояние датчиков и сбор калибровки (`Some` — идёт).
#[derive(Clone, Copy)]
pub struct Sensors {
    pub state: RwSignal<State>,
    pub collect: RwSignal<Option<Collect>>,
}

impl Sensors {
    pub fn new() -> Self {
        Sensors { state: use_signal(State::Starting), collect: use_signal(None) }
    }

    /// Начать калибровку «восьмёркой».
    pub fn start_calibration(&self) {
        self.collect.set(Some(Collect::default()));
    }

    /// Закончить: сохранить центр сферы как поправку. `false` — точек мало.
    pub fn finish_calibration(&self) -> bool {
        let Some(c) = self.collect.get_untracked() else { return false };
        self.collect.set(None);
        match fit_sphere(&c.points) {
            Some((center, _)) => {
                let cal = Calibration { bias: center };
                if let Err(e) = cal.save() {
                    tracing::warn!("калибровка компаса не сохранена: {e}");
                }
                OWN_CAL.with(|o| o.set(Some(cal)));
                true
            }
            None => false,
        }
    }
}

thread_local! {
    static OWN_CAL: std::cell::Cell<Option<Calibration>> = const { std::cell::Cell::new(None) };
}

/// Сглаживание векторов (низкие частоты) и курса.
const ALPHA: f32 = 0.2;

/// Запустить источник в фоне. `SYN_COMPASS_DEMO=градусы` — без датчиков: курс плавно
/// ходит вокруг заданного (для проверки на компьютере и снимков).
pub fn start(s: Sensors) {
    OWN_CAL.with(|o| o.set(Calibration::load()));
    if let Ok(d) = std::env::var("SYN_COMPASS_DEMO") {
        let base: f32 = d.trim().parse().unwrap_or(0.0);
        let t0 = Instant::now();
        let state = s.state;
        std::thread::spawn(move || loop {
            let t = t0.elapsed().as_secs_f32();
            let r = Reading { heading: (base + 8.0 * (t * 0.7).sin() + 360.0) % 360.0, pitch: 4.0 * (t * 0.5).sin(), roll: 3.0 * (t * 0.4).cos(), field: 48.5, cal: Cal::Slpi, upright: false };
            run_on_main_thread(move || state.set(State::Ok(r)));
            std::thread::sleep(Duration::from_millis(100));
        });
        return;
    }
    let src = std::env::var("SYN_COMPASS_SOURCE").unwrap_or_else(|_| SOURCE.to_string());
    let child = Command::new(&src).args(["accel", "mag"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            tracing::info!("датчиков компаса нет ({src}: {e})");
            s.state.set(State::Missing(t!("Нет магнитометра: источник датчиков платформы не найден ({src})", src = src)));
            return;
        }
    };
    let out = child.stdout.take();
    let state = s.state;
    let collect = s.collect;
    std::thread::spawn(move || {
        // stdin держим открытым: закрылся — источник выключит датчики и выйдет
        let _stdin = child.stdin.take();
        let Some(out) = out else { return };
        let mut g: Option<V3> = None;
        let mut m: Option<V3> = None;
        let mut slpi = false;
        let mut last = Instant::now() - Duration::from_secs(1);
        for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
            let mut it = line.split_whitespace();
            let tag = it.next().unwrap_or("");
            let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
            let [x, y, z] = v[..] else { continue };
            match tag {
                "A" => g = Some(lowpass(g, [x, y, z])),
                "B" => slpi = norm([x, y, z]) > 0.0,
                "U" => {
                    let p = [x, y, z];
                    run_on_main_thread(move || {
                        if let Some(mut c) = collect.get_untracked() {
                            c.add(p);
                            collect.set(Some(c));
                        }
                    });
                }
                "M" => {
                    // без поправки SLPI — своя калибровка
                    let own = (!slpi).then(|| OWN_CAL.with(|o| o.get())).flatten();
                    let raw = [x, y, z];
                    let fixed = match own {
                        Some(c) => [raw[0] - c.bias[0], raw[1] - c.bias[1], raw[2] - c.bias[2]],
                        None => raw,
                    };
                    m = Some(lowpass(m, fixed));
                    // не чаще 20 раз в секунду
                    if last.elapsed() < Duration::from_millis(50) {
                        continue;
                    }
                    last = Instant::now();
                    let (Some(gv), Some(mv)) = (g, m) else { continue };
                    let Some((heading, pitch, roll, upright)) = orientation(gv, mv) else { continue };
                    let cal = if slpi {
                        Cal::Slpi
                    } else if own.is_some() {
                        Cal::Own
                    } else {
                        Cal::None
                    };
                    let r = Reading { heading, pitch, roll, field: norm(mv), cal, upright };
                    run_on_main_thread(move || state.set(State::Ok(r)));
                }
                _ => {}
            }
        }
        let err = child.stderr.take().map(|e| BufReader::new(e).lines().map_while(|l| l.ok()).collect::<Vec<_>>().join("; ")).unwrap_or_default();
        let _ = child.wait();
        tracing::warn!("источник датчиков компаса завершился: {err}");
        let msg = if err.is_empty() { t!("Источник датчиков завершился") } else { t!("Источник датчиков завершился: {err}", err = err) };
        run_on_main_thread(move || state.set(State::Missing(msg)));
    });
}

fn lowpass(prev: Option<V3>, v: V3) -> V3 {
    match prev {
        Some(p) => [p[0] + ALPHA * (v[0] - p[0]), p[1] + ALPHA * (v[1] - p[1]), p[2] + ALPHA * (v[2] - p[2])],
        None => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Земное поле на широте ~55°: север 17 мкТл, вниз 48 мкТл (Z мира вверх).
    fn world_field(heading_deg: f32) -> (V3, V3) {
        // Телефон лежит экраном вверх, верхний край смотрит на heading: оси телефона в мире
        let h = heading_deg.to_radians();
        // мир: x — восток, y — север, z — вверх; поле (0, 17, −48), гравитация-реакция (0, 0, 9.8)
        let (n, d) = (17.0, -48.0);
        // перевод мира в оси телефона: поворот вокруг Z на −heading
        let to_dev = |w: V3| -> V3 { [w[0] * h.cos() - w[1] * h.sin(), w[0] * h.sin() + w[1] * h.cos(), w[2]] };
        (to_dev([0.0, 0.0, 9.8]), to_dev([0.0, n, d]))
    }

    #[test]
    fn heading_flat() {
        for want in [0.0f32, 45.0, 90.0, 180.0, 270.0, 333.0] {
            let (g, m) = world_field(want);
            let (h, pitch, roll, upright) = orientation(g, m).unwrap();
            let d = ((h - want + 540.0) % 360.0) - 180.0;
            assert!(d.abs() < 0.5, "курс {h} вместо {want}");
            assert!(pitch.abs() < 0.5 && roll.abs() < 0.5 && !upright);
        }
    }

    #[test]
    fn sphere_fit_finds_bias() {
        let c = [12.0f32, -40.0, 75.0];
        let mut pts = vec![];
        for i in 0..20 {
            for j in 0..10 {
                let (a, b) = (i as f32 * 0.31, j as f32 * 0.29 - 1.4);
                pts.push([c[0] + 50.0 * b.cos() * a.cos(), c[1] + 50.0 * b.cos() * a.sin(), c[2] + 50.0 * b.sin()]);
            }
        }
        let (center, r) = fit_sphere(&pts).unwrap();
        assert!((center[0] - c[0]).abs() < 0.1 && (center[1] - c[1]).abs() < 0.1 && (center[2] - c[2]).abs() < 0.1);
        assert!((r - 50.0).abs() < 0.1);
        // в плоскости — вырождено
        let flat: Vec<V3> = (0..30).map(|i| [(i as f32).cos() * 50.0, (i as f32).sin() * 50.0, 0.0]).collect();
        assert!(fit_sphere(&flat).is_none_or(|(_, r)| r.is_nan() || r > 0.0));
        assert_eq!(sector([0.0, 0.0, 0.0]), None);
        assert!(sector([1.0, 0.0, 0.0]).is_some());
    }
}
