//! Поведение окон, рабочие столы, правила окон.

use synshell_common::action::LayoutKind;
use synshell_common::config::{DecorationMode, FocusMode, Placement, WindowRule};
use syngui::prelude::*;
use toml_edit::{Array, Table, Value};

use crate::op;
use crate::state;
use crate::store;
use crate::sys;
use crate::ui::*;

const LAYOUTS: &[(&str, &str)] = &[
    ("floating", "Свободные окна"),
    ("tile", "Мастер и стопка"),
    ("columns", "Колонки"),
    ("grid", "Сетка"),
    ("monocle", "Одно окно"),
];

const TITLEBAR_ACTIONS: &[(&str, &str)] = &[
    ("toggle-maximize", "Развернуть/восстановить"),
    ("minimize", "Свернуть"),
    ("toggle-fullscreen", "Во весь экран"),
    ("toggle-sticky", "На всех столах"),
    ("toggle-always-on-top", "Поверх других"),
    ("close", "Закрыть"),
    ("none", "Ничего"),
];

pub fn windows() -> W {
    let c = store::config();
    let w = &c.windows;
    let focus = match w.focus_mode {
        FocusMode::Click => "click",
        FocusMode::Sloppy => "sloppy",
        FocusMode::FollowMouse => "follow-mouse",
    };
    let deco = match w.decorations {
        DecorationMode::Server => "server",
        DecorationMode::Client => "client",
    };
    let placement = match w.placement {
        Placement::Center => "center",
        Placement::Cascade => "cascade",
        Placement::UnderMouse => "under-mouse",
        Placement::Smart => "smart",
    };
    let dbl = w.titlebar_double_click.to_string();
    let mid = w.titlebar_middle_click.to_string();
    page(
        "Поведение окон",
        "Фокус, размещение, рамки, прилипание и плиточные раскладки.",
        vec![
            group(
                "Фокус",
                vec![
                    choice_row(
                        "Фокус окна",
                        "",
                        op!["windows", "focus_mode"],
                        focus,
                        &[
                            ("click", "По щелчку"),
                            ("sloppy", "За курсором (не терять над столом)"),
                            ("follow-mouse", "Строго под курсором"),
                        ],
                    ),
                    switch_row("Поднимать окно при фокусе", "", op!["windows", "raise_on_focus"], w.raise_on_focus),
                    int_row(
                        "Задержка поднятия",
                        "При фокусе за курсором, мс",
                        op!["windows", "autoraise_delay"],
                        w.autoraise_delay as i64,
                        0,
                        3000,
                        50,
                    ),
                    switch_row(
                        "Разрешить окнам забирать фокус",
                        "Иначе окно, попросившее фокус, только помечается «требует внимания»",
                        op!["windows", "focus_stealing"],
                        w.focus_stealing,
                    ),
                ],
            ),
            group(
                "Размещение и рамки",
                vec![
                    choice_row(
                        "Новые окна",
                        "",
                        op!["windows", "placement"],
                        placement,
                        &[
                            ("center", "По центру"),
                            ("cascade", "Каскадом"),
                            ("under-mouse", "Под курсором"),
                            ("smart", "Где меньше перекрытие"),
                        ],
                    ),
                    choice_row(
                        "Рамки окон",
                        "",
                        op!["windows", "decorations"],
                        deco,
                        &[("server", "Рисует композитор"), ("client", "Рисует приложение")],
                    ),
                    switch_row(
                        "Развёрнутые окна без заголовка",
                        "Заголовок, кнопки и меню развёрнутого окна показывает панель (апплеты «Заголовок окна», «Кнопки окна», «Глобальное меню»)",
                        op!["windows", "borderless_maximized"],
                        w.borderless_maximized,
                    ),
                    int_row("Толщина рамки", "px", op!["windows", "border_width"], w.border_width as i64, 0, 16, 1),
                    choice_row(
                        "Клавиша-модификатор",
                        "С ней: ЛКМ — перетащить окно, ПКМ — изменить размер",
                        op!["windows", "mod_key"],
                        &w.mod_key,
                        &[("Super", "Super (Win)"), ("Alt", "Alt"), ("Ctrl", "Ctrl")],
                    ),
                    slider_row(
                        "Затемнять неактивные окна",
                        "0 — не затемнять",
                        op!["windows", "dim_inactive"],
                        w.dim_inactive as f64,
                        0.0,
                        0.6,
                        0.01,
                        2,
                    ),
                    int_row(
                        "Прятать курсор",
                        "Через N мс без движения (0 — никогда)",
                        op!["windows", "hide_cursor_after"],
                        w.hide_cursor_after as i64,
                        0,
                        60000,
                        500,
                    ),
                ],
            ),
            group(
                "Прилипание",
                vec![
                    switch_row("Прилипать к краям и окнам", "", op!["windows", "snap"], w.snap),
                    int_row("Расстояние прилипания", "px", op!["windows", "snap_distance"], w.snap_distance as i64, 0, 64, 1),
                    switch_row(
                        "Половина экрана у края",
                        "Перетащите окно к краю — оно займёт половину (как Aero Snap)",
                        op!["windows", "edge_tiling"],
                        w.edge_tiling,
                    ),
                ],
            ),
            group(
                "Плиточная раскладка",
                vec![
                    choice_row(
                        "Раскладка по умолчанию",
                        "Для новых столов",
                        op!["windows", "default_layout"],
                        w.default_layout.as_str(),
                        LAYOUTS,
                    ),
                    int_row("Зазор между окнами", "px", op!["windows", "gaps_inner"], w.gaps_inner as i64, 0, 64, 1),
                    int_row("Отступ от краёв экрана", "px", op!["windows", "gaps_outer"], w.gaps_outer as i64, 0, 64, 1),
                    slider_row(
                        "Доля мастер-области",
                        "",
                        op!["windows", "master_ratio"],
                        w.master_ratio as f64,
                        0.1,
                        0.9,
                        0.01,
                        2,
                    ),
                    int_row("Окон в мастер-области", "", op!["windows", "master_count"], w.master_count as i64, 1, 8, 1),
                    switch_row(
                        "Новое окно становится мастером",
                        "",
                        op!["windows", "new_is_master"],
                        w.new_is_master,
                    ),
                ],
            ),
            group(
                "Заголовок окна",
                vec![
                    choice_row("Двойной щелчок", "", op!["windows", "titlebar_double_click"], &dbl, TITLEBAR_ACTIONS),
                    choice_row("Средняя кнопка", "", op!["windows", "titlebar_middle_click"], &mid, TITLEBAR_ACTIONS),
                    choice_row(
                        "Колесо мыши",
                        "",
                        op!["windows", "titlebar_wheel"],
                        &w.titlebar_wheel,
                        &[("none", "Ничего"), ("opacity", "Прозрачность окна")],
                    ),
                ],
            ),
        ],
    )
}

// ─── Рабочие столы ──────────────────────────────────────────────────────────

pub fn workspaces() -> W {
    let c = store::config();
    let ws = &c.workspaces;
    let mut per: Vec<W> = Vec::new();
    for i in 0..ws.count {
        let name = ws.names.get(i as usize).cloned().unwrap_or_default();
        let layout = c.workspace_layout(i);
        let has_own = ws.layouts.contains_key(&(i + 1).to_string());
        let key = (i + 1).to_string();
        let key2 = key.clone();
        let idx = i as usize;
        let mut dd = Dropdown::new().width(190.0).item(DropdownItem::new("default", "Как по умолчанию"));
        for (v, l) in LAYOUTS {
            dd = dd.item(DropdownItem::new(*v, *l));
        }
        let dd = dd.selected(if has_own { layout.as_str() } else { "default" }).on_change(move |v: &str| {
            if v == "default" {
                unset(&op!["workspaces", "layouts", key2.as_str()]);
            } else {
                set(&op!["workspaces", "layouts", key2.as_str()], v.to_string());
            }
        });
        let _ = key;
        per.push(row(
            &format!("Стол {}", i + 1),
            "",
            Row::new()
                .gap(8.0)
                .child(TextField::with_text(name).placeholder(format!("{}", i + 1)).width(180.0).on_change(move |s| {
                    set_name(idx, s);
                }))
                .child(dd),
        ));
    }
    page(
        "Рабочие столы",
        "Виртуальные рабочие столы: сколько их, как называются, какая раскладка окон на каждом.",
        vec![
            group(
                "",
                vec![
                    row(
                        "Количество столов",
                        "",
                        SpinBox::new().range(1.0, 20.0).value(ws.count as f64).width(140.0).on_change(|v| {
                            set(&op!["workspaces", "count"], v.round() as i64);
                            state::bump();
                        }),
                    ),
                    switch_row("По кругу", "С последнего стола — на первый", op!["workspaces", "wrap"], ws.wrap),
                    switch_row(
                        "Туда и обратно",
                        "Повторное «стол N» на активном столе возвращает на прошлый",
                        op!["workspaces", "back_and_forth"],
                        ws.back_and_forth,
                    ),
                ],
            ),
            group("Имена и раскладки", per),
        ],
    )
}

/// Имена — массив строк; дописываем пустыми до нужного индекса.
fn set_name(idx: usize, name: &str) {
    let mut names = store::config().workspaces.names;
    while names.len() <= idx {
        names.push(String::new());
    }
    names[idx] = name.to_string();
    while names.last().is_some_and(|n| n.is_empty()) {
        names.pop();
    }
    let mut arr = Array::new();
    for n in names {
        arr.push(n);
    }
    set(&op!["workspaces", "names"], Value::Array(arr));
}

// ─── Правила окон ───────────────────────────────────────────────────────────

fn rule_title(r: &WindowRule) -> String {
    let mut parts = Vec::new();
    if let Some(a) = &r.app_id {
        parts.push(format!("app_id ~ {a}"));
    }
    if let Some(t) = &r.title {
        parts.push(format!("заголовок ~ {t}"));
    }
    if parts.is_empty() {
        "Все окна".into()
    } else {
        parts.join(", ")
    }
}

fn pair_field(p: P, value: Option<[i32; 2]>, a: &str, b: &str) -> impl Widget {
    let cur = use_signal(value.map(|v| (v[0].to_string(), v[1].to_string())).unwrap_or_default());
    let write = move |p: &P| {
        let (x, y) = cur.get_untracked();
        match (x.trim().parse::<i64>(), y.trim().parse::<i64>()) {
            (Ok(x), Ok(y)) => {
                let mut arr = Array::new();
                arr.push(x);
                arr.push(y);
                set(p, Value::Array(arr));
            }
            _ if x.trim().is_empty() && y.trim().is_empty() => unset(p),
            _ => {}
        }
    };
    let (p1, p2) = (p.clone(), p);
    let (v0, v1) = cur.get_untracked();
    Row::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(TextField::with_text(v0).placeholder(a).width(90.0).on_change(move |s| {
            cur.update(|c| c.0 = s.to_string());
            write(&p1);
        }))
        .child(Text::new("×").class("row-hint"))
        .child(TextField::with_text(v1).placeholder(b).width(90.0).on_change(move |s| {
            cur.update(|c| c.1 = s.to_string());
            write(&p2);
        }))
}

fn rule_card(i: usize, n: usize, r: &WindowRule) -> W {
    let tick = state::ctx().tick;
    let title = Reactive::new(move || -> Vec<W> {
        tick.get();
        let t = store::config().rules.get(i).map(rule_title).unwrap_or_default();
        vec![boxed(Text::new(t).class("group-title grow"))]
    });
    let header = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(title))
        .child(icon_button(icons::UP, move || {
            if i > 0 {
                store::edit(|d| store::doc_swap(d, &[store::Seg::K("rule")], i, i - 1));
                state::bump();
            }
        }))
        .child(icon_button(icons::DOWN, move || {
            if i + 1 < n {
                store::edit(|d| store::doc_swap(d, &[store::Seg::K("rule")], i, i + 1));
                state::bump();
            }
        }))
        .child(danger_icon_button(icons::DELETE, move || {
            unset(&op!["rule", i]);
            state::bump();
        }));
    let opacity_on = r.opacity.is_some();
    let rows = vec![
        row("app_id", "Регулярное выражение", opt_text(op!["rule", i, "app_id"], r.app_id.as_deref().unwrap_or(""), "^org\\.kde\\.kcalc$", 260.0)),
        row("Заголовок", "Регулярное выражение", opt_text(op!["rule", i, "title"], r.title.as_deref().unwrap_or(""), "Picture-in-Picture", 260.0)),
        row("Плавающее", "", tri(op!["rule", i, "floating"], r.floating)),
        row(
            "Рабочий стол",
            "0 — не менять",
            SpinBox::new().range(0.0, 20.0).value(r.workspace.unwrap_or(0) as f64).width(140.0).on_change(move |v| {
                let v = v.round() as i64;
                if v <= 0 {
                    unset(&op!["rule", i, "workspace"])
                } else {
                    set(&op!["rule", i, "workspace"], v)
                }
            }),
        ),
        row("Монитор", "Имя вывода", opt_text(op!["rule", i, "output"], r.output.as_deref().unwrap_or(""), "HDMI-A-1", 200.0)),
        row("Размер", "Ширина × высота", pair_field(op!["rule", i, "size"], r.size, "ширина", "высота")),
        row("Положение", "x × y", pair_field(op!["rule", i, "position"], r.position, "x", "y")),
        row("Мин. размер", "", pair_field(op!["rule", i, "min_size"], r.min_size, "ширина", "высота")),
        row("Макс. размер", "", pair_field(op!["rule", i, "max_size"], r.max_size, "ширина", "высота")),
        row("По центру", "", tri(op!["rule", i, "center"], r.center)),
        row("Развёрнуто", "", tri(op!["rule", i, "maximized"], r.maximized)),
        row("Во весь экран", "", tri(op!["rule", i, "fullscreen"], r.fullscreen)),
        row("На всех столах", "", tri(op!["rule", i, "sticky"], r.sticky)),
        row("Поверх других", "", tri(op!["rule", i, "always_on_top"], r.always_on_top)),
        row("Рамки композитора", "", tri(op!["rule", i, "decorations"], r.decorations)),
        row("Не давать фокус", "", tri(op!["rule", i, "no_focus"], r.no_focus)),
        row("Скрыть с панели задач", "", tri(op!["rule", i, "skip_taskbar"], r.skip_taskbar)),
        row("Касания как мышь", "Для программ без поддержки тача (Wine под X11 — по умолчанию)", tri(op!["rule", i, "touch_as_mouse"], r.touch_as_mouse)),
        row(
            "Непрозрачность",
            "",
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Slider::new().range(0.1, 1.0).step(0.01).value(r.opacity.unwrap_or(1.0)).show_value(2).width(200.0).on_change(
                    move |v| set(&op!["rule", i, "opacity"], round_to(v as f64, 2)),
                ))
                .child(Toggle::with_state(opacity_on).on_change(move |v| {
                    if v {
                        set(&op!["rule", i, "opacity"], 0.9);
                    } else {
                        unset(&op!["rule", i, "opacity"]);
                    }
                    state::bump();
                })),
        ),
    ];
    boxed(Column::new().gap(8.0).child(header).child(group("", rows)))
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if "\\.+*?()|[]{}^$".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn add_rule(app_id: Option<String>, title: Option<String>) {
    let mut t = Table::new();
    if let Some(a) = app_id {
        t.insert("app_id", toml_edit::value(a));
    }
    if let Some(ti) = title {
        t.insert("title", toml_edit::value(ti));
    }
    store::edit(|d| {
        store::doc_push_table(d, "rule", t.clone());
        true
    });
    state::bump();
}

pub fn rules() -> W {
    let c = store::config();
    let n = c.rules.len();
    let mut body: Vec<W> = vec![note(
        "Условия — регулярные выражения по app_id и заголовку; все заданные должны совпасть. Правила применяются сверху вниз.",
    )];
    // Открытые окна — из композитора; без него кнопки нет.
    let from_window = use_signal(false);
    let windows = sys::windows();
    let mut actions = Row::new().gap(8.0).child(primary_button("Новое правило", || add_rule(None, None)));
    if windows.is_some() {
        actions = actions.child(button("Взять из открытого окна…", move || from_window.set(!from_window.get_untracked())));
    }
    body.push(boxed(actions));
    if let Some(ws) = windows {
        body.push(boxed(Reactive::new(move || -> Vec<W> {
            if !from_window.get() {
                return vec![];
            }
            let mut col = Column::new().gap(4.0).class("group-card pad");
            if ws.is_empty() {
                col = col.child(Text::new("Открытых окон нет").class("row-hint"));
            }
            for w in &ws {
                let app = w.app_id.clone();
                col = col.child(
                    Button::new(format!("{} — {}", if w.app_id.is_empty() { "?" } else { &w.app_id }, w.title))
                        .class("btn list-btn")
                        .on_click(move || {
                            add_rule(Some(format!("^{}$", regex_escape(&app))), None);
                            from_window.set(false);
                        }),
                );
            }
            vec![boxed(col)]
        })));
    }
    for (i, r) in c.rules.iter().enumerate() {
        body.push(rule_card(i, n, r));
    }
    let _ = LayoutKind::Floating;
    page("Правила окон", "Особое поведение для выбранных приложений.", body)
}
