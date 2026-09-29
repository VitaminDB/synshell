//! Внешний вид, обои, оформление окон, анимации.

use synshell_common::config::Rgba;
use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store;
use crate::sys;
use crate::ui::*;

const ACCENTS: &[(&str, &str)] = &[
    ("#3d8bfd", "Синий"),
    ("#7c5cff", "Фиолетовый"),
    ("#d946ef", "Пурпурный"),
    ("#ef4466", "Красный"),
    ("#f97316", "Оранжевый"),
    ("#eab308", "Жёлтый"),
    ("#22c55e", "Зелёный"),
    ("#14b8a6", "Бирюзовый"),
    ("#64748b", "Графит"),
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
                                    .child(Text::new("Окно").class("preview-title").style("color", to_color(&p.fg.hex())))
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
        icon_opts.insert(0, (a.icon_theme.clone(), format!("{} (не найдена)", a.icon_theme)));
    }
    let mut cursor_opts = cursor_themes.clone();
    if !cursor_opts.iter().any(|(id, _)| *id == a.cursor_theme) {
        cursor_opts.insert(0, (a.cursor_theme.clone(), format!("{} (не найдена)", a.cursor_theme)));
    }
    let fonts = sys::font_families();

    let colors_rows: Vec<W> = [
        ("bg", "Фон"),
        ("surface", "Поверхности"),
        ("surface_alt", "Поверхности (вторичные)"),
        ("fg", "Текст"),
        ("muted", "Приглушённый текст"),
        ("border", "Границы"),
        ("accent_fg", "Текст на акценте"),
        ("danger", "Опасность"),
        ("success", "Успех"),
        ("warning", "Предупреждение"),
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
        "Внешний вид",
        "Цвета, шрифты и темы — общие для панелей, меню, рамок окон и этого окна.",
        vec![
            preview(),
            group(
                "Цвета",
                vec![
                    row(
                        "Цветовая схема",
                        "",
                        SegmentedButton::new(vec!["Тёмная", "Светлая"]).selected(scheme_idx).on_change(|i| {
                            set(&op!["appearance", "color_scheme"], if i == 0 { "dark" } else { "light" });
                        }),
                    ),
                    row_wide("Акцентный цвет", "Кнопки, выделение, активные элементы", swatches),
                    row("Свой акцент", "Пусто — акцент темы", color_field(op!["appearance", "accent"], &a.accent, "из темы")),
                ],
            ),
            group(
                "Шрифты",
                vec![
                    row(
                        "Шрифт интерфейса",
                        "Пусто — системный (fontconfig)",
                        {
                            let p = op!["appearance", "font"];
                            let p2 = p.clone();
                            Autocomplete::new(fonts)
                                .text(a.font.clone())
                                .placeholder("Системный")
                                .width(260.0)
                                .on_change(move |s| set(&p, s.to_string()))
                                .on_select(move |s| set(&p2, s.to_string()))
                        },
                    ),
                    row(
                        "Размер шрифта",
                        "",
                        float_spin(op!["appearance", "font_size"], a.font_size as f64, 8.0, 24.0, 0.5, 1),
                    ),
                ],
            ),
            group(
                "Форма и масштаб",
                vec![
                    slider_row(
                        "Скругление углов",
                        "Панели, меню и окна",
                        op!["appearance", "corner_radius"],
                        a.corner_radius as f64,
                        0.0,
                        24.0,
                        1.0,
                        0,
                    ),
                    slider_row(
                        "Непрозрачность панелей",
                        "",
                        op!["appearance", "panel_opacity"],
                        a.panel_opacity as f64,
                        0.3,
                        1.0,
                        0.01,
                        2,
                    ),
                    slider_row(
                        "Масштаб оболочки",
                        "Дополнительно к масштабу монитора",
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
                "Темы",
                vec![
                    row("Значки", "", choice_owned(op!["appearance", "icon_theme"], &a.icon_theme, icon_opts, 260.0)),
                    row(
                        "Курсор",
                        "",
                        choice_owned(op!["appearance", "cursor_theme"], &a.cursor_theme, cursor_opts, 260.0),
                    ),
                    row(
                        "Размер курсора",
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
                ],
            ),
            group(
                "Программы",
                vec![switch_row(
                    "Цвета программ по теме",
                    "GTK (Breeze, libadwaita), Qt/KDE и GIMP; открытые окна GTK — после перезапуска",
                    op!["appearance", "app_colors"],
                    a.app_colors,
                )],
            ),
            group("Палитра", colors_rows),
            note("Пустое поле — цвет схемы. Тонкая настройка стилей — в ~/.config/synshell/theme.mss (MSS поверх встроенной темы)."),
        ],
    )
}

// ─── Обои ───────────────────────────────────────────────────────────────────

fn wallpaper_preview() -> W {
    let tick = state::ctx().tick;
    boxed(DecoratedBox::new().class("wall-preview").clip(true).child(move || {
        tick.get();
        let w = store::config().wallpaper;
        let path = synshell_common::paths::expand_tilde(&w.path);
        // Градиент из [wallpaper] или обои темы — правило `.desk-bg`.
        let bg = DecoratedBox::new().class("wall-fill desk-bg");
        let mut st = Stack::new().fit(StackFit::Expand).clip(true).child(bg);
        let img = if path.is_file() {
            Some(path)
        } else if path.is_dir() {
            first_image(&path)
        } else {
            None
        };
        if let Some(p) = img {
            let fit = match w.mode.as_str() {
                "fit" | "center" => ImageFit::Contain,
                "stretch" => ImageFit::Fill,
                _ => ImageFit::Cover,
            };
            st = st.child(Image::new(p.display().to_string()).fit(fit).placeholder(false).class("wall-image"));
        }
        st
    }))
}

fn first_image(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp" | "bmp")
            )
        })
        .collect();
    v.sort();
    v.into_iter().next()
}

/// Кнопка «Выбрать…»: внешний диалог в отдельном потоке, результат —
/// обратно на главный поток.
fn pick_button(label: &str, directory: bool, p: P) -> impl Widget {
    Button::new(label).class("btn").icon(if directory { icons::FOLDER } else { icons::FILE }).on_click(move || {
        let p = p.clone();
        let start = store::config().wallpaper.path;
        std::thread::spawn(move || {
            if let Some(path) = sys::pick_path(directory, &start) {
                run_on_main_thread(move || {
                    set(&p, path);
                    state::bump();
                });
            }
        });
    })
}

pub fn wallpaper() -> W {
    let c = store::config();
    let w = &c.wallpaper;
    let has_dialog = sys::has_file_dialog();

    let mut pick = Row::new().gap(8.0).child(text(op!["wallpaper", "path"], &w.path, "Путь к картинке или каталогу", 340.0));
    if has_dialog {
        pick = pick
            .child(pick_button("Файл…", false, op!["wallpaper", "path"]))
            .child(pick_button("Каталог…", true, op!["wallpaper", "path"]));
    }

    let mut per_output: Vec<W> = w
        .per_output
        .iter()
        .map(|(out, path)| {
            let out_s = out.clone();
            row(
                out,
                "",
                Row::new()
                    .gap(8.0)
                    .child(text(op!["wallpaper", "per_output", out.as_str()], path, "Путь", 300.0))
                    .child(danger_icon_button(icons::DELETE, move || {
                        unset(&op!["wallpaper", "per_output", out_s.as_str()]);
                        state::bump();
                    })),
            )
        })
        .collect();
    let new_out = use_signal(String::new());
    per_output.push(row(
        "Добавить монитор",
        "Имя вывода: eDP-1, HDMI-A-1…",
        Row::new()
            .gap(8.0)
            .child(TextField::new().placeholder("HDMI-A-1").width(160.0).on_change(move |s| new_out.set(s.to_string())))
            .child(icon_button(icons::ADD, move || {
                let n = new_out.get_untracked().trim().to_string();
                if !n.is_empty() {
                    set(&op!["wallpaper", "per_output", n], "");
                    state::bump();
                }
            })),
    ));

    page(
        "Обои",
        "Фон рабочего стола. Картинка, каталог для слайд-шоу или градиент из двух цветов.",
        vec![
            wallpaper_preview(),
            group(
                "Изображение",
                vec![
                    row_wide("Картинка или каталог", "Пусто — градиент из цветов ниже", pick),
                    choice_row(
                        "Заполнение",
                        "",
                        op!["wallpaper", "mode"],
                        &w.mode,
                        &[
                            ("fill", "Заполнить (обрезка)"),
                            ("fit", "Вписать"),
                            ("stretch", "Растянуть"),
                            ("center", "По центру"),
                            ("tile", "Мозаика"),
                        ],
                    ),
                    int_row(
                        "Слайд-шоу",
                        "Смена картинки из каталога, минут (0 — выкл.)",
                        op!["wallpaper", "slideshow_minutes"],
                        w.slideshow_minutes as i64,
                        0,
                        1440,
                        5,
                    ),
                ],
            ),
            group(
                "Цвет",
                vec![
                    row("Основной цвет", "", color_field(op!["wallpaper", "color"], &w.color, "#1b2233")),
                    row("Второй цвет", "Пусто — сплошной цвет", color_field(op!["wallpaper", "color2"], &w.color2, "")),
                ],
            ),
            group(
                "Рабочий стол",
                vec![switch_row(
                    "Значки на рабочем столе",
                    "Файлы из ~/Desktop",
                    op!["wallpaper", "desktop_icons"],
                    w.desktop_icons,
                )],
            ),
            group("Свои обои для мониторов", per_output),
        ],
    )
}

// ─── Оформление окон ────────────────────────────────────────────────────────

const BUTTON_NAMES: &[(&str, &str)] = &[
    ("icon", "Значок (меню окна)"),
    ("sticky", "На всех столах"),
    ("above", "Поверх других"),
    ("minimize", "Свернуть"),
    ("maximize", "Развернуть"),
    ("close", "Закрыть"),
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
            let label = BUTTON_NAMES.iter().find(|(n, _)| n == name).map(|(_, l)| *l).unwrap_or(name.as_str());
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
        add = add.child(Button::new(format!("+ {label}")).class("btn small").on_click(move || {
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
                    .child(side("Слева", left.clone(), true, left.clone(), right.clone()))
                    .child(side("Справа", right.clone(), false, left.clone(), right.clone())),
            )
            .child(add),
    )
}

pub fn decorations() -> W {
    let c = store::config();
    let d = &c.decorations;
    let color_opts: &[(&str, &str)] =
        &[("theme", "Из темы"), ("accent", "Акцент"), ("surface", "Поверхность"), ("bg", "Фон"), ("surface_alt", "Вторичная поверхность")];
    let color_row = |label: &str, key: &'static str, cur: &str| -> W {
        let is_named = color_opts.iter().any(|(v, _)| *v == cur);
        let named_val = if is_named { cur } else { "custom" };
        let mut opts: Vec<(String, String)> = color_opts.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        opts.push(("custom".into(), "Свой цвет".into()));
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
        "Оформление окон",
        "Серверные рамки и заголовки, которые рисует композитор (если в «Поведении окон» выбраны рамки композитора).",
        vec![
            group(
                "Заголовок",
                vec![
                    int_row("Высота заголовка", "", op!["decorations", "title_height"], d.title_height as i64, 16, 64, 1),
                    row(
                        "Размер шрифта",
                        "",
                        float_spin(op!["decorations", "font_size"], d.font_size as f64, 8.0, 24.0, 0.5, 1),
                    ),
                    choice_row(
                        "Выравнивание заголовка",
                        "",
                        op!["decorations", "title_align"],
                        &d.title_align,
                        &[("left", "Слева"), ("center", "По центру")],
                    ),
                    color_row("Цвет активного окна", "active_color", &d.active_color),
                    color_row("Цвет неактивного окна", "inactive_color", &d.inactive_color),
                ],
            ),
            group("Кнопки заголовка", vec![row_wide("Расположение", "", buttons_editor(&d.buttons))]),
            group(
                "Форма и тень",
                vec![
                    slider_row(
                        "Скругление углов окна",
                        "",
                        op!["decorations", "corner_radius"],
                        d.corner_radius as f64,
                        0.0,
                        24.0,
                        1.0,
                        0,
                    ),
                    int_row(
                        "Зона изменения размера",
                        "Невидимая полоса вокруг окна, px",
                        op!["decorations", "resize_border"],
                        d.resize_border as i64,
                        0,
                        32,
                        1,
                    ),
                    switch_row("Тень", "", op!["decorations", "shadow"], d.shadow),
                    int_row("Размер тени", "", op!["decorations", "shadow_size"], d.shadow_size as i64, 0, 96, 2),
                    slider_row(
                        "Плотность тени",
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
    let win: &[(&str, &str)] = &[("zoom", "Масштаб"), ("fade", "Растворение"), ("slide", "Выезд"), ("none", "Нет")];
    page(
        "Анимации",
        "Эффекты открытия окон, переключения столов и сворачивания.",
        vec![
            group(
                "",
                vec![
                    switch_row("Анимации включены", "", op!["animations", "enabled"], a.enabled),
                    slider_row(
                        "Длительность",
                        "0.5 — вдвое быстрее, 2 — вдвое медленнее",
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
                "Эффекты",
                vec![
                    choice_row("Открытие окна", "", op!["animations", "window_open"], &a.window_open, win),
                    choice_row("Закрытие окна", "", op!["animations", "window_close"], &a.window_close, win),
                    choice_row(
                        "Сворачивание",
                        "",
                        op!["animations", "minimize"],
                        &a.minimize,
                        &[("zoom", "К панели"), ("fade", "Растворение"), ("none", "Нет")],
                    ),
                    choice_row(
                        "Переключение столов",
                        "",
                        op!["animations", "workspace_switch"],
                        &a.workspace_switch,
                        &[
                            ("slide", "Сдвиг по горизонтали"),
                            ("slide-vertical", "Сдвиг по вертикали"),
                            ("fade", "Растворение"),
                            ("none", "Нет"),
                        ],
                    ),
                    switch_row(
                        "Плавная смена раскладки",
                        "Окна перетекают на новые места при плиточной раскладке",
                        op!["animations", "layout_changes"],
                        a.layout_changes,
                    ),
                ],
            ),
            group(
                "Оболочка",
                vec![
                    switch_row(
                        "Анимации оболочки",
                        "Всплывающие окна вырастают из панели, меню запуска перетекает между разделами, уведомления въезжают",
                        op!["animations", "shell"],
                        a.shell,
                    ),
                    switch_row(
                        "Плавная смена темы",
                        "Цвета перетекают, обои растворяются при смене оформления",
                        op!["animations", "theme_change"],
                        a.theme_change,
                    ),
                    switch_row("Меньше движения", "Короче переходы, без частиц и волн", op!["animations", "reduce_motion"], a.reduce_motion),
                ],
            ),
            group(
                "Группы оболочки",
                vec![
                    switch_row("Домашний экран", "Листание страниц, появление и уход (телефон)", op!["animations", "home"], a.home),
                    switch_row("«Пуск»", "Появление, перетекание в «Все приложения», меню значка", op!["animations", "menu"], a.menu),
                    switch_row("Шторка", "Выезд сверху, затемнение (телефон)", op!["animations", "shade"], a.shade),
                    switch_row("Док и панели", "Увеличение значков, прыжки при запуске", op!["animations", "dock"], a.dock),
                    switch_row("Приложения и режимы окон", "Листание приложений, «Недавние», смена режима (телефон)", op!["animations", "pages"], a.pages),
                ],
            ),
            group(
                "Эффекты",
                vec![
                    switch_row("Частицы", "Искры и конфетти дока", op!["animations", "particles"], a.particles),
                    switch_row("Размытие", "Под меню и всплывающими окнами; на CPU-композиторе телефона дорого", op!["animations", "blur"], a.blur),
                    switch_row("Волна от нажатия", "", op!["animations", "ripple"], a.ripple),
                ],
            ),
            group("Наборы", vec![row_inline("Быстро выставить", "", presets())]),
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
        .child(button("Максимум", || apply(true, 1.0, true, true, false)))
        .child(button("Сбалансировано", || apply(true, 0.85, false, false, false)))
        .child(button("Экономия", || apply(true, 0.6, false, false, true)))
        .child(button("Выкл.", || apply(false, 1.0, false, false, true)))
}
