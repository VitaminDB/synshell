//! Питание: аккумулятор подробно, регулятор частоты по кластерам,
//! гашение экрана и блокировка.

use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

/// Задержка глубокого сна после гашения экрана (телефон, `[idle] sleep_delay`), секунды.
const SLEEP_DELAYS: &[(i64, &str)] = &[
    (0, "Сразу"),
    (15, "15 секунд"),
    (30, "30 секунд"),
    (60, "1 минута"),
    (120, "2 минуты"),
    (300, "5 минут"),
    (600, "10 минут"),
    (900, "15 минут"),
    (1800, "30 минут"),
    (3600, "1 час"),
];

/// Записать регулятор частоты всех политик кластера (нужны права root).
fn set_governor(policy: &str, gov: &str) {
    let path = format!("/sys/devices/system/cpu/cpufreq/{policy}/scaling_governor");
    match std::fs::write(&path, gov) {
        Ok(()) => state::toast(format!("{policy}: регулятор {gov}")),
        Err(e) => state::toast(format!("Нет прав на {path}: {e}")),
    }
}

pub fn power() -> W {
    let sys = synsystem::Sys::host();
    let c = store::config();
    let mut body: Vec<W> = Vec::new();
    let bats = synsystem::battery::batteries(&sys);
    if let Some(sum) = synsystem::battery::read(&sys) {
        let state_text = if sum.charging {
            "Заряжается"
        } else if sum.full {
            "Заряжена"
        } else {
            "Разряжается"
        };
        let time = sum.minutes.map(|m| format!(" · осталось {}:{:02}", m / 60, m % 60)).unwrap_or_default();
        let mut rows: Vec<W> = vec![row_inline(
            "Заряд",
            &format!("{state_text}{time}"),
            Text::new(format!("{}%", sum.percent)).class("row-value big"),
        )];
        if let Some(w) = sum.power_w.filter(|w| *w > 0.05) {
            rows.push(row_inline("Мощность", "", Text::new(format!("{w:.1} Вт")).class("row-value")));
        }
        for b in &bats {
            if let Some(w) = b.wear_percent() {
                rows.push(row_inline("Ёмкость", "От паспортной", Text::new(format!("{w}%")).class("row-value")));
            }
            if let Some(n) = b.cycles {
                rows.push(row_inline("Циклов заряда", "", Text::new(n.to_string()).class("row-value")));
            }
            if let Some(t) = b.temp_c {
                rows.push(row_inline("Температура", "", Text::new(format!("{t:.1} °C")).class("row-value")));
            }
        }
        body.push(group("Аккумулятор", rows));
    } else {
        body.push(note("Драйвер аккумулятора не найден (на телефоне без ADSP его нет) — напряжение видно в «Оборудовании»."));
    }

    let clusters = synsystem::cpu::clusters(&sys);
    if !clusters.is_empty() {
        let mut rows: Vec<W> = Vec::new();
        for (i, cl) in clusters.iter().enumerate() {
            let avail = sys
                .read(format!("/sys/devices/system/cpu/cpufreq/{}/scaling_available_governors", cl.policy))
                .unwrap_or_default();
            let govs: Vec<String> = avail.split_whitespace().map(String::from).collect();
            if govs.is_empty() {
                continue;
            }
            let mut dd = Dropdown::new().width(200.0);
            for g in &govs {
                dd = dd.item(DropdownItem::new(g.clone(), g.clone()));
            }
            let policy = cl.policy.clone();
            let label = if clusters.len() > 1 && clusters.len() < 8 {
                format!("Кластер {} (ядра {:?})", i + 1, cl.cpus)
            } else {
                format!("Политика {}", cl.policy)
            };
            let hint = cl.min_mhz.zip(cl.max_mhz).map(|(a, b)| format!("{a}–{b} МГц")).unwrap_or_default();
            rows.push(row(&label, &hint, dd.selected(cl.governor.clone().unwrap_or_default()).on_change(move |g: &str| set_governor(&policy, g))));
            if clusters.len() >= 8 && rows.len() >= 2 {
                break;
            }
        }
        if !rows.is_empty() {
            body.push(group("Процессор", rows));
            body.push(note("performance — всегда максимум, schedutil/ondemand — по нагрузке, powersave — экономия. Нужны права root; до перезагрузки."));
        }
    }

    let mut screen: Vec<W> = vec![
        int_row("Гасить экран через", "Секунды без действий; 0 — никогда", op!["idle", "dpms_after"], c.idle.dpms_after as i64, 0, 7200, 30),
        int_row("Блокировать через", "Секунды; 0 — не блокировать", op!["idle", "lock_after"], c.idle.lock_after as i64, 0, 7200, 30),
    ];
    if c.process_form_factor() == synshell_common::config::FormFactor::Phone {
        // Телефон: усыпляет служба платформы syn-sleepd по флагу композитора; задержка — от гашения экрана
        screen.push(choice_int_row(
            "Глубокий сон после блокировки",
            "Через сколько после гашения экрана (кнопкой или по простою) телефон засыпает: сеть и Bluetooth во сне остаются, но сам он недоступен до пробуждения",
            op!["idle", "sleep_delay"],
            c.idle.sleep_delay as i64,
            SLEEP_DELAYS,
        ));
    } else {
        screen.push(int_row("Засыпать через", "Секунды; 0 — никогда", op!["idle", "suspend_after"], c.idle.suspend_after as i64, 0, 14400, 60));
    }
    screen.push(switch_row("Блокировать перед сном", "", op!["lock", "before_sleep"], c.lock.before_sleep));
    body.push(group("Экран и сон", screen));
    page("Питание", "Аккумулятор, частота процессора, гашение экрана и сон.", body)
}
