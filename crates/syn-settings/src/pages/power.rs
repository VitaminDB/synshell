//! Питание: аккумулятор подробно, регулятор частоты по кластерам,
//! гашение экрана и блокировка, оповещения о заряде, сне и выключении (`[power_alerts]`).

use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

/// Задержка глубокого сна после гашения экрана (телефон, `[idle] sleep_delay`), секунды.
const SLEEP_DELAYS: &[(i64, &str)] = &[
    (0, n_!("Сразу")),
    (15, n_!("15 секунд")),
    (30, n_!("30 секунд")),
    (60, n_!("1 минута")),
    (120, n_!("2 минуты")),
    (300, n_!("5 минут")),
    (600, n_!("10 минут")),
    (900, n_!("15 минут")),
    (1800, n_!("30 минут")),
    (3600, n_!("1 час")),
    (synshell_common::config::SLEEP_NEVER as i64, n_!("Никогда")),
];

const PERCENTS_LOW: &[(i64, &str)] = &[(0, n_!("Нет")), (10, "10 %"), (15, "15 %"), (20, "20 %"), (25, "25 %"), (30, "30 %")];
const PERCENTS_CRITICAL: &[(i64, &str)] = &[(0, n_!("Нет")), (5, "5 %"), (7, "7 %"), (10, "10 %"), (15, "15 %")];
const PERCENTS_ACTION: &[(i64, &str)] = &[(1, "1 %"), (2, "2 %"), (3, "3 %"), (5, "5 %"), (7, "7 %")];
const DELAYS: &[(i64, &str)] = &[(15, n_!("15 секунд")), (30, n_!("30 секунд")), (60, n_!("1 минута")), (120, n_!("2 минуты"))];
const SLEEP_WARNINGS: &[(i64, &str)] = &[(0, n_!("Нет")), (30, n_!("30 секунд")), (60, n_!("1 минута")), (120, n_!("2 минуты")), (300, n_!("5 минут"))];

/// Записать регулятор частоты всех политик кластера (нужны права root).
fn set_governor(policy: &str, gov: &str) {
    let path = format!("/sys/devices/system/cpu/cpufreq/{policy}/scaling_governor");
    match std::fs::write(&path, gov) {
        Ok(()) => state::toast(t!("{policy}: регулятор {gov}", policy = policy, gov = gov)),
        Err(e) => state::toast(t!("Нет прав на {path}: {e}", path = path, e = e)),
    }
}

pub fn power() -> W {
    let sys = syn_system::Sys::host();
    let c = store::config();
    let mut body: Vec<W> = Vec::new();
    let bats = syn_system::battery::batteries(&sys);
    if let Some(sum) = syn_system::battery::read(&sys) {
        let state_text = if sum.charging {
            t!("Заряжается")
        } else if sum.full {
            t!("Заряжена")
        } else {
            t!("Разряжается")
        };
        let time = sum.minutes.map(|m| t!(" · осталось {v}:{v2}", v = m / 60, v2 = format!("{:02}", m % 60))).unwrap_or_default();
        let mut rows: Vec<W> = vec![row_inline(
            &t!("Заряд"),
            &format!("{state_text}{time}"),
            Text::new(format!("{}%", sum.percent)).class("row-value big"),
        )];
        if let Some(w) = sum.power_w.filter(|w| *w > 0.05) {
            rows.push(row_inline(&t!("Мощность"), "", Text::new(t!("{w} Вт", w = format!("{:.1}", w))).class("row-value")));
        }
        for b in &bats {
            if let Some(w) = b.wear_percent() {
                rows.push(row_inline(t!("Ёмкость"), t!("От паспортной"), Text::new(format!("{w}%")).class("row-value")));
            }
            if let Some(n) = b.cycles {
                rows.push(row_inline(&t!("Циклов заряда"), "", Text::new(n.to_string()).class("row-value")));
            }
            if let Some(t) = b.temp_c {
                rows.push(row_inline(&t!("Температура"), "", Text::new(format!("{t:.1} °C")).class("row-value")));
            }
        }
        body.push(group(&t!("Аккумулятор"), rows));
    } else {
        body.push(note(&t!("Драйвер аккумулятора не найден (на телефоне без ADSP его нет) — напряжение видно в «Оборудовании».")));
    }

    let clusters = syn_system::cpu::clusters(&sys);
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
                t!("Кластер {v} (ядра {cpus})", v = i + 1, cpus = format!("{:?}", cl.cpus))
            } else {
                t!("Политика {policy}", policy = cl.policy)
            };
            let hint = cl.min_mhz.zip(cl.max_mhz).map(|(a, b)| t!("{a}–{b} МГц", a = a, b = b)).unwrap_or_default();
            rows.push(row(&label, &hint, dd.selected(cl.governor.clone().unwrap_or_default()).on_change(move |g: &str| set_governor(&policy, g))));
            if clusters.len() >= 8 && rows.len() >= 2 {
                break;
            }
        }
        if !rows.is_empty() {
            body.push(group(&t!("Процессор"), rows));
            body.push(note(&t!("performance — всегда максимум, schedutil/ondemand — по нагрузке, powersave — экономия. Нужны права root; до перезагрузки.")));
        }
    }

    let mut screen: Vec<W> = vec![
        int_row(t!("Гасить экран через"), t!("Секунды без действий; 0 — никогда"), op!["idle", "dpms_after"], c.idle.dpms_after as i64, 0, 7200, 30),
        int_row(t!("Блокировать через"), t!("Секунды; 0 — не блокировать"), op!["idle", "lock_after"], c.idle.lock_after as i64, 0, 7200, 30),
    ];
    if c.process_form_factor() == synshell_common::config::FormFactor::Phone {
        // Телефон: усыпляет служба платформы syn-sleepd по флагу композитора; задержка — от гашения экрана
        screen.push(choice_int_row(
            t!("Глубокий сон после блокировки"),
            t!("Через сколько после гашения экрана (кнопкой или по простою) телефон засыпает: сеть и Bluetooth во сне остаются, но сам он недоступен до пробуждения"),
            op!["idle", "sleep_delay"],
            c.idle.sleep_delay as i64,
            SLEEP_DELAYS,
        ));
    } else {
        screen.push(int_row(t!("Засыпать через"), t!("Секунды; 0 — никогда"), op!["idle", "suspend_after"], c.idle.suspend_after as i64, 0, 14400, 60));
    }
    screen.push(switch_row(&t!("Блокировать перед сном"), "", op!["lock", "before_sleep"], c.lock.before_sleep));
    body.push(group(&t!("Экран и сон"), screen));

    // Оповещения: заряд, выключение у нуля, сон, неожиданные выключения, соединённые устройства
    let a = &c.power_alerts;
    let has_battery = syn_system::battery::read(&sys).is_some();
    let mut alerts: Vec<W> = Vec::new();
    if has_battery {
        alerts.push(choice_int_row(t!("Низкий заряд"), t!("Уведомление при разряде до этого уровня"), op!["power_alerts", "low"], a.low as i64, PERCENTS_LOW));
        alerts.push(choice_int_row(
            t!("Критический заряд"),
            t!("Срочное уведомление со звуком — видно и в «Не беспокоить»"),
            op!["power_alerts", "critical"],
            a.critical as i64,
            PERCENTS_CRITICAL,
        ));
        alerts.push(choice_row(
            t!("У самого нуля"),
            t!("Выключиться чисто, пока есть заряд, — файлы не испортятся, а после включения будет видно, почему устройство погасло"),
            op!["power_alerts", "action"],
            &a.action,
            &[("poweroff", t!("Выключить")), ("suspend", t!("Уснуть")), ("none", t!("Ничего"))],
        ));
        if a.action != "none" {
            alerts.push(choice_int_row(t!("Порог"), t!("Заряд, при котором начинается отсчёт"), op!["power_alerts", "action_at"], a.action_at as i64, PERCENTS_ACTION));
            alerts.push(choice_int_row(
                t!("Отсчёт"),
                t!("Время, чтобы отменить или подключить зарядку; экран включается сам"),
                op!["power_alerts", "action_delay"],
                a.action_delay as i64,
                DELAYS,
            ));
        }
    }
    if c.process_form_factor() != synshell_common::config::FormFactor::Phone && c.idle.suspend_after > 0 {
        alerts.push(choice_int_row(
            t!("Предупреждать о сне"),
            t!("За сколько до сна по простою показать «Сон через…» с кнопкой «Не засыпать»"),
            op!["power_alerts", "sleep_warning"],
            a.sleep_warning as i64,
            SLEEP_WARNINGS,
        ));
    }
    alerts.push(switch_row(
        t!("Сообщать о неожиданном выключении"),
        t!("После включения — когда и при каком заряде устройство погасло без завершения работы"),
        op!["power_alerts", "unexpected"],
        a.unexpected,
    ));
    alerts.push(switch_row(
        t!("Соединённые устройства"),
        t!("Показывать, что телефон или компьютер уснул, выключается или пропал при разряде (synlink); свои оповещения о заряде уходят им же"),
        op!["power_alerts", "linked"],
        a.linked,
    ));
    body.push(group(&t!("Оповещения"), alerts));
    page(t!("Питание"), t!("Аккумулятор, частота процессора, гашение экрана, сон и оповещения."), body)
}
