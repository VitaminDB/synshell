//! Внешний вид, оформление окон, анимации.

use synshell_common::config::Rgba;
use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store;
use crate::sys;
use crate::ui::*;

const ACCENTS: &[(&str, &str)] = &[
    ("#3d8bfd", n_!("Синий")),
    ("#7c5cff", n_!("Фиолетовый")),
    ("#d946ef", n_!("Пурпурный")),
    ("#ef4466", n_!("Красный")),
    ("#f97316", n_!("Оранжевый")),
    ("#eab308", n_!("Жёлтый")),
    ("#22c55e", n_!("Зелёный")),
    ("#14b8a6", n_!("Бирюзовый")),
    ("#64748b", n_!("Графит")),
];

fn to_color(hex: &str) -> Color {
    Rgba::parse(hex)
        .map(|c| Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0))
        .unwrap_or(Color::transparent())
}

/// Мини-превью рабочего стола в текущей палитре: обновляется при каждой
/// правке (подписан на `tick`).
fn preview() -> W {
    let tick = state::ctx().tick;
    boxed(DecoratedBox::new().class("preview-frame").child(move || {
        tick.get();
        let c = store::config();
        let p = c.appearance.palette();
        let r = c.appearance.corner_radius;
        Stack::new()
            .fit(StackFit::Expand)
            .child(DecoratedBox::new().class("preview-desktop desk-bg"))
            .child(
                Column::new()
                    .class("preview-layer")
                    .child(
                        Column::new()
                            .class("preview-window")
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .class("preview-titlebar")
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Text::new(t!("Окно")).class("preview-title").style("color", to_color(&p.fg.hex())))
                                    .child(DecoratedBox::new().class("grow"))
                                    .child(DecoratedBox::new().class("preview-dot").style("background", to_color(&p.muted.hex())))
                                    .child(DecoratedBox::new().class("preview-dot").style("background", to_color(&p.danger.hex()))),
                            )
                            .child(
                                Column::new()
                                    .gap(6.0)
                                    .class("preview-content")
                                    .child(DecoratedBox::new().class("preview-line").style("background", to_color(&p.muted.with_alpha(0.5).hex())))
                                    .child(DecoratedBox::new().class("preview-line short").style("background", to_color(&p.muted.with_alpha(0.35).hex())))
                                    .child(
                                        DecoratedBox::new()
                                            .class("preview-button")
                                            .style("background", to_color(&p.accent.hex()))
                                            .style("border-radius", (r * 0.6).max(2.0)),
                                    ),
                            )
                            .style("background", to_color(&p.surface.hex()))
                            .style("border-radius", r),
                    )
                    .child(DecoratedBox::new().class("grow"))
                    .child(
                        Row::new()
                            .gap(6.0)
                            .class("preview-panel")
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class("preview-dot big").style("background", to_color(&p.accent.hex())))
                            .child(DecoratedBox::new().class("preview-task").style("background", to_color(&p.fg.with_alpha(0.14).hex())))
                            .child(DecoratedBox::new().class("preview-task").style("background", to_color(&p.fg.with_alpha(0.08).hex())))
                            .child(DecoratedBox::new().class("grow"))
                            .child(Text::new("12:30").class("preview-clock").style("color", to_color(&p.fg.hex())))
                            .style("background", to_color(&p.bg.with_alpha(c.appearance.panel_opacity).hex()))
                            .style("border-radius", r),
                    ),
            )
    }))
}

pub fn appearance() -> W {
    let c = store::config();
    let a = &c.appearance;
    let scheme_idx = if a.is_dark() { 0 } else { 1 };

    // Образцы акцента: клик — записать цвет и перестроить страницу, чтобы
    // поле «свой цвет» показало новое значение.
    let mut swatches = Row::new().gap(8.0).class("swatches");
    for (hex, name) in ACCENTS {
        let selected = a.accent.eq_ignore_ascii_case(hex);
        let hex_s = hex.to_string();
        swatches = swatches.child(
            Button::new("")
                .class(if selected { "accent-swatch selected" } else { "accent-swatch" })
                .on_click(move || {
                    set(&op!["appearance", "accent"], hex_s.clone());
                    state::bump();
                })
                .style("background", to_color(hex)),
        );
        let _ = name;
    }

    let icon_themes: Vec<(String, String)> = sys::icon_themes();
    let cursor_themes: Vec<(String, String)> = sys::cursor_themes();
    let mut icon_opts = icon_themes.clone();
    if !icon_opts.iter().any(|(id, _)| *id == a.icon_theme) {
        icon_opts.insert(0, (a.icon_theme.clone(), t!("{icon_theme} (не найдена)", icon_theme = a.icon_theme)));
    }
    let mut cursor_opts = cursor_themes.clone();
    if !cursor_opts.iter().any(|(id, _)| *id == a.cursor_theme) {
        cursor_opts.insert(0, (a.cursor_theme.clone(), t!("{cursor_theme} (не найдена)", cursor_theme = a.cursor_theme)));
    }
    let fonts = sys::font_families();

    let colors_rows: Vec<W> = [
        ("bg", t!("Фон")),
        ("surface", t!("Поверхности")),
        ("surface_alt", t!("Поверхности (вторичные)")),
        ("fg", t!("Текст")),
        ("muted", t!("Приглушённый текст")),
        ("border", t!("Границы")),
        ("accent_fg", t!("Текст на акценте")),
        ("danger", t!("Опасность")),
        ("success", t!("Успех")),
        ("warning", t!("Предупреждение")),
    ]
    .iter()
    .map(|(key, label)| {
        let cur = a.colors.get(*key).cloned().unwrap_or_default();
        let base = synshell_common::config::Appearance { colors: Default::default(), ..a.clone() }.palette();
        let def = match *key {
            "bg" => base.bg,
            "surface" => base.surface,
            "surface_alt" => base.surface_alt,
            "fg" => base.fg,
            "muted" => base.muted,
            "border" => base.border,
            "accent_fg" => base.accent_fg,
            "danger" => base.danger,
            "success" => base.success,
            _ => base.warning,
        };
        row(label, "", color_field(op!["appearance", "colors", *key], &cur, &def.hex()))
    })
    .collect();

    page(
        t!("Внешний вид"),
        t!("Цвета, шрифты и темы — общие для панелей, меню, рамок окон и этого окна."),
        vec![
            preview(),
            group(
                &t!("Цвета"),
                vec![
                    row(
                        &t!("Цветовая схема"),
                        "",
                        SegmentedButton::new(vec![t!("Тёмная"), t!("Светлая")]).selected(scheme_idx).on_change(|i| {
                            set(&op!["appearance", "color_scheme"], if i == 0 { "dark" } else { "light" });
                        }),
                    ),
                    row_wide(t!("Акцентный цвет"), t!("Кнопки, выделение, активные элементы"), swatches),
                    row(t!("Свой акцент"), t!("Пусто — акцент темы"), color_field(op!["appearance", "accent"], &a.accent, n_!("из темы"))),
                ],
            ),
            group(
                &t!("Шрифты"),
                vec![
                    row(
                        t!("Шрифт интерфейса"),
                        t!("Пусто — системный (fontconfig)"),
                        {
                            let p = op!["appearance", "font"];
                            let p2 = p.clone();
                            Autocomplete::new(fonts)
                                .text(a.font.clone())
                                .placeholder(t!("Системный"))
                                .width(260.0)
                                .on_change(move |s| set(&p, s.to_string()))
                                .on_select(move |s| set(&p2, s.to_string()))
                        },
                    ),
                    row(
                        &t!("Размер шрифта"),
                        "",
                        float_spin(op!["appearance", "font_size"], a.font_size as f64, 8.0, 24.0, 0.5, 1),
                    ),
                ],
            ),
            group(
                &t!("Форма и масштаб"),
                vec![
                    slider_row(
                        t!("Скругление углов"),
                        t!("Панели, меню и окна"),
                        op!["appearance", "corner_radius"],
                        a.corner_radius as f64,
                        0.0,
                        24.0,
                        1.0,
                        0,
                    ),
                    slider_row(
                        &t!("Непрозрачность панелей"),
                        "",
                        op!["appearance", "panel_opacity"],
                        a.panel_opacity as f64,
                        0.3,
                        1.0,
                        0.01,
                        2,
                    ),
                    slider_row(
                        t!("Масштаб оболочки"),
                        t!("Дополнительно к масштабу монитора"),
                        op!["appearance", "ui_scale"],
                        a.ui_scale as f64,
                        0.75,
                        2.0,
                        0.05,
                        2,
                    ),
                ],
            ),
            group(
                &t!("Темы"),
                vec![
                    row(&t!("Значки"), "", choice_owned(op!["appearance", "icon_theme"], &a.icon_theme, icon_opts, 260.0)),
                    row(
                        &t!("Курсор"),
                        "",
                        choice_owned(op!["appearance", "cursor_theme"], &a.cursor_theme, cursor_opts, 260.0),
                    ),
                    row(
                        &t!("Размер курсора"),
                        "",
                        {
                            let mut dd = Dropdown::new().width(140.0);
                            let mut sizes = vec![16u32, 24, 32, 36, 48, 64, 72, 96];
                            if !sizes.contains(&a.cursor_size) {
                                sizes.push(a.cursor_size);
                                sizes.sort();
                            }
                            for s in sizes {
                                dd = dd.item(DropdownItem::new(s.to_string(), format!("{s} px")));
                            }
                            dd.selected(a.cursor_size.to_string()).on_change(|v: &str| {
                                if let Ok(n) = v.parse::<i64>() {
                                    set(&op!["appearance", "cursor_size"], n);
                                }
                            })
                        },
                    ),
                    choice_row(
                        t!("Скрывать курсор"),
                        t!("Для телефонов и планшетов: при касании экрана курсор прячется до движения мыши или не показывается вовсе"),
                        op!["appearance", "cursor_hide"],
                        &a.cursor_hide,
                        &[("never", n_!("Никогда")), ("touch", n_!("При касании экрана")), ("always", n_!("Всегда"))],
                    ),
                ],
            ),
            group(
                &t!("Программы"),
                vec![switch_row(
                    t!("Цвета программ по теме"),
                    t!("GTK (Breeze, libadwaita), Qt/KDE и GIMP; открытые окна GTK — после перезапуска"),
                    op!["appearance", "app_colors"],
                    a.app_colors,
                )],
            ),
            group(&t!("Палитра"), colors_rows),
            note(&t!("Пустое поле — цвет схемы. Тонкая настройка стилей — в ~/.config/synshell/theme.mss (MSS поверх встроенной темы).")),
        ],
    )
}

// ─── Оформление окон ────────────────────────────────────────────────────────

const BUTTON_NAMES: &[(&str, &str)] = &[
    ("icon", n_!("Значок (меню окна)")),
    ("sticky", n_!("На всех столах")),
    ("above", n_!("Поверх других")),
    ("minimize", n_!("Свернуть")),
    ("maximize", n_!("Развернуть")),
    ("close", n_!("Закрыть")),
];

/// Редактор раскладки кнопок заголовка: две колонки (слева/справа) с
/// перестановкой — вместо ручного ввода «icon:minimize,maximize,close».
fn buttons_editor(layout: &str) -> W {
    let (left, right) = layout.split_once(':').unwrap_or(("", layout));
    let parse = |s: &str| -> Vec<String> {
        s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    };
    let left = parse(left);
    let right = parse(right);
    let write = |l: &[String], r: &[String]| {
        set(&op!["decorations", "buttons"], format!("{}:{}", l.join(","), r.join(",")));
        state::bump();
    };
    let side = |title: &str, items: Vec<String>, is_left: bool, left: Vec<String>, right: Vec<String>| -> W {
        let mut col = Column::new().gap(6.0).class("btn-side").child(Text::new(title).class("row-hint"));
        for (i, name) in items.iter().enumerate() {
            let label = BUTTON_NAMES.iter().find(|(n, _)| n == name).map(|(_, l)| tl(l)).unwrap_or_else(|| name.clone());
            let (l1, r1, l2, r2, l3, r3) =
                (left.clone(), right.clone(), left.clone(), right.clone(), left.clone(), right.clone());
            col = col.child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .class("chip-row")
                    .child(Text::new(label).class("grow"))
                    .child(icon_button(icons::UP, move || {
                        let (mut l, mut r) = (l1.clone(), r1.clone());
                        let v = if is_left { &mut l } else { &mut r };
                        if i > 0 {
                            v.swap(i, i - 1);
                            write(&l, &r);
                        }
                    }))
                    .child(icon_button(if is_left { "\u{e5cc}" } else { "\u{e5cb}" }, move || {
                        // Перенести на другую сторону.
                        let (mut l, mut r) = (l2.clone(), r2.clone());
                        if is_left {
                            let x = l.remove(i);
                            r.insert(0, x);
                        } else {
                            let x = r.remove(i);
                            l.push(x);
                        }
                        write(&l, &r);
                    }))
                    .child(danger_icon_button(icons::CLOSE, move || {
                        let (mut l, mut r) = (l3.clone(), r3.clone());
                        if is_left {
                            l.remove(i);
                        } else {
                            r.remove(i);
                        }
                        write(&l, &r);
                    })),
            );
        }
        boxed(col)
    };
    let used: Vec<&String> = left.iter().chain(right.iter()).collect();
    let mut add = Row::new().gap(6.0);
    for (name, label) in BUTTON_NAMES {
        if used.iter().any(|u| u.as_str() == *name) {
            continue;
        }
        let (l, r) = (left.clone(), right.clone());
        let n = name.to_string();
        add = add.child(Button::new(format!("+ {}", tl(label))).class("btn small").on_click(move || {
            let mut r = r.clone();
            r.insert(0, n.clone());
            write(&l, &r);
        }));
    }
    boxed(
        Column::new()
            .gap(10.0)
            .child(
                Row::new()
                    .gap(16.0)
                    .child(side(&t!("Слева"), left.clone(), true, left.clone(), right.clone()))
                    .child(side(&t!("Справа"), right.clone(), false, left.clone(), right.clone())),
            )
            .child(add),
    )
}

pub fn decorations() -> W {
    let c = store::config();
    let d = &c.decorations;
    let color_opts: &[(&str, &str)] =
        &[("theme", n_!("Из темы")), ("accent", n_!("Акцент")), ("surface", n_!("Поверхность")), ("bg", n_!("Фон")), ("surface_alt", n_!("Вторичная поверхность"))];
    let color_row = |label: &str, key: &'static str, cur: &str| -> W {
        let is_named = color_opts.iter().any(|(v, _)| *v == cur);
        let named_val = if is_named { cur } else { "custom" };
        let mut opts: Vec<(String, String)> = color_opts.iter().map(|(a, b)| (a.to_string(), tl(b))).collect();
        opts.push(("custom".into(), t!("Свой цвет").into()));
        let mut dd = Dropdown::new().width(190.0);
        for (v, l) in &opts {
            dd = dd.item(DropdownItem::new(v.clone(), l.clone()));
        }
        let dd = dd.selected(named_val).on_change(move |v: &str| {
            if v == "custom" {
                set(&op!["decorations", key], "#2a2e37");
            } else {
                set(&op!["decorations", key], v.to_string());
            }
            state::bump();
        });
        let mut r = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(dd);
        if !is_named {
            r = r.child(color_field(op!["decorations", key], cur, "#rrggbb"));
        }
        row(label, "", r)
    };
    page(
        t!("Оформление окон"),
        t!("Серверные рамки и заголовки, которые рисует композитор (если в «Поведении окон» выбраны рамки композитора)."),
        vec![
            group(
                &t!("Заголовок"),
                vec![
                    int_row(&t!("Высота заголовка"), "", op!["decorations", "title_height"], d.title_height as i64, 16, 64, 1),
                    row(
                        &t!("Размер шрифта"),
                        "",
                        float_spin(op!["decorations", "font_size"], d.font_size as f64, 8.0, 24.0, 0.5, 1),
                    ),
                    choice_row(
                        &t!("Выравнивание заголовка"),
                        "",
                        op!["decorations", "title_align"],
                        &d.title_align,
                        &[("left", n_!("Слева")), ("center", n_!("По центру"))],
                    ),
                    color_row(&t!("Цвет активного окна"), "active_color", &d.active_color),
                    color_row(&t!("Цвет неактивного окна"), "inactive_color", &d.inactive_color),
                ],
            ),
            group(&t!("Кнопки заголовка"), vec![row_wide(&t!("Расположение"), "", buttons_editor(&d.buttons))]),
            group(
                &t!("Форма и тень"),
                vec![
                    slider_row(
                        &t!("Скругление углов окна"),
                        "",
                        op!["decorations", "corner_radius"],
                        d.corner_radius as f64,
                        0.0,
                        24.0,
                        1.0,
                        0,
                    ),
                    int_row(
                        t!("Зона изменения размера"),
                        t!("Невидимая полоса вокруг окна, px"),
                        op!["decorations", "resize_border"],
                        d.resize_border as i64,
                        0,
                        32,
                        1,
                    ),
                    switch_row(&t!("Тень"), "", op!["decorations", "shadow"], d.shadow),
                    int_row(&t!("Размер тени"), "", op!["decorations", "shadow_size"], d.shadow_size as i64, 0, 96, 2),
                    slider_row(
                        &t!("Плотность тени"),
                        "",
                        op!["decorations", "shadow_opacity"],
                        d.shadow_opacity as f64,
                        0.0,
                        1.0,
                        0.01,
                        2,
                    ),
                ],
            ),
        ],
    )
}

// ─── Анимации ───────────────────────────────────────────────────────────────

pub fn animations() -> W {
    let c = store::config();
    let a = &c.animations;
    let win: &[(&str, &str)] = &[("zoom", n_!("Масштаб")), ("fade", n_!("Растворение")), ("slide", n_!("Выезд")), ("none", n_!("Нет"))];
    page(
        t!("Анимации"),
        t!("Эффекты открытия окон, переключения столов и сворачивания."),
        vec![
            group(
                "",
                vec![
                    switch_row(&t!("Анимации включены"), "", op!["animations", "enabled"], a.enabled),
                    slider_row(
                        t!("Длительность"),
                        t!("0.5 — вдвое быстрее, 2 — вдвое медленнее"),
                        op!["animations", "speed"],
                        a.speed as f64,
                        0.25,
                        3.0,
                        0.05,
                        2,
                    ),
                ],
            ),
            group(
                &t!("Эффекты"),
                vec![
                    choice_row(&t!("Открытие окна"), "", op!["animations", "window_open"], &a.window_open, win),
                    choice_row(&t!("Закрытие окна"), "", op!["animations", "window_close"], &a.window_close, win),
                    choice_row(
                        &t!("Сворачивание"),
                        "",
                        op!["animations", "minimize"],
                        &a.minimize,
                        &[("zoom", n_!("К панели")), ("fade", n_!("Растворение")), ("none", n_!("Нет"))],
                    ),
                    choice_row(
                        &t!("Переключение столов"),
                        "",
                        op!["animations", "workspace_switch"],
                        &a.workspace_switch,
                        &[
                            ("slide", n_!("Сдвиг по горизонтали")),
                            ("slide-vertical", n_!("Сдвиг по вертикали")),
                            ("fade", n_!("Растворение")),
                            ("none", n_!("Нет")),
                        ],
                    ),
                    switch_row(
                        t!("Плавная смена раскладки"),
                        t!("Окна перетекают на новые места при плиточной раскладке"),
                        op!["animations", "layout_changes"],
                        a.layout_changes,
                    ),
                ],
            ),
            group(
                &t!("Оболочка"),
                vec![
                    switch_row(
                        t!("Анимации оболочки"),
                        t!("Всплывающие окна вырастают из панели, меню запуска перетекает между разделами, уведомления въезжают"),
                        op!["animations", "shell"],
                        a.shell,
                    ),
                    switch_row(
                        t!("Плавная смена темы"),
                        t!("Цвета перетекают, обои растворяются при смене оформления"),
                        op!["animations", "theme_change"],
                        a.theme_change,
                    ),
                    switch_row(t!("Меньше движения"), t!("Короче переходы, без частиц и волн"), op!["animations", "reduce_motion"], a.reduce_motion),
                ],
            ),
            group(
                &t!("Группы оболочки"),
                vec![
                    switch_row(t!("Домашний экран"), t!("Листание страниц, появление и уход (телефон)"), op!["animations", "home"], a.home),
                    switch_row(t!("«Пуск»"), t!("Появление, перетекание в «Все приложения», меню значка"), op!["animations", "menu"], a.menu),
                    switch_row(t!("Шторка"), t!("Выезд сверху, затемнение (телефон)"), op!["animations", "shade"], a.shade),
                    switch_row(t!("Док и панели"), t!("Увеличение значков, прыжки при запуске"), op!["animations", "dock"], a.dock),
                    switch_row(t!("Приложения и режимы окон"), t!("Листание приложений, «Недавние», смена режима (телефон)"), op!["animations", "pages"], a.pages),
                ],
            ),
            group(
                &t!("Эффекты"),
                vec![
                    switch_row(t!("Частицы"), t!("Искры и конфетти дока"), op!["animations", "particles"], a.particles),
                    switch_row(t!("Размытие"), t!("Под меню и всплывающими окнами; на CPU-композиторе телефона дорого"), op!["animations", "blur"], a.blur),
                    switch_row(&t!("Волна от нажатия"), "", op!["animations", "ripple"], a.ripple),
                ],
            ),
            group(&t!("Наборы"), vec![row_inline(&t!("Быстро выставить"), "", presets())]),
        ],
    )
}

/// Наборы: максимум, сбалансировано (телефон), экономия, выключено.
fn presets() -> impl Widget {
    fn apply(on: bool, speed: f64, particles: bool, blur: bool, reduce: bool) {
        for k in ["enabled", "shell", "theme_change", "home", "menu", "shade", "dock", "pages", "ripple"] {
            set(&op!["animations", k], on);
        }
        set(&op!["animations", "speed"], speed);
        set(&op!["animations", "particles"], particles);
        set(&op!["animations", "blur"], blur);
        set(&op!["animations", "reduce_motion"], reduce);
        crate::state::bump();
    }
    Row::new()
        .gap(6.0)
        .child(button(&t!("Максимум"), || apply(true, 1.0, true, true, false)))
        .child(button(&t!("Сбалансировано"), || apply(true, 0.85, false, false, false)))
        .child(button(&t!("Экономия"), || apply(true, 0.6, false, false, true)))
        .child(button(&t!("Выкл."), || apply(false, 1.0, false, false, true)))
}
