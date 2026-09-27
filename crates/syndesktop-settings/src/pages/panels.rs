//! Панели и их апплеты.

use std::cell::Cell;

use syndesktop_common::config::{Applet, Config, Panel};
use syngui::prelude::*;
use toml_edit::{InlineTable, Value};

use crate::op;
use crate::state;
use crate::store::{self, doc_array_push, doc_swap, Seg};
use crate::sys;
use crate::ui::*;

/// Опция апплета для редактора.
enum Opt {
    Text(&'static str, &'static str, &'static str),
    Bool(&'static str, &'static str, bool),
    Int(&'static str, &'static str, i64, i64, i64),
    /// Ключ, подпись, значение по умолчанию, варианты (значение, подпись).
    Choice(&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)]),
    /// Список строк через запятую (ключ, подпись, подсказка).
    List(&'static str, &'static str, &'static str),
}

const STACK_VIEWS: &[(&str, &str)] = &[("grid", "Сетка"), ("list", "Список"), ("fan", "Веер (док снизу)")];
const STACK_OPEN: &[(&str, &str)] = &[("click", "По клику"), ("hover", "Наведением")];

/// Типы апплетов: (тип, название, описание, опции).
fn applet_types() -> Vec<(&'static str, &'static str, &'static str, Vec<Opt>)> {
    use Opt::*;
    vec![
        ("launcher", "Меню запуска", "Кнопка меню приложений", vec![
            Text("icon", "Значок", "start-here"),
            Text("label", "Подпись", ""),
        ]),
        ("taskbar", "Панель задач", "Кнопки открытых окон", vec![
            Bool("labels", "Подписи окон", true),
            Bool("group", "Группировать по приложению", false),
            Bool("all_workspaces", "Окна со всех столов", false),
            Choice("title", "Длинный заголовок", "elide", &[
                ("elide", "Обрезать в конце"),
                ("middle", "Обрезать в середине"),
                ("wrap", "Перенос на 2 строки мельче"),
            ]),
            Int("max_width", "Ширина кнопки, px", 220, 80, 600),
        ]),
        ("workspaces", "Рабочие столы", "Переключатель столов", vec![
            Bool("names", "Номера/имена столов", true),
            Bool("hide_empty", "Скрывать пустые столы", false),
            Bool("show_single", "Показывать, когда стол один", false),
        ]),
        ("app", "Значок приложения", "Запуск и окна приложения (быстрый запуск)", vec![
            Text("app", "Приложение (id .desktop)", "firefox"),
            Text("name", "Подпись", ""),
            Text("icon", "Значок", ""),
            Text("command", "Своя команда", ""),
            Bool("label", "Подпись на панели", false),
        ]),
        ("group", "Раздел", "Группа значков во всплывающем окне", vec![
            Text("name", "Название", "Разработка"),
            Text("icon", "Значок (пусто — сетка значков)", ""),
            List("items", "Приложения и пути", "org.kde.konsole, code, ~/Проекты"),
            Choice("view", "Вид", "grid", STACK_VIEWS),
            Choice("open", "Открывать", "click", STACK_OPEN),
        ]),
        ("folder", "Папка", "Содержимое каталога во всплывающем окне", vec![
            Text("path", "Путь (xdg:DOWNLOAD, trash:)", "xdg:DOWNLOAD"),
            Text("name", "Название", ""),
            Text("icon", "Значок", ""),
            Choice("view", "Вид", "grid", STACK_VIEWS),
            Choice("open", "Открывать", "click", STACK_OPEN),
        ]),
        ("spacer", "Растяжка", "Заполняет свободное место", vec![]),
        ("separator", "Разделитель", "Тонкая линия", vec![]),
        ("clock", "Часы", "Время и календарь", vec![
            Text("format", "Формат времени", "%H:%M"),
            Text("date_format", "Формат даты", "%a, %d %b"),
            Bool("seconds", "Секунды", false),
        ]),
        ("keyboard", "Раскладка клавиатуры", "Индикатор и переключение", vec![]),
        ("volume", "Громкость", "Звук и микрофон", vec![]),
        ("network", "Сеть", "Состояние подключения", vec![]),
        ("battery", "Батарея", "Заряд и питание", vec![]),
        ("tray", "Системный лоток", "Значки фоновых программ", vec![
            Bool("hide_passive", "Прятать неактивные значки", true),
            Int("icon_size", "Размер значков, px", 20, 12, 64),
            Int("max_visible", "Видимых значков (0 — все)", 0, 0, 64),
        ]),
        ("notifications", "Уведомления", "Центр уведомлений", vec![]),
        ("window-title", "Заголовок окна", "Значок и заголовок активного окна: двойной щелчок — развернуть, перетащить — вытащить окно", vec![
            Bool("only_maximized", "Только у развёрнутого окна", true),
            Bool("icon", "Значок", true),
            Choice("text", "Текст", "title", &[
                ("title", "Заголовок окна"),
                ("app", "Имя программы"),
                ("both", "Программа — заголовок"),
            ]),
            Int("max_width", "Наибольшая ширина, px", 480, 40, 2000),
        ]),
        ("window-buttons", "Кнопки окна", "Свернуть, развернуть, закрыть активное окно", vec![
            Bool("only_maximized", "Только у развёрнутого окна", true),
            List("buttons", "Кнопки", "minimize, maximize, close"),
        ]),
        ("appmenu", "Глобальное меню", "Строка меню активной программы (Qt/KDE; как в macOS)", vec![
            Bool("only_maximized", "Только у развёрнутого окна", false),
            Bool("app_name", "Имя программы первым пунктом", false),
        ]),
        ("power", "Питание", "Выход, сон, перезагрузка", vec![]),
        ("show-desktop", "Показать рабочий стол", "Свернуть все окна", vec![]),
        ("layout", "Раскладка окон", "Плавающая/плитка на столе", vec![]),
        ("cpu", "Процессор", "Загрузка ЦП", vec![]),
        ("memory", "Память", "Занятая память", vec![]),
        ("button", "Кнопка", "Своя кнопка с действием", vec![
            Text("icon", "Значок", "utilities-terminal"),
            Text("label", "Подпись", ""),
            Text("action", "Действие", "spawn konsole"),
        ]),
        ("command", "Вывод команды", "Строка из скрипта (как в waybar)", vec![
            Text("command", "Команда", "date +%s"),
            Int("interval", "Интервал, с", 5, 1, 86400),
            Text("action", "Действие по клику", ""),
        ]),
    ]
}

fn type_label(kind: &str) -> String {
    applet_types()
        .into_iter()
        .find(|t| t.0 == kind)
        .map(|t| t.1.to_string())
        .unwrap_or_else(|| kind.to_string())
}

thread_local! {
    /// Развёрнутый апплет (панель, индекс) — сигнал создаётся один раз.
    static EXPANDED: Cell<Option<RwSignal<Option<(usize, usize)>>>> = const { Cell::new(None) };
}

fn expanded() -> RwSignal<Option<(usize, usize)>> {
    EXPANDED.with(|c| match c.get() {
        Some(s) => s,
        None => {
            let s = use_signal(None);
            c.set(Some(s));
            s
        }
    })
}

/// Перед любой правкой панелей: панели по умолчанию — в файл.
fn ensure() {
    store::ensure_aot("panel", &Config::default().panels);
}

fn pset(pi: usize, key: &str, v: impl Into<toml_edit::Value>) {
    ensure();
    set(&op!["panel", pi, key], v);
}

fn applet_options(pi: usize, ai: usize, applet: &Applet) -> Vec<W> {
    let Some((_, _, _, opts)) = applet_types().into_iter().find(|t| t.0 == applet.kind) else {
        return vec![];
    };
    opts.into_iter()
        .map(|o| match o {
            Opt::Text(key, label, def) => {
                let cur = applet.str(key).unwrap_or("").to_string();
                let p = op!["panel", pi, "applets", ai, key];
                row(
                    label,
                    "",
                    TextField::with_text(cur).placeholder(def).width(240.0).on_change(move |s| {
                        ensure();
                        if s.is_empty() {
                            unset(&p)
                        } else {
                            set(&p, s.to_string())
                        }
                    }),
                )
            }
            Opt::Bool(key, label, def) => {
                let cur = applet.bool_or(key, def);
                let p = op!["panel", pi, "applets", ai, key];
                row(label, "", Toggle::with_state(cur).on_change(move |v| {
                    ensure();
                    set(&p, v)
                }))
            }
            Opt::Int(key, label, def, min, max) => {
                let cur = applet.int_or(key, def);
                let p = op!["panel", pi, "applets", ai, key];
                row(
                    label,
                    "",
                    SpinBox::new().range(min as f64, max as f64).value(cur as f64).width(140.0).on_change(move |v| {
                        ensure();
                        set(&p, v.round() as i64)
                    }),
                )
            }
            Opt::List(key, label, hint) => {
                let cur = applet.strings(key).join(", ");
                let p = op!["panel", pi, "applets", ai, key];
                row(
                    label,
                    "Через запятую",
                    TextField::with_text(cur).placeholder(hint).width(320.0).on_change(move |s| {
                        ensure();
                        let mut arr = toml_edit::Array::new();
                        for part in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
                            arr.push(part);
                        }
                        set(&p, toml_edit::Value::Array(arr))
                    }),
                )
            }
            Opt::Choice(key, label, def, options) => {
                let cur = applet.str_or(key, def).to_string();
                let p = op!["panel", pi, "applets", ai, key];
                let mut dd = Dropdown::new().width(240.0);
                for (v, l) in options {
                    dd = dd.item(DropdownItem::new(v.to_string(), l.to_string()));
                }
                row(
                    label,
                    "",
                    dd.selected(cur).on_change(move |v: &str| {
                        ensure();
                        set(&p, v.to_string())
                    }),
                )
            }
        })
        .collect()
}

fn applets_editor(pi: usize, panel: &Panel) -> W {
    let exp = expanded();
    let n = panel.applets.len();
    let mut col = Column::new().gap(4.0).class("applet-list");
    for (ai, applet) in panel.applets.iter().enumerate() {
        let applet = applet.clone();
        let label = type_label(&applet.kind);
        let has_opts = applet_types().iter().any(|t| t.0 == applet.kind && !t.3.is_empty());
        let head = Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("applet-row")
            .child(Text::new(format!("{}.", ai + 1)).class("applet-num"))
            .child(Column::new().class("grow").child(Text::new(label).class("row-label")).child(Text::new(applet.kind.clone()).class("row-hint")))
            .child(if has_opts {
                boxed(icon_button(icons::TUNE, move || {
                    exp.set(if exp.get_untracked() == Some((pi, ai)) { None } else { Some((pi, ai)) });
                }))
            } else {
                boxed(DecoratedBox::new().class("icon-btn-space"))
            })
            .child(icon_button(icons::UP, move || {
                if ai > 0 {
                    ensure();
                    store::edit(|d| doc_swap(d, &[Seg::K("panel"), Seg::I(pi), Seg::K("applets")], ai, ai - 1));
                    exp.set(None);
                    state::bump();
                }
            }))
            .child(icon_button(icons::DOWN, move || {
                if ai + 1 < n {
                    ensure();
                    store::edit(|d| doc_swap(d, &[Seg::K("panel"), Seg::I(pi), Seg::K("applets")], ai, ai + 1));
                    exp.set(None);
                    state::bump();
                }
            }))
            .child(danger_icon_button(icons::DELETE, move || {
                ensure();
                unset(&op!["panel", pi, "applets", ai]);
                exp.set(None);
                state::bump();
            }));
        col = col.child(head);
        let applet2 = applet.clone();
        col = col.child(Reactive::new(move || -> Vec<W> {
            if exp.get() == Some((pi, ai)) {
                vec![boxed(Column::new().class("applet-options").children(applet_options(pi, ai, &applet2)))]
            } else {
                vec![]
            }
        }));
    }
    // Добавить апплет.
    let pick = use_signal("clock".to_string());
    let mut dd = Dropdown::new().width(240.0).max_height(360.0);
    for (kind, label, desc, _) in applet_types() {
        dd = dd.item(DropdownItem::new(kind, format!("{label} — {desc}")));
    }
    col = col.child(
        Row::new()
            .gap(8.0)
            .class("applet-add")
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(dd.selected("clock").on_change(move |v: &str| pick.set(v.to_string())))
            .child(primary_button("Добавить апплет", move || {
                ensure();
                let mut t = InlineTable::new();
                t.insert("type", pick.get_untracked().into());
                store::edit(|d| doc_array_push(d, &[Seg::K("panel"), Seg::I(pi), Seg::K("applets")], Value::InlineTable(t)));
                state::bump();
            })),
    );
    boxed(col)
}

fn dset(pi: usize, key: &str, v: impl Into<toml_edit::Value>) {
    ensure();
    set(&op!["panel", pi, "dock", key], v);
}

fn choice_row(pi: usize, key: &'static str, label: &str, hint: &str, cur: &str, options: &[(&str, &str)]) -> W {
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in options {
        dd = dd.item(DropdownItem::new(v.to_string(), l.to_string()));
    }
    row(label, hint, dd.selected(cur.to_string()).on_change(move |v: &str| dset(pi, key, v.to_string())))
}

/// Параметры дока (`[panel.dock]`).
fn dock_rows(pi: usize, d: &syndesktop_common::config::Dock) -> Vec<W> {
    vec![
        row("Размер значков", "Без увеличения, px", {
            SpinBox::new().range(24.0, 128.0).value(d.icon_size as f64).width(140.0).on_change(move |v| dset(pi, "icon_size", v.round() as i64))
        }),
        row("Увеличение", "Во сколько раз растёт значок под курсором (1 — без увеличения)", {
            Slider::new().range(1.0, 3.0).step(0.05).value(d.zoom).show_value(2).width(240.0).on_change(move |v| dset(pi, "zoom", round_to(v as f64, 2)))
        }),
        row("Радиус увеличения", "В значках", {
            Slider::new().range(1.0, 5.0).step(0.1).value(d.zoom_range).show_value(1).width(240.0).on_change(move |v| dset(pi, "zoom_range", round_to(v as f64, 1)))
        }),
        choice_row(pi, "style", "Оформление", "Полка — 3D-подложка с отражениями", &d.style, &[
            ("glass", "Стекло"),
            ("shelf", "3D-полка"),
            ("flat", "Плоское"),
            ("neon", "Неон"),
            ("none", "Только значки"),
        ]),
        choice_row(pi, "indicator", "Индикатор окон", "", &d.indicator, &[
            ("dot", "Точка"),
            ("dots", "Точка на окно"),
            ("line", "Черта"),
            ("glow", "Свечение"),
            ("none", "Нет"),
        ]),
        choice_row(pi, "hover_effect", "При наведении", "3D-наклон и вращение — через MSS rotate-x/rotate-y", &d.hover_effect, &[
            ("lift", "Приподнять"),
            ("tilt", "3D-наклон"),
            ("spin", "3D-вращение"),
            ("glow", "Свечение"),
            ("none", "Ничего"),
        ]),
        choice_row(pi, "hover_particles", "Частицы при наведении", "", &d.hover_particles, &[
            ("none", "Нет"),
            ("sparkle", "Искорки"),
            ("magic", "Магия"),
            ("embers", "Угольки"),
            ("bubbles", "Пузыри"),
            ("hearts", "Сердечки"),
            ("snow", "Снег"),
            ("trail", "След за курсором"),
        ]),
        choice_row(pi, "launch_animation", "Анимация запуска", "", &d.launch_animation, &[
            ("bounce", "Прыжки"),
            ("pulse", "Пульс"),
            ("spin", "3D-вращение"),
            ("none", "Нет"),
        ]),
        choice_row(pi, "launch_particles", "Частицы при запуске", "", &d.launch_particles, &[
            ("stars", "Звёзды"),
            ("sparkle", "Искорки"),
            ("confetti", "Конфетти"),
            ("fireworks", "Фейерверк"),
            ("magic", "Магия"),
            ("poof", "Облачко"),
            ("none", "Нет"),
        ]),
        row("Подписи", "Имя приложения над значком при наведении", Toggle::with_state(d.labels).on_change(move |v| dset(pi, "labels", v))),
        row("Умное скрытие", "Прятать, когда окно перекрывает док", Toggle::with_state(d.intellihide).on_change(move |v| dset(pi, "intellihide", v))),
    ]
}

fn panel_card(pi: usize, p: &Panel, outputs: &[(String, String)]) -> W {
    let title = format!(
        "{} {} · {} · {}",
        if p.is_dock() { "Док" } else { "Панель" },
        pi + 1,
        match p.edge {
            syndesktop_common::config::Edge::Top => "сверху",
            syndesktop_common::config::Edge::Bottom => "снизу",
            syndesktop_common::config::Edge::Left => "слева",
            syndesktop_common::config::Edge::Right => "справа",
        },
        match p.output.as_str() {
            "*" => "все мониторы",
            "primary" => "основной монитор",
            o => o,
        }
    );
    let edge = match p.edge {
        syndesktop_common::config::Edge::Top => "top",
        syndesktop_common::config::Edge::Bottom => "bottom",
        syndesktop_common::config::Edge::Left => "left",
        syndesktop_common::config::Edge::Right => "right",
    };
    let mut out_opts: Vec<(String, String)> = vec![
        ("*".into(), "Все мониторы".into()),
        ("primary".into(), "Основной монитор".into()),
    ];
    out_opts.extend(outputs.iter().cloned());
    if !out_opts.iter().any(|(v, _)| *v == p.output) {
        out_opts.push((p.output.clone(), p.output.clone()));
    }
    let has_opacity = p.opacity.is_some();
    let opacity = p.opacity.unwrap_or(store::config().appearance.panel_opacity);

    let mut settings = vec![
        row("Вид", "Док — значки с увеличением под курсором, как в macOS", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("panel", "Панель"), ("dock", "Док")] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(if p.is_dock() { "dock" } else { "panel" }).on_change(move |v: &str| {
                pset(pi, "mode", v.to_string());
                state::bump();
            })
        }),
        row("Край экрана", "", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("top", "Сверху"), ("bottom", "Снизу"), ("left", "Слева"), ("right", "Справа")] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(edge).on_change(move |v: &str| {
                pset(pi, "edge", v.to_string());
                state::bump();
            })
        }),
        row("Монитор", "", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in out_opts {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.output.clone()).on_change(move |v: &str| {
                pset(pi, "output", v.to_string());
                state::bump();
            })
        }),
        row("Толщина", "Логические пиксели", {
            SpinBox::new().range(20.0, 128.0).value(p.size as f64).width(140.0).on_change(move |v| pset(pi, "size", v.round() as i64))
        }),
        row("Плавающая", "Отступ от края и скругления (как в Plasma 6)", Toggle::with_state(p.floating).on_change(move |v| pset(pi, "floating", v))),
        row("Прилипать к краю", "Плавающая панель встаёт к краю во всю длину, как адаптивная панель Plasma", {
            let mut dd = Dropdown::new().width(240.0);
            for (v, l) in [("never", "Никогда"), ("maximized", "Когда окно развёрнуто"), ("touch", "Когда окно касается панели")] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.defloat.clone()).on_change(move |v: &str| pset(pi, "defloat", v.to_string()))
        }),
        row("Длина", "Доля края экрана", {
            Slider::new().range(0.1, 1.0).step(0.01).value(p.length).show_value(2).width(240.0).on_change(move |v| pset(pi, "length", round_to(v as f64, 2)))
        }),
        row("Выравнивание", "При длине меньше 1", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("start", "К началу"), ("center", "По центру"), ("end", "К концу")] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.align.clone()).on_change(move |v: &str| pset(pi, "align", v.to_string()))
        }),
        row("Автоскрытие", "Выезжает при подводе курсора к краю", Toggle::with_state(p.autohide).on_change(move |v| pset(pi, "autohide", v))),
        row("Резервировать место", "Развёрнутые окна не заходят под панель", Toggle::with_state(p.exclusive).on_change(move |v| pset(pi, "exclusive", v))),
        row(
            "Своя непрозрачность",
            "Иначе — из «Внешнего вида»",
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Slider::new().range(0.2, 1.0).step(0.01).value(opacity).show_value(2).width(200.0).on_change(move |v| {
                    pset(pi, "opacity", round_to(v as f64, 2))
                }))
                .child(Toggle::with_state(has_opacity).on_change(move |v| {
                    ensure();
                    if v {
                        set(&op!["panel", pi, "opacity"], round_to(opacity as f64, 2));
                    } else {
                        unset(&op!["panel", pi, "opacity"]);
                    }
                })),
        ),
    ];
    if p.is_dock() {
        settings.extend(dock_rows(pi, &p.dock));
    }

    let header = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title).class("group-title grow"))
        .child(Button::new("Удалить панель").class("btn danger small").icon(icons::DELETE).on_click(move || {
            ensure();
            unset(&op!["panel", pi]);
            expanded().set(None);
            state::bump();
        }));

    boxed(
        Column::new()
            .gap(8.0)
            .child(header)
            .child(group("", settings))
            .child(Text::new("Апплеты").class("subgroup-title"))
            .child(DecoratedBox::new().class("group-card pad").child(applets_editor(pi, p))),
    )
}

pub fn panels() -> W {
    let c = store::config();
    let outputs: Vec<(String, String)> = sys::outputs()
        .unwrap_or_default()
        .into_iter()
        .map(|o| {
            let label = if o.description.is_empty() { o.name.clone() } else { format!("{} — {}", o.name, o.description) };
            (o.name, label)
        })
        .collect();
    let mut body: Vec<W> = Vec::new();
    if c.panels.is_empty() {
        body.push(note("Панелей нет. Добавьте хотя бы одну — иначе не будет меню и списка окон."));
    }
    for (i, p) in c.panels.iter().enumerate() {
        body.push(panel_card(i, p, &outputs));
    }
    body.push(boxed(
        Row::new()
            .gap(8.0)
            .child(primary_button("Добавить панель", || {
                ensure();
                let taken_top = store::config().panels.iter().any(|p| p.edge == syndesktop_common::config::Edge::Top);
                let mut p = Panel {
                    applets: vec![Applet::new("launcher"), Applet::new("spacer"), Applet::new("clock")],
                    ..Panel::default()
                };
                if !taken_top {
                    p.edge = syndesktop_common::config::Edge::Top;
                    p.size = 36;
                    p.floating = false;
                }
                store::edit(|d| {
                    store::doc_push_table(d, "panel", store::to_table(&p));
                    true
                });
                state::bump();
            }))
            .child(button("Добавить панель-заголовок", || {
                // Верхняя панель как строка меню macOS/Plasma: у развёрнутого
                // окна она становится его заголовком.
                ensure();
                let a = |kind: &str| Applet::new(kind);
                let p = Panel {
                    edge: syndesktop_common::config::Edge::Top,
                    size: 34,
                    floating: false,
                    applets: vec![a("launcher"), a("window-title"), a("appmenu"), a("spacer"), a("tray"), a("clock"), a("window-buttons")],
                    ..Panel::default()
                };
                store::edit(|d| {
                    store::doc_push_table(d, "panel", store::to_table(&p));
                    true
                });
                set(&op!["windows", "borderless_maximized"], true);
                state::bump();
            }))
            .child(button("Добавить док", || {
                ensure();
                store::edit(|d| {
                    store::doc_push_table(d, "panel", store::to_table(&Panel::dock_default()));
                    true
                });
                state::bump();
            }))
            .child(button("Вернуть панель по умолчанию", || {
                store::edit(|d| {
                    d.remove("panel");
                    for p in Config::default().panels {
                        store::doc_push_table(d, "panel", store::to_table(&p));
                    }
                    true
                });
                expanded().set(None);
                state::bump();
            })),
    ));
    page(
        "Панели и доки",
        "Панели и доки на любом крае любого монитора. Значки, разделы и папки удобнее добавлять прямо на панели: правый клик → «Изменить».",
        body,
    )
}
