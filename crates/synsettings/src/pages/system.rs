//! Мониторы, меню запуска, уведомления, блокировка, автозапуск, сеанс,
//! «О системе».

use synshell_common::config::OutputConfig;
use synshell_common::ipc::OutputInfo;
use syngui::prelude::*;
use syngui::widgets::GestureDetector;
use toml_edit::{Array, Table, Value};

use crate::op;
use crate::state;
use crate::store;
use crate::sys;
use crate::ui::*;

// ─── Мониторы ───────────────────────────────────────────────────────────────

const TRANSFORMS: &[(&str, &str)] = &[
    ("normal", "Без поворота"),
    ("90", "90°"),
    ("180", "180°"),
    ("270", "270°"),
    ("flipped", "Отражение"),
    ("flipped-90", "Отражение + 90°"),
    ("flipped-180", "Отражение + 180°"),
    ("flipped-270", "Отражение + 270°"),
];

/// Индекс записи `[[output]]` для монитора `name`; создаёт запись при нужде.
fn output_index(name: &str) -> usize {
    let c = store::config();
    if let Some(i) = c.outputs.iter().position(|o| o.name == name) {
        return i;
    }
    let mut t = Table::new();
    t.insert("name", toml_edit::value(name));
    let mut idx = 0;
    store::edit(|d| {
        idx = store::doc_push_table(d, "output", t.clone());
        true
    });
    idx
}

fn oset(name: &str, key: &str, v: impl Into<Value>) {
    let i = output_index(name);
    set(&op!["output", i, key], v);
}

fn output_card(name: String, info: Option<&OutputInfo>, cfg: &OutputConfig) -> W {
    let mut rows: Vec<W> = Vec::new();
    let n = name.clone();
    rows.push(row("Включён", "", Toggle::with_state(cfg.enabled).on_change(move |v| oset(&n, "enabled", v))));

    // Режимы — из композитора, иначе поле ввода.
    let n = name.clone();
    match info.filter(|i| !i.modes.is_empty()) {
        Some(i) => {
            let mut dd = Dropdown::new().width(260.0).max_height(360.0).item(DropdownItem::new("", "Предпочтительный"));
            let mut seen = std::collections::BTreeSet::new();
            for m in &i.modes {
                let hz = (m.refresh_mhz as f64 / 1000.0).round() as i64;
                let v = format!("{}x{}@{}", m.width, m.height, hz);
                if !seen.insert(v.clone()) {
                    continue;
                }
                let label = format!("{} × {} · {} Гц{}", m.width, m.height, hz, if m.preferred { " ★" } else { "" });
                dd = dd.item(DropdownItem::new(v, label));
            }
            rows.push(row("Режим", "", dd.selected(cfg.mode.clone()).on_change(move |v: &str| oset(&n, "mode", v.to_string()))));
        }
        None => rows.push(row(
            "Режим",
            "Например 1920x1080@60; пусто — предпочтительный",
            TextField::with_text(cfg.mode.clone()).placeholder("1920x1080@60").width(200.0).on_change(move |s| oset(&n, "mode", s.to_string())),
        )),
    }
    let n = name.clone();
    let auto = cfg.scale <= 0.0;
    let cur_scale = if auto { info.map(|i| i.scale).unwrap_or(1.0) } else { cfg.scale };
    let n2 = name.clone();
    rows.push(row(
        "Масштаб",
        "Выключите «авто», чтобы задать вручную",
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Slider::new().range(0.5, 3.0).step(0.05).value(cur_scale as f32).show_value(2).width(200.0).on_change(move |v| {
                oset(&n, "scale", round_to(v as f64, 2))
            }))
            .child(Text::new("авто").class("row-hint"))
            .child(Toggle::with_state(auto).on_change(move |v| {
                oset(&n2, "scale", if v { 0.0 } else { round_to(cur_scale, 2) });
                state::bump();
            })),
    ));
    let n = name.clone();
    rows.push(row(
        "Яркость на максимуме",
        "Ниты панели при полной подсветке (паспорт); для яркости в нитах; 0 — не задано",
        SpinBox::new().range(0.0, 10000.0).step(50.0).value(cfg.max_nits.unwrap_or(0.0) as f64).width(140.0).on_change(move |v| {
            if v <= 0.0 {
                crate::ui::unset(&crate::op!["output", output_index(&n), "max_nits"]);
            } else {
                oset(&n, "max_nits", v.round() as i64);
            }
        }),
    ));
    let n = name.clone();
    let n2 = name.clone();
    let pos = cfg.position;
    let (px, py) = pos.map(|p| (p[0], p[1])).unwrap_or((0, 0));
    let xy = use_signal((px, py));
    let write_pos = move |nm: &str| {
        let (x, y) = xy.get_untracked();
        let mut a = Array::new();
        a.push(x as i64);
        a.push(y as i64);
        oset(nm, "position", Value::Array(a));
    };
    rows.push(row(
        "Положение",
        "Логические px; пусто — справа от предыдущего",
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(SpinBox::new().range(-16384.0, 16384.0).value(px as f64).width(120.0).on_change(move |v| {
                xy.update(|p| p.0 = v.round() as i32);
                write_pos(&n);
            }))
            .child(SpinBox::new().range(-16384.0, 16384.0).value(py as f64).width(120.0).on_change(move |v| {
                xy.update(|p| p.1 = v.round() as i32);
                write_pos(&n2);
            })),
    ));
    let n = name.clone();
    rows.push(row(
        "Поворот",
        "",
        {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in TRANSFORMS {
                dd = dd.item(DropdownItem::new(*v, *l));
            }
            dd.selected(cfg.transform.clone()).on_change(move |v: &str| oset(&n, "transform", v.to_string()))
        },
    ));
    let n = name.clone();
    rows.push(row("Переменная частота (VRR)", "", Toggle::with_state(cfg.vrr).on_change(move |v| oset(&n, "vrr", v))));
    let n = name.clone();
    rows.push(row(
        "Основной монитор",
        "Панель по умолчанию и новые окна",
        Toggle::with_state(cfg.primary).on_change(move |v| {
            // Основной — только один.
            if v {
                let c = store::config();
                for (i, o) in c.outputs.iter().enumerate() {
                    if o.primary && o.name != n {
                        set(&op!["output", i, "primary"], false);
                    }
                }
            }
            oset(&n, "primary", v);
        }),
    ));

    let subtitle = match info {
        Some(i) => {
            let mut parts = vec![];
            if !i.description.is_empty() {
                parts.push(i.description.clone());
            }
            parts.push(format!("{} × {}", i.geometry[2], i.geometry[3]));
            if i.physical_mm[0] > 0 {
                let diag = ((i.physical_mm[0].pow(2) + i.physical_mm[1].pow(2)) as f64).sqrt() / 25.4;
                parts.push(format!("{diag:.1}\""));
            }
            parts.join(" · ")
        }
        None => "Не подключён или композитор не запущен".into(),
    };
    let in_config = store::config().outputs.iter().position(|o| o.name == name);
    let mut header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Column::new().class("grow").gap(2.0).child(Text::new(name.clone()).class("group-title")).child(Text::new(subtitle).class("row-hint")));
    if let Some(i) = in_config {
        header = header.child(Button::new("Сбросить").class("btn small").icon(icons::UNDO).on_click(move || {
            unset(&op!["output", i]);
            state::bump();
        }));
    }
    boxed(Column::new().gap(8.0).child(header).child(group("", rows)))
}

/// Яркость подсветки: проценты и ниты (если у вывода задан `max_nits`).
fn brightness_group(c: &synshell_common::Config) -> Option<W> {
    let sysfs = synsystem::Sys::host();
    let bl = synsystem::backlight::primary(&sysfs)?;
    let nits = c.outputs.iter().find_map(|o| o.max_nits);
    let pct = use_signal(bl.percent());
    let label = Reactive::new(move || -> Vec<W> {
        let p = pct.get();
        let t = match nits {
            Some(max) => format!("{:.0} нит · {p:.0}%", p / 100.0 * max),
            None => format!("{p:.0}%"),
        };
        vec![boxed(Text::new(t).class("row-value"))]
    });
    let slider = Slider::new().range(1.0, 100.0).step(1.0).value(bl.percent()).width(220.0).on_change(move |v| {
        pct.set(v);
        std::thread::spawn(move || {
            if let Err(e) = synsystem::backlight::set_percent(&synsystem::Sys::host(), v) {
                syngui::async_runtime::run_on_main_thread(move || state::toast(format!("Яркость: {e}")));
            }
        });
    });
    let hint = if nits.is_some() {
        format!("Подсветка {}", bl.name)
    } else {
        format!("Подсветка {} · ниты — задайте яркость панели на максимуме ниже", bl.name)
    };
    let b = &c.brightness;
    Some(group(
        "Яркость",
        vec![
            row("Яркость экрана", &hint, Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(slider).child(label)),
            switch_row(
                "Автояркость",
                "По датчику освещённости; сдвиг ползунка запоминается как поправка",
                op!["brightness", "auto"],
                b.auto,
            ),
            int_row("Минимум автояркости, %", "В темноте", op!["brightness", "min_pct"], b.min_pct as i64, 1, 50, 1),
            int_row("Максимум автояркости, %", "На солнце", op!["brightness", "max_pct"], b.max_pct as i64, 20, 100, 5),
        ],
    ))
}

pub fn displays() -> W {
    let c = store::config();
    let live = sys::outputs();
    let mut body: Vec<W> = Vec::new();
    if let Some(g) = brightness_group(&c) {
        body.push(g);
    }
    let mut names: Vec<String> = Vec::new();
    match &live {
        Some(outs) => {
            for o in outs {
                names.push(o.name.clone());
            }
        }
        None => body.push(note(
            "Композитор synshell не запущен — показаны записи из config.toml. Режимы можно вписать вручную.",
        )),
    }
    for o in &c.outputs {
        if !names.contains(&o.name) && !o.name.is_empty() {
            names.push(o.name.clone());
        }
    }
    for name in names {
        let info = live.as_ref().and_then(|l| l.iter().find(|o| o.name == name));
        let cfg = c.outputs.iter().find(|o| o.name == name).cloned().unwrap_or_default();
        body.push(output_card(name, info, &cfg));
    }
    let new_name = use_signal(String::new());
    body.push(group(
        "Добавить запись монитора",
        vec![row(
            "Имя вывода или описание",
            "eDP-1, HDMI-A-1, «Dell U2720Q»",
            Row::new()
                .gap(8.0)
                .child(TextField::new().placeholder("HDMI-A-1").width(200.0).on_change(move |s| new_name.set(s.to_string())))
                .child(primary_button("Добавить", move || {
                    let n = new_name.get_untracked().trim().to_string();
                    if !n.is_empty() {
                        output_index(&n);
                        state::bump();
                    }
                })),
        )],
    ));
    page("Мониторы", "Разрешение, частота, масштаб, расположение и поворот.", body)
}

// ─── Меню запуска ───────────────────────────────────────────────────────────

fn favorites_editor(favs: Vec<String>) -> W {
    let mut col = Column::new().gap(4.0);
    let n = favs.len();
    let write = |v: &[String]| {
        let mut a = Array::new();
        for s in v {
            a.push(s.as_str());
        }
        set(&op!["launcher", "favorites"], Value::Array(a));
        state::bump();
    };
    for (i, f) in favs.iter().enumerate() {
        let (a, b, c) = (favs.clone(), favs.clone(), favs.clone());
        col = col.child(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("chip-row")
                .child(Text::new(f.clone()).class("grow"))
                .child(icon_button(icons::UP, move || {
                    if i > 0 {
                        let mut v = a.clone();
                        v.swap(i, i - 1);
                        write(&v);
                    }
                }))
                .child(icon_button(icons::DOWN, move || {
                    if i + 1 < n {
                        let mut v = b.clone();
                        v.swap(i, i + 1);
                        write(&v);
                    }
                }))
                .child(danger_icon_button(icons::DELETE, move || {
                    let mut v = c.clone();
                    v.remove(i);
                    write(&v);
                })),
        );
    }
    let new = use_signal(String::new());
    let apps = desktop_ids();
    let favs2 = favs.clone();
    col = col.child(
        Row::new()
            .gap(8.0)
            .child(Autocomplete::new(apps).placeholder("org.kde.dolphin").width(280.0).on_change(move |s| new.set(s.to_string())).on_select(move |s| new.set(s.to_string())))
            .child(primary_button("Добавить", move || {
                let s = new.get_untracked().trim().trim_end_matches(".desktop").to_string();
                if !s.is_empty() && !favs2.contains(&s) {
                    let mut v = favs2.clone();
                    v.push(s);
                    write(&v);
                }
            })),
    );
    boxed(col)
}

/// Имена .desktop-файлов приложений (без расширения).
fn desktop_ids() -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for d in synshell_common::paths::data_dirs() {
        if let Ok(rd) = std::fs::read_dir(d.join("applications")) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(id) = n.strip_suffix(".desktop") {
                    set.insert(id.to_string());
                }
            }
        }
    }
    set.into_iter().collect()
}

pub fn launcher() -> W {
    let c = store::config();
    let l = &c.launcher;
    page(
        "Меню запуска",
        "Меню приложений на панели и по Super+D.",
        vec![
            group(
                "Вид",
                vec![
                    choice_row("Стиль", "", op!["launcher", "style"], &l.style, &[("menu", "Меню у кнопки"), ("fullscreen", "На весь экран")]),
                    int_row("Колонок в сетке", "", op!["launcher", "columns"], l.columns as i64, 2, 12, 1),
                    int_row("Ширина меню", "px", op!["launcher", "width"], l.width as i64, 320, 1600, 10),
                    int_row("Высота меню", "px", op!["launcher", "height"], l.height as i64, 240, 1200, 10),
                    switch_row("Категории", "", op!["launcher", "show_categories"], l.show_categories),
                    switch_row("Недавние", "", op!["launcher", "show_recent"], l.show_recent),
                ],
            ),
            group(
                "Поиск",
                vec![
                    switch_row("Искать по ключевым словам и команде", "", op!["launcher", "search_keywords"], l.search_keywords),
                    switch_row("Калькулятор", "Выражения вроде 2+2*3 прямо в поиске", op!["launcher", "calculator"], l.calculator),
                    switch_row("Запуск команд", "Enter запускает введённое, если ничего не найдено", op!["launcher", "run_commands"], l.run_commands),
                ],
            ),
            group("Избранное", vec![row_wide("", "Имена .desktop без расширения", favorites_editor(l.favorites.clone()))]),
        ],
    )
}

// ─── Уведомления ────────────────────────────────────────────────────────────

/// Места уведомлений: (значение, подпись).
const POSITIONS: [(&str, &str); 6] = [
    ("top-left", "Сверху слева"),
    ("top", "Сверху по центру"),
    ("top-right", "Сверху справа"),
    ("bottom-left", "Снизу слева"),
    ("bottom", "Снизу по центру"),
    ("bottom-right", "Снизу справа"),
];

/// Мини-экран с шестью точками (три сверху, три снизу): где всплывают уведомления.
fn position_picker(current: &str, phone: bool) -> W {
    let auto = current.trim().is_empty() || current == "auto";
    let effective = if auto { if phone { "top" } else { "top-right" } } else { current };
    let sel = use_signal(effective.to_string());
    let dot = move |key: &'static str| {
        GestureDetector::new()
            .on_click(move || {
                sel.set(key.to_string());
                set(&op!["notifications", "position"], key);
            })
            .child(Reactive::new(move || -> Vec<W> {
                let on = sel.get() == key;
                vec![boxed(DecoratedBox::new().class(if on { "pos-dot pos-dot-on" } else { "pos-dot" }))]
            }))
    };
    let line = move |keys: &[&'static str]| {
        let mut r = Row::new().main_axis_alignment(MainAxisAlignment::SpaceBetween);
        for k in keys {
            r = r.child(dot(k));
        }
        r
    };
    // Телефон — вертикальный экран, компьютер — горизонтальный
    let screen = Column::new()
        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .child(line(&["top-left", "top", "top-right"]))
        .child(line(&["bottom-left", "bottom", "bottom-right"]))
        .class(if phone { "pos-screen pos-screen-phone" } else { "pos-screen pos-screen-desk" });
    let caption = Reactive::new(move || -> Vec<W> {
        let k = sel.get();
        let name = POSITIONS.iter().find(|(v, _)| *v == k).map(|(_, l)| *l).unwrap_or("");
        vec![boxed(Text::new(name.to_string()).class("row-hint"))]
    });
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Start).child(screen).child(caption))
}

pub fn notifications() -> W {
    let c = store::config();
    let n = &c.notifications;
    let phone = c.process_form_factor() == synshell_common::config::FormFactor::Phone;
    page(
        "Уведомления",
        "Всплывающие уведомления и центр уведомлений (org.freedesktop.Notifications).",
        vec![
            group(
                "",
                vec![
                    switch_row("Сервер уведомлений", "Выключите, если используете mako/dunst", op!["notifications", "enabled"], n.enabled),
                    switch_row("Не беспокоить", "Показывать только критичные", op!["notifications", "do_not_disturb"], n.do_not_disturb),
                    row_wide("Положение", "Нажмите точку — оттуда будут появляться уведомления", position_picker(&n.position, phone)),
                    int_row("Время показа", "мс", op!["notifications", "timeout"], n.timeout as i64, 1000, 60000, 500),
                    int_row("Критичные", "мс, 0 — пока не закроют", op!["notifications", "critical_timeout"], n.critical_timeout as i64, 0, 600000, 1000),
                    int_row("Одновременно на экране", "", op!["notifications", "max_visible"], n.max_visible as i64, 1, 20, 1),
                    int_row("Ширина", "px", op!["notifications", "width"], n.width as i64, 240, 800, 10),
                ],
            ),
            group(
                "История",
                vec![
                    switch_row("Хранить историю", "", op!["notifications", "history"], n.history),
                    int_row("Размер истории", "", op!["notifications", "history_size"], n.history_size as i64, 10, 1000, 10),
                ],
            ),
        ],
    )
}

// ─── Блокировка и простой ───────────────────────────────────────────────────

pub fn lock() -> W {
    let c = store::config();
    let l = &c.lock;
    let i = &c.idle;
    page(
        "Блокировка и простой",
        "Что делать, когда компьютер не используется.",
        vec![
            group(
                "Блокировка экрана",
                vec![
                    text_row("Внешний экран блокировки", "Пусто — встроенный", op!["lock", "command"], &l.command, "swaylock -f"),
                    switch_row("Блокировать перед сном", "", op!["lock", "before_sleep"], l.before_sleep),
                    choice_row(
                        "Снятие блокировки",
                        "Свайп — без проверки (удобно, если пароль не задан)",
                        op!["lock", "method"],
                        &l.method,
                        &[("password", "Паролем пользователя"), ("swipe", "Свайпом")],
                    ),
                ],
            ),
            group(
                "Простой",
                vec![
                    int_row("Погасить мониторы через", "секунды, 0 — никогда", op!["idle", "dpms_after"], i.dpms_after as i64, 0, 86400, 30),
                    int_row("Заблокировать через", "секунды, 0 — никогда", op!["idle", "lock_after"], i.lock_after as i64, 0, 86400, 30),
                    int_row("Уснуть через", "секунды, 0 — никогда", op!["idle", "suspend_after"], i.suspend_after as i64, 0, 86400, 60),
                ],
            ),
        ],
    )
}

// ─── Автозапуск ─────────────────────────────────────────────────────────────

fn write_autostart(v: &[String]) {
    let mut a = Array::new();
    for s in v {
        a.push(s.as_str());
    }
    set(&op!["general", "autostart"], Value::Array(a));
    state::bump();
}

pub fn autostart() -> W {
    let c = store::config();
    let cmds = c.general.autostart.clone();
    let mut cmd_rows: Vec<W> = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        let all = cmds.clone();
        let all2 = cmds.clone();
        cmd_rows.push(boxed(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("setting-row")
                .child(TextField::with_text(cmd.clone()).width(460.0).on_change(move |s| {
                    let mut v = all.clone();
                    v[i] = s.to_string();
                    let mut a = Array::new();
                    for s in &v {
                        a.push(s.as_str());
                    }
                    set(&op!["general", "autostart"], Value::Array(a));
                }))
                .child(DecoratedBox::new().class("grow"))
                .child(danger_icon_button(icons::DELETE, move || {
                    let mut v = all2.clone();
                    v.remove(i);
                    write_autostart(&v);
                })),
        ));
    }
    let new = use_signal(String::new());
    let all = cmds.clone();
    cmd_rows.push(boxed(
        Row::new()
            .gap(8.0)
            .class("setting-row")
            .child(TextField::new().placeholder("nm-applet --indicator").width(460.0).on_change(move |s| new.set(s.to_string())))
            .child(primary_button("Добавить", move || {
                let s = new.get_untracked().trim().to_string();
                if !s.is_empty() {
                    let mut v = all.clone();
                    v.push(s);
                    write_autostart(&v);
                }
            })),
    ));

    let entries = sys::autostart_entries();
    let mut xdg_rows: Vec<W> = Vec::new();
    for e in entries {
        let file = e.file.clone();
        let file2 = e.file.clone();
        let hint = if e.only_in.is_empty() { e.exec.clone() } else { format!("{} · только в {}", e.exec, e.only_in) };
        let mut r = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Toggle::with_state(e.enabled).on_change(move |v| {
                if let Err(err) = sys::set_autostart_enabled(&file, v) {
                    state::toast(format!("Не удалось: {err}"));
                }
            }));
        if e.user {
            r = r.child(danger_icon_button(icons::DELETE, move || {
                match sys::remove_user_autostart(&file2) {
                    Ok(()) => state::bump(),
                    Err(err) => state::toast(format!("Не удалось: {err}")),
                }
            }));
        }
        xdg_rows.push(row(&e.name, &hint, r));
    }
    if xdg_rows.is_empty() {
        xdg_rows.push(note("Записей нет."));
    }
    page(
        "Автозапуск",
        "Программы, которые запускаются при входе в сеанс.",
        vec![
            group("Команды synshell", cmd_rows),
            group(
                "",
                vec![switch_row(
                    "Запускать XDG Autostart",
                    "~/.config/autostart и /etc/xdg/autostart",
                    op!["general", "xdg_autostart"],
                    c.general.xdg_autostart,
                )],
            ),
            group("XDG Autostart", xdg_rows),
        ],
    )
}

// ─── Сеанс ──────────────────────────────────────────────────────────────────

pub fn general() -> W {
    let c = store::config();
    let g = &c.general;
    let mut env_rows: Vec<W> = Vec::new();
    for (k, v) in &g.environment {
        let k2 = k.clone();
        env_rows.push(row(
            k,
            "",
            Row::new()
                .gap(8.0)
                .child(text(op!["general", "environment", k.as_str()], v, "", 300.0))
                .child(danger_icon_button(icons::DELETE, move || {
                    unset(&op!["general", "environment", k2.as_str()]);
                    state::bump();
                })),
        ));
    }
    let nk = use_signal(String::new());
    let nv = use_signal(String::new());
    env_rows.push(row(
        "Новая переменная",
        "",
        Row::new()
            .gap(8.0)
            .child(TextField::new().placeholder("QT_QPA_PLATFORMTHEME").width(200.0).on_change(move |s| nk.set(s.to_string())))
            .child(TextField::new().placeholder("kde").width(160.0).on_change(move |s| nv.set(s.to_string())))
            .child(icon_button(icons::ADD, move || {
                let k = nk.get_untracked().trim().to_string();
                if !k.is_empty() {
                    set(&op!["general", "environment", k], nv.get_untracked());
                    state::bump();
                }
            })),
    ));
    page(
        "Сеанс",
        "Программы по умолчанию, оболочка и окружение.",
        vec![
            group(
                "Программы по умолчанию",
                vec![
                    text_row("Терминал", "Super+Return, Ctrl+Alt+T", op!["general", "terminal"], &g.terminal, "konsole"),
                    text_row("Файловый менеджер", "Super+E", op!["general", "file_manager"], &g.file_manager, "dolphin"),
                    text_row("Браузер", "", op!["general", "browser"], &g.browser, "firefox"),
                ],
            ),
            group(
                "Сеанс",
                vec![
                    text_row("Оболочка", "Пусто — без оболочки (например, waybar)", op!["general", "shell"], &g.shell, "syndesktop-shell"),
                    switch_row("Xwayland", "X11-программы (нужен пакет xorg-xwayland)", op!["general", "xwayland"], g.xwayland),
                    text_row("Каталог снимков экрана", "", op!["general", "screenshot_dir"], &g.screenshot_dir, "~/Pictures/Screenshots"),
                ],
            ),
            group("Переменные окружения", env_rows),
        ],
    )
}

// ─── О системе ──────────────────────────────────────────────────────────────

pub fn about() -> W {
    let a = sys::about();
    let compositor = match sys::ipc(synshell_common::ipc::Request::Version) {
        Some(synshell_common::ipc::Response::Version { version }) => format!("запущен, {version}"),
        _ => "не запущен".into(),
    };
    let kv = |k: &str, v: String| -> W {
        boxed(
            Row::new()
                .gap(16.0)
                .class("setting-row")
                .child(Text::new(k).class("row-hint about-key"))
                .child(Text::new(v).selectable(true).class("row-label grow")),
        )
    };
    let mut rows = vec![
        kv("Окружение", format!("synshell {}", env!("CARGO_PKG_VERSION"))),
        kv("Композитор", compositor),
        kv("Система", a.os),
        kv("Ядро", a.kernel),
        kv("Компьютер", a.hostname),
        kv("Процессор", format!("{} ({} потоков)", a.cpu, a.cores)),
        kv("Память", a.memory),
    ];
    for g in a.gpus {
        rows.push(kv("Видеокарта", g));
    }
    if !a.session.is_empty() {
        rows.push(kv("Текущий сеанс", a.session));
    }
    rows.push(kv("Файл настроек", store::path().display().to_string()));
    page(
        "О системе",
        "",
        vec![
            boxed(
                Column::new()
                    .gap(4.0)
                    .class("about-hero")
                    .child(Text::new("synshell").class("about-title"))
                    .child(Text::new("Рабочий стол для Wayland на Rust и syngui").class("row-hint")),
            ),
            group("", rows),
        ],
    )
}
