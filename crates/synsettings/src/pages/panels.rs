//! Панели и их апплеты.

use std::cell::Cell;

use synshell_common::config::{Applet, Config, Panel};
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

const STACK_VIEWS: &[(&str, &str)] = &[("grid", n_!("Сетка")), ("list", n_!("Список")), ("fan", n_!("Веер (док снизу)"))];
const STACK_OPEN: &[(&str, &str)] = &[("click", n_!("По клику")), ("hover", n_!("Наведением"))];

/// Типы апплетов: (тип, название, описание, опции).
fn applet_types() -> Vec<(&'static str, &'static str, &'static str, Vec<Opt>)> {
    use Opt::*;
    vec![
        ("launcher", n_!("Меню запуска"), n_!("Кнопка меню приложений"), vec![
            Text("icon", n_!("Значок"), "start-here"),
            Text("label", n_!("Подпись"), ""),
        ]),
        ("taskbar", n_!("Панель задач"), n_!("Кнопки открытых окон"), vec![
            Bool("labels", n_!("Подписи окон"), true),
            Bool("group", n_!("Группировать по приложению"), false),
            Bool("all_workspaces", n_!("Окна со всех столов"), false),
            Choice("title", n_!("Длинный заголовок"), "elide", &[
                ("elide", n_!("Обрезать в конце")),
                ("middle", n_!("Обрезать в середине")),
                ("wrap", n_!("Перенос на 2 строки мельче")),
            ]),
            Int("max_width", n_!("Ширина кнопки, px"), 220, 80, 600),
        ]),
        ("workspaces", n_!("Рабочие столы"), n_!("Переключатель столов"), vec![
            Bool("names", n_!("Номера/имена столов"), true),
            Bool("hide_empty", n_!("Скрывать пустые столы"), false),
            Bool("show_single", n_!("Показывать, когда стол один"), false),
        ]),
        ("app", n_!("Значок приложения"), n_!("Запуск и окна приложения (быстрый запуск)"), vec![
            Text("app", n_!("Приложение (id .desktop)"), "firefox"),
            Text("name", n_!("Подпись"), ""),
            Text("icon", n_!("Значок"), ""),
            Text("command", n_!("Своя команда"), ""),
            Bool("label", n_!("Подпись на панели"), false),
        ]),
        ("group", n_!("Раздел"), n_!("Группа значков во всплывающем окне"), vec![
            Text("name", n_!("Название"), n_!("Разработка")),
            Text("icon", n_!("Значок (пусто — сетка значков)"), ""),
            List("items", n_!("Приложения и пути"), n_!("org.kde.konsole, code, ~/Проекты")),
            Choice("view", n_!("Вид"), "grid", STACK_VIEWS),
            Choice("open", n_!("Открывать"), "click", STACK_OPEN),
        ]),
        ("folder", n_!("Папка"), n_!("Содержимое каталога во всплывающем окне"), vec![
            Text("path", n_!("Путь (xdg:DOWNLOAD, trash:)"), "xdg:DOWNLOAD"),
            Text("name", n_!("Название"), ""),
            Text("icon", n_!("Значок"), ""),
            Choice("view", n_!("Вид"), "grid", STACK_VIEWS),
            Choice("open", n_!("Открывать"), "click", STACK_OPEN),
        ]),
        ("spacer", n_!("Растяжка"), n_!("Заполняет свободное место"), vec![]),
        ("separator", n_!("Разделитель"), n_!("Тонкая линия"), vec![]),
        ("clock", n_!("Часы"), n_!("Время и календарь"), vec![
            Text("format", n_!("Формат времени"), "%H:%M"),
            Text("date_format", n_!("Формат даты"), "%a, %d %b"),
            Bool("seconds", n_!("Секунды"), false),
        ]),
        ("keyboard", n_!("Раскладка клавиатуры"), n_!("Индикатор и переключение"), vec![]),
        ("volume", n_!("Громкость"), n_!("Звук и микрофон"), vec![]),
        ("network", n_!("Сеть"), n_!("Состояние подключения"), vec![]),
        ("battery", n_!("Батарея"), n_!("Заряд и питание"), vec![]),
        ("tray", n_!("Системный лоток"), n_!("Значки фоновых программ"), vec![
            Bool("hide_passive", n_!("Прятать неактивные значки"), true),
            Int("icon_size", n_!("Размер значков, px"), 20, 12, 64),
            Int("max_visible", n_!("Видимых значков (0 — все)"), 0, 0, 64),
        ]),
        ("notifications", n_!("Уведомления"), n_!("Центр уведомлений"), vec![]),
        ("window-title", n_!("Заголовок окна"), n_!("Значок и заголовок активного окна: двойной щелчок — развернуть, перетащить — вытащить окно"), vec![
            Bool("only_maximized", n_!("Только у развёрнутого окна"), true),
            Bool("icon", n_!("Значок"), true),
            Choice("text", n_!("Текст"), "title", &[
                ("title", n_!("Заголовок окна")),
                ("app", n_!("Имя программы")),
                ("both", n_!("Программа — заголовок")),
            ]),
            Int("max_width", n_!("Наибольшая ширина, px"), 480, 40, 2000),
        ]),
        ("window-buttons", n_!("Кнопки окна"), n_!("Свернуть, развернуть, закрыть активное окно"), vec![
            Bool("only_maximized", n_!("Только у развёрнутого окна"), true),
            List("buttons", n_!("Кнопки"), "minimize, maximize, close"),
        ]),
        ("appmenu", n_!("Глобальное меню"), n_!("Строка меню активной программы (Qt/KDE; как в macOS)"), vec![
            Bool("only_maximized", n_!("Только у развёрнутого окна"), false),
            Bool("app_name", n_!("Имя программы первым пунктом"), false),
        ]),
        ("power", n_!("Питание"), n_!("Выход, сон, перезагрузка"), vec![]),
        ("show-desktop", n_!("Показать рабочий стол"), n_!("Свернуть все окна"), vec![]),
        ("layout", n_!("Раскладка окон"), n_!("Плавающая/плитка на столе"), vec![]),
        ("cpu", n_!("Процессор"), n_!("Загрузка ЦП"), vec![]),
        ("memory", n_!("Память"), n_!("Занятая память"), vec![]),
        ("button", n_!("Кнопка"), n_!("Своя кнопка с действием"), vec![
            Text("icon", n_!("Значок"), "utilities-terminal"),
            Text("label", n_!("Подпись"), ""),
            Text("action", n_!("Действие"), "spawn konsole"),
        ]),
        ("command", n_!("Вывод команды"), n_!("Строка из скрипта (как в waybar)"), vec![
            Text("command", n_!("Команда"), "date +%s"),
            Int("interval", n_!("Интервал, с"), 5, 1, 86400),
            Text("action", n_!("Действие по клику"), ""),
        ]),
    ]
}

fn type_label(kind: &str) -> String {
    applet_types()
        .into_iter()
        .find(|t| t.0 == kind)
        .map(|t| tl(t.1))
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
                    TextField::with_text(cur).placeholder(if key == "name" { tl(def) } else { def.to_string() }).width(240.0).on_change(move |s| {
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
                    &t!("Через запятую"),
                    TextField::with_text(cur).placeholder(tl(hint)).width(320.0).on_change(move |s| {
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
                    dd = dd.item(DropdownItem::new(v.to_string(), tl(l)));
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
        dd = dd.item(DropdownItem::new(kind, format!("{} — {}", tl(label), tl(desc))));
    }
    col = col.child(
        Row::new()
            .gap(8.0)
            .class("applet-add")
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(dd.selected("clock").on_change(move |v: &str| pick.set(v.to_string())))
            .child(primary_button(&t!("Добавить апплет"), move || {
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

fn choice_row(pi: usize, key: &'static str, label: impl AsRef<str>, hint: impl AsRef<str>, cur: &str, options: &[(&str, &str)]) -> W {
    let mut dd = Dropdown::new().width(220.0);
    for (v, l) in options {
        dd = dd.item(DropdownItem::new(v.to_string(), tl(l)));
    }
    row(label, hint, dd.selected(cur.to_string()).on_change(move |v: &str| dset(pi, key, v.to_string())))
}

/// Параметры дока (`[panel.dock]`).
fn dock_rows(pi: usize, d: &synshell_common::config::Dock) -> Vec<W> {
    vec![
        row(t!("Размер значков"), t!("Без увеличения, px"), {
            SpinBox::new().range(24.0, 128.0).value(d.icon_size as f64).width(140.0).on_change(move |v| dset(pi, "icon_size", v.round() as i64))
        }),
        row(t!("Увеличение"), t!("Во сколько раз растёт значок под курсором (1 — без увеличения)"), {
            Slider::new().range(1.0, 3.0).step(0.05).value(d.zoom).show_value(2).width(240.0).on_change(move |v| dset(pi, "zoom", round_to(v as f64, 2)))
        }),
        row(t!("Радиус увеличения"), t!("В значках"), {
            Slider::new().range(1.0, 5.0).step(0.1).value(d.zoom_range).show_value(1).width(240.0).on_change(move |v| dset(pi, "zoom_range", round_to(v as f64, 1)))
        }),
        choice_row(pi, "style", t!("Оформление"), t!("Полка — 3D-подложка с отражениями"), &d.style, &[
            ("glass", n_!("Стекло")),
            ("shelf", n_!("3D-полка")),
            ("flat", n_!("Плоское")),
            ("neon", n_!("Неон")),
            ("none", n_!("Только значки")),
        ]),
        choice_row(pi, "indicator", &t!("Индикатор окон"), "", &d.indicator, &[
            ("dot", n_!("Точка")),
            ("dots", n_!("Точка на окно")),
            ("line", n_!("Черта")),
            ("glow", n_!("Свечение")),
            ("none", n_!("Нет")),
        ]),
        choice_row(pi, "hover_effect", t!("При наведении"), t!("3D-наклон и вращение — через MSS rotate-x/rotate-y"), &d.hover_effect, &[
            ("lift", n_!("Приподнять")),
            ("tilt", n_!("3D-наклон")),
            ("spin", n_!("3D-вращение")),
            ("glow", n_!("Свечение")),
            ("none", n_!("Ничего")),
        ]),
        choice_row(pi, "hover_particles", &t!("Частицы при наведении"), "", &d.hover_particles, &[
            ("none", n_!("Нет")),
            ("sparkle", n_!("Искорки")),
            ("magic", n_!("Магия")),
            ("embers", n_!("Угольки")),
            ("bubbles", n_!("Пузыри")),
            ("hearts", n_!("Сердечки")),
            ("snow", n_!("Снег")),
            ("trail", n_!("След за курсором")),
        ]),
        choice_row(pi, "launch_animation", &t!("Анимация запуска"), "", &d.launch_animation, &[
            ("bounce", n_!("Прыжки")),
            ("pulse", n_!("Пульс")),
            ("spin", n_!("3D-вращение")),
            ("none", n_!("Нет")),
        ]),
        choice_row(pi, "launch_particles", &t!("Частицы при запуске"), "", &d.launch_particles, &[
            ("stars", n_!("Звёзды")),
            ("sparkle", n_!("Искорки")),
            ("confetti", n_!("Конфетти")),
            ("fireworks", n_!("Фейерверк")),
            ("magic", n_!("Магия")),
            ("poof", n_!("Облачко")),
            ("none", n_!("Нет")),
        ]),
        row(t!("Подписи"), t!("Имя приложения над значком при наведении"), Toggle::with_state(d.labels).on_change(move |v| dset(pi, "labels", v))),
        row(t!("Умное скрытие"), t!("Прятать, когда окно перекрывает док"), Toggle::with_state(d.intellihide).on_change(move |v| dset(pi, "intellihide", v))),
    ]
}

fn panel_card(pi: usize, p: &Panel, outputs: &[(String, String)]) -> W {
    let title = format!(
        "{} {} · {} · {}",
        if p.is_dock() { t!("Док") } else { t!("Панель") },
        pi + 1,
        match p.edge {
            synshell_common::config::Edge::Top => t!("сверху"),
            synshell_common::config::Edge::Bottom => t!("снизу"),
            synshell_common::config::Edge::Left => t!("слева"),
            synshell_common::config::Edge::Right => t!("справа"),
        },
        match p.output.as_str() {
            "*" => t!("все мониторы"),
            "primary" => t!("основной монитор"),
            o => o.to_string(),
        }
    );
    let edge = match p.edge {
        synshell_common::config::Edge::Top => "top",
        synshell_common::config::Edge::Bottom => "bottom",
        synshell_common::config::Edge::Left => "left",
        synshell_common::config::Edge::Right => "right",
    };
    let mut out_opts: Vec<(String, String)> = vec![
        ("*".into(), t!("Все мониторы").into()),
        ("primary".into(), t!("Основной монитор").into()),
    ];
    out_opts.extend(outputs.iter().cloned());
    if !out_opts.iter().any(|(v, _)| *v == p.output) {
        out_opts.push((p.output.clone(), p.output.clone()));
    }
    let has_opacity = p.opacity.is_some();
    let opacity = p.opacity.unwrap_or(store::config().appearance.panel_opacity);

    let mut settings = vec![
        row(t!("Вид"), t!("Док — значки с увеличением под курсором, как в macOS"), {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("panel", t!("Панель")), ("dock", t!("Док"))] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(if p.is_dock() { "dock" } else { "panel" }).on_change(move |v: &str| {
                pset(pi, "mode", v.to_string());
                state::bump();
            })
        }),
        row(&t!("Край экрана"), "", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("top", t!("Сверху")), ("bottom", t!("Снизу")), ("left", t!("Слева")), ("right", t!("Справа"))] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(edge).on_change(move |v: &str| {
                pset(pi, "edge", v.to_string());
                state::bump();
            })
        }),
        row(&t!("Монитор"), "", {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in out_opts {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.output.clone()).on_change(move |v: &str| {
                pset(pi, "output", v.to_string());
                state::bump();
            })
        }),
        row(t!("Толщина"), t!("Логические пиксели"), {
            SpinBox::new().range(20.0, 128.0).value(p.size as f64).width(140.0).on_change(move |v| pset(pi, "size", v.round() as i64))
        }),
        row(t!("Плавающая"), t!("Отступ от края и скругления (как в Plasma 6)"), Toggle::with_state(p.floating).on_change(move |v| pset(pi, "floating", v))),
        row(t!("Прилипать к краю"), t!("Плавающая панель встаёт к краю во всю длину, как адаптивная панель Plasma"), {
            let mut dd = Dropdown::new().width(240.0);
            for (v, l) in [("never", t!("Никогда")), ("maximized", t!("Когда окно развёрнуто")), ("touch", t!("Когда окно касается панели"))] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.defloat.clone()).on_change(move |v: &str| pset(pi, "defloat", v.to_string()))
        }),
        row(t!("Толщина у края"), t!("Прилипшей панели; 0 — как обычно"), {
            SpinBox::new().range(0.0, 128.0).value(p.defloated_size as f64).width(140.0).on_change(move |v| pset(pi, "defloated_size", v.round() as i64))
        }),
        row(t!("Длина"), t!("Доля края экрана"), {
            Slider::new().range(0.1, 1.0).step(0.01).value(p.length).show_value(2).width(240.0).on_change(move |v| pset(pi, "length", round_to(v as f64, 2)))
        }),
        row(t!("Выравнивание"), t!("При длине меньше 1"), {
            let mut dd = Dropdown::new().width(200.0);
            for (v, l) in [("start", t!("К началу")), ("center", t!("По центру")), ("end", t!("К концу"))] {
                dd = dd.item(DropdownItem::new(v, l));
            }
            dd.selected(p.align.clone()).on_change(move |v: &str| pset(pi, "align", v.to_string()))
        }),
        row(t!("Автоскрытие"), t!("Выезжает при подводе курсора к краю; на телефоне — по свайпу от края"), Toggle::with_state(p.autohide).on_change(move |v| pset(pi, "autohide", v))),
        row(t!("Задержка скрытия"), t!("Через сколько миллисекунд прячется после ухода курсора или пальца"), {
            SpinBox::new().range(100.0, 10000.0).step(100.0).value(p.autohide_delay as f64).width(140.0).on_change(move |v| pset(pi, "autohide_delay", v.round() as i64))
        }),
        row(t!("Резервировать место"), t!("Развёрнутые окна не заходят под панель"), Toggle::with_state(p.exclusive).on_change(move |v| pset(pi, "exclusive", v))),
        row(
            t!("Своя непрозрачность"),
            t!("Иначе — из «Внешнего вида»"),
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
        .child(Button::new(t!("Удалить панель")).class("btn danger small").icon(icons::DELETE).on_click(move || {
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
            .child(Text::new(t!("Апплеты")).class("subgroup-title"))
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
        body.push(note(&t!("Панелей нет. Добавьте хотя бы одну — иначе не будет меню и списка окон.")));
    }
    for (i, p) in c.panels.iter().enumerate() {
        body.push(panel_card(i, p, &outputs));
    }
    body.push(boxed(
        Row::new()
            .gap(8.0)
            .child(primary_button(&t!("Добавить панель"), || {
                ensure();
                let taken_top = store::config().panels.iter().any(|p| p.edge == synshell_common::config::Edge::Top);
                let mut p = Panel {
                    applets: vec![Applet::new("launcher"), Applet::new("spacer"), Applet::new("clock")],
                    ..Panel::default()
                };
                if !taken_top {
                    p.edge = synshell_common::config::Edge::Top;
                    p.size = 36;
                    p.floating = false;
                }
                store::edit(|d| {
                    store::doc_push_table(d, "panel", store::to_table(&p));
                    true
                });
                state::bump();
            }))
            .child(button(&t!("Добавить панель-заголовок"), || {
                // Верхняя панель как строка меню macOS/Plasma: у развёрнутого
                // окна она становится его заголовком.
                ensure();
                let a = |kind: &str| Applet::new(kind);
                let p = Panel {
                    edge: synshell_common::config::Edge::Top,
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
            .child(button(&t!("Добавить док"), || {
                ensure();
                store::edit(|d| {
                    store::doc_push_table(d, "panel", store::to_table(&Panel::dock_default()));
                    true
                });
                state::bump();
            }))
            .child(button(&t!("Вернуть панель по умолчанию"), || {
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
        t!("Панели и доки"),
        t!("Панели и доки на любом крае любого монитора. Значки, разделы и папки удобнее добавлять прямо на панели: правый клик → «Изменить»."),
        body,
    )
}
