//! Темы оформления: галерея с миниатюрами и применение темы.

use std::cell::RefCell;
use std::sync::Arc;

use synshell_common::config::{Appearance, ColorScheme, Config, Decorations, Palette, Panel, Rgba, COLOR_KEYS};
use synshell_common::theme::{self, Theme, APPEARANCE_KEYS, DECORATION_KEYS, PANEL_KEYS};
use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store::{self, doc_remove, doc_set, Seg};
use crate::sys;
use crate::ui::*;

const COLUMNS: usize = 3;

thread_local! {
    /// Список тем и правила их миниатюр: читаются с диска при открытии
    /// страницы, а таблица стилей окна пересобирается на каждую правку.
    static GALLERY: RefCell<Option<(ColorScheme, String)>> = const { RefCell::new(None) };
}

fn to_color(c: Rgba) -> Color {
    Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0)
}

/// Имя класса миниатюры обоев темы (`""` — стандартная).
fn wall_class(id: &str) -> String {
    if id.is_empty() {
        "theme-wall-default".into()
    } else {
        format!("theme-wall-{id}")
    }
}

/// Перечитать темы и правила обоев для миниатюр.
fn refresh_gallery(scheme: ColorScheme) -> Vec<Arc<Theme>> {
    let themes: Vec<Arc<Theme>> = theme::list().into_iter().map(Arc::new).collect();
    let mut mss = String::new();
    let empty = synshell_common::config::Wallpaper { color: String::new(), color2: String::new(), ..Default::default() };
    for t in std::iter::once(None).chain(themes.iter().map(Some)) {
        let a = preview_appearance(t, scheme);
        mss.push_str(&format!(".{} {{ background: {}; }}\n", wall_class(&a.theme), a.wallpaper_background(&empty)));
    }
    GALLERY.with(|g| *g.borrow_mut() = Some((scheme, mss)));
    themes
}

/// Правила миниатюр для таблицы стилей окна (темы читаются при первом
/// обращении и при открытии страницы).
pub fn gallery_mss(scheme: ColorScheme) -> String {
    if let Some(mss) = GALLERY.with(|g| g.borrow().as_ref().filter(|(s, _)| *s == scheme).map(|(_, m)| m.clone())) {
        return mss;
    }
    refresh_gallery(scheme);
    GALLERY.with(|g| g.borrow().as_ref().map(|(_, m)| m.clone()).unwrap_or_default())
}

/// Как выглядело бы оформление с этой темой (без пользовательских цветов).
fn preview_appearance(t: Option<&Arc<Theme>>, scheme: ColorScheme) -> Appearance {
    Appearance {
        theme: t.map(|t| t.id.clone()).unwrap_or_default(),
        color_scheme: scheme,
        resolved: t.cloned(),
        ..Default::default()
    }
}

fn dot(class: &str, c: Rgba) -> impl Widget {
    DecoratedBox::new().class(class).style("background", to_color(c))
}

fn rounded_dot(class: &str, c: Rgba, radius: f32) -> impl Widget {
    DecoratedBox::new().class(class).style("background", to_color(c)).style("border-radius", radius)
}

/// Миниатюра рабочего стола в палитре темы.
fn thumbnail(a: &Appearance, p: &Palette, radius: f32, opacity: f32) -> impl Widget {
    let r = radius.clamp(0.0, 14.0);
    let (title_active, _) = a.titlebar_colors(&Decorations::default());
    let window = Column::new()
        .class("thumb-window")
        .child(
            Row::new()
                .gap(4.0)
                .class("thumb-titlebar")
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow"))
                .child(dot("thumb-btn", p.warning))
                .child(dot("thumb-btn", p.success))
                .child(dot("thumb-btn", p.danger))
                .style("background", to_color(title_active)),
        )
        .child(
            Column::new()
                .gap(5.0)
                .class("thumb-content")
                .child(dot("thumb-line", p.fg.with_alpha(0.7)))
                .child(dot("thumb-line short", p.muted.with_alpha(0.6)))
                .child(
                    Row::new()
                        .gap(5.0)
                        .child(rounded_dot("thumb-button", p.accent, (r * 0.6).max(1.0)))
                        .child(rounded_dot("thumb-button ghost", p.surface_alt, (r * 0.6).max(1.0))),
                ),
        )
        .style("background", to_color(p.surface))
        .style("border-radius", r);
    let panel = Row::new()
        .gap(5.0)
        .class("thumb-panel")
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(dot("thumb-launcher", p.accent))
        .child(dot("thumb-task", p.accent.with_alpha(0.3)))
        .child(dot("thumb-task", p.fg.with_alpha(0.1)))
        .child(DecoratedBox::new().class("grow"))
        .child(Text::new("12:30").class("thumb-clock").style("color", to_color(p.fg)))
        .style("background", to_color(p.bg.with_alpha(opacity)))
        .style("border-radius", r * 0.7);
    let wall = wall_class(&a.theme);
    DecoratedBox::new().class("theme-thumb").child(
        Stack::new()
            .fit(StackFit::Expand)
            .child(DecoratedBox::new().class(format!("theme-wall {wall}")))
            .child(Column::new().class("thumb-layer").child(window).child(DecoratedBox::new().class("grow")).child(panel)),
    )
}

fn card(t: Option<&Arc<Theme>>, scheme: ColorScheme, selected: bool) -> W {
    let a = preview_appearance(t, scheme);
    let p = a.palette();
    let num = |key: &str, default: f64| {
        t.and_then(|t| t.appearance.get(key)).and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64))).unwrap_or(default)
    };
    let radius = num("corner_radius", 10.0) as f32;
    let opacity = num("panel_opacity", 0.92) as f32;
    let (name, desc) = match t {
        Some(t) => (t.name.clone(), t.description.clone()),
        None => ("Стандартная".to_string(), "Сдержанная палитра synshell: графит и синий акцент.".to_string()),
    };
    let variants = match t {
        Some(t) if t.has_both_variants() => "тёмная · светлая",
        Some(t) if t.dark.is_some() => "тёмная",
        Some(_) => "светлая",
        None => "тёмная · светлая",
    };
    let mut swatches = Row::new().gap(4.0).class("theme-swatches");
    for c in [p.bg, p.surface, p.accent, p.fg, p.success, p.warning, p.danger] {
        swatches = swatches.child(dot("theme-swatch", c));
    }
    let t2 = t.cloned();
    boxed(
        syngui::GestureDetector::new().on_click(move || apply(t2.as_deref())).child(
            DecoratedBox::new().class(if selected { "theme-card selected" } else { "theme-card" }).child(
                Column::new()
                    .gap(0.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(thumbnail(&a, &p, radius, opacity))
                    .child(
                        Column::new()
                            .gap(4.0)
                            .class("theme-meta")
                            .child(
                                Row::new()
                                    .gap(6.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(Text::new(name).class("theme-name"))
                                    .child(DecoratedBox::new().class("grow"))
                                    .child(if selected {
                                        boxed(Icon::new(icons::CHECK).class("theme-check"))
                                    } else {
                                        boxed(Text::new(variants).class("theme-variants"))
                                    }),
                            )
                            .child(Text::new(desc).max_lines(2).class("theme-desc"))
                            .child(swatches),
                    ),
            ),
        ),
    )
}

/// Значение рекомендуемого ключа в тип поля по умолчанию (тема может
/// написать `corner_radius = 12`, а поле — `f32`).
fn to_edit_value(v: &toml::Value, like: Option<&toml::Value>) -> Option<toml_edit::Value> {
    Some(match (v, like) {
        (toml::Value::Integer(i), Some(toml::Value::Float(_))) => (*i as f64).into(),
        (toml::Value::Float(f), Some(toml::Value::Integer(_))) => (f.round() as i64).into(),
        (toml::Value::Integer(i), _) => (*i).into(),
        (toml::Value::Float(f), _) => (*f).into(),
        (toml::Value::String(s), _) => s.as_str().into(),
        (toml::Value::Boolean(b), _) => (*b).into(),
        _ => return None,
    })
}

/// Применить тему (`None` — стандартная): записать `appearance.theme`,
/// сбросить свой акцент, цвета палитры и обоев, выставить рекомендуемую
/// форму. Шрифт и значки не трогаются, если тема их не задаёт (шрифт
/// прошлой темы — снимается).
pub fn apply(t: Option<&Theme>) {
    let prev = store::config().appearance.resolved.clone();
    let cur_font = store::config().appearance.font;
    let fonts = sys::font_families();
    let app_defaults = toml::Table::try_from(Appearance::default()).unwrap_or_default();
    let deco_defaults = toml::Table::try_from(Decorations::default()).unwrap_or_default();
    let empty = toml::Table::new();
    let panel_defaults = toml::Table::try_from(Panel::default()).unwrap_or_default();
    let (rec_app, rec_deco, rec_panel) =
        t.map(|t| (&t.appearance, &t.decorations, &t.panel)).unwrap_or((&empty, &empty, &empty));
    let id = t.map(|t| t.id.clone()).unwrap_or_default();
    // Толщина панелей (не доков) — из темы; панели по умолчанию — в файл.
    store::ensure_aot("panel", &Config::default().panels);
    let panels: Vec<bool> = store::config().panels.iter().map(Panel::is_dock).collect();
    store::edit(move |d| {
        let mut ch = false;
        let k = Seg::K;
        ch |= doc_set(d, &[k("appearance"), k("theme")], id.as_str().into());
        ch |= doc_set(d, &[k("appearance"), k("accent")], "".into());
        for key in COLOR_KEYS {
            ch |= doc_remove(d, &[k("appearance"), k("colors"), k(key)]);
        }
        for key in APPEARANCE_KEYS {
            let rec = rec_app.get(*key);
            match *key {
                "font" => {
                    let prev_font = prev.as_ref().and_then(|p| p.appearance.get("font")).and_then(|v| v.as_str());
                    match rec.and_then(|v| v.as_str()) {
                        // Шрифт темы — только если установлен.
                        Some(f) if f.is_empty() || fonts.iter().any(|x| x.eq_ignore_ascii_case(f)) => {
                            ch |= doc_set(d, &[k("appearance"), k("font")], f.into());
                        }
                        Some(_) => {}
                        None if prev_font.is_some_and(|f| f == cur_font) => {
                            ch |= doc_set(d, &[k("appearance"), k("font")], "".into());
                        }
                        None => {}
                    }
                }
                "icon_theme" | "cursor_theme" => {
                    if let Some(v) = rec.and_then(|v| to_edit_value(v, None)) {
                        ch |= doc_set(d, &[k("appearance"), k(key)], v);
                    }
                }
                _ => {
                    if let Some(v) = rec.or(app_defaults.get(*key)).and_then(|v| to_edit_value(v, app_defaults.get(*key))) {
                        ch |= doc_set(d, &[k("appearance"), k(key)], v);
                    }
                }
            }
        }
        for key in DECORATION_KEYS {
            let like = deco_defaults.get(*key);
            if let Some(v) = rec_deco.get(*key).or(like).and_then(|v| to_edit_value(v, like)) {
                ch |= doc_set(d, &[k("decorations"), k(key)], v);
            }
        }
        for (pi, _) in panels.iter().enumerate().filter(|(_, dock)| !**dock) {
            for key in PANEL_KEYS {
                let like = panel_defaults.get(*key);
                if let Some(v) = rec_panel.get(*key).or(like).and_then(|v| to_edit_value(v, like)) {
                    ch |= doc_set(d, &[k("panel"), Seg::I(pi), k(key)], v);
                }
            }
        }
        ch |= doc_set(d, &[k("decorations"), k("active_color")], "theme".into());
        ch |= doc_set(d, &[k("decorations"), k("inactive_color")], "theme".into());
        ch |= doc_set(d, &[k("wallpaper"), k("color")], "".into());
        ch |= doc_set(d, &[k("wallpaper"), k("color2")], "".into());
        ch
    });
    state::toast(match t {
        Some(t) => format!("Тема «{}» применена", t.name),
        None => "Стандартная тема применена".to_string(),
    });
    state::bump();
}

pub fn themes() -> W {
    let c = store::config();
    let a = &c.appearance;
    let list = refresh_gallery(a.color_scheme);
    // Правила миниатюр появились только что — пересобрать стиль окна.
    state::refresh_theme();
    let current = a.resolved.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    let missing = !a.theme.trim().is_empty() && a.resolved.is_none();

    let mut cards: Vec<W> = vec![card(None, a.color_scheme, current.is_empty() && !missing)];
    cards.extend(list.iter().map(|t| card(Some(t), a.color_scheme, t.id == current)));
    let mut grid = Column::new().gap(14.0);
    let mut it = cards.into_iter().peekable();
    while it.peek().is_some() {
        let mut row = Row::new().gap(14.0);
        for _ in 0..COLUMNS {
            match it.next() {
                Some(w) => row = row.child(w),
                None => row = row.child(DecoratedBox::new().class("theme-card-empty")),
            }
        }
        grid = grid.child(row);
    }

    let scheme_idx = if a.is_dark() { 0 } else { 1 };
    let single = a.resolved.as_ref().is_some_and(|t| !t.has_both_variants());
    let mut body = vec![group(
        "",
        vec![
            row(
                "Вариант",
                if single { "У этой темы один вариант" } else { "Тёмный или светлый вариант темы" },
                SegmentedButton::new(vec!["Тёмный", "Светлый"]).selected(scheme_idx).on_change(|i| {
                    set(&op!["appearance", "color_scheme"], if i == 0 { "dark" } else { "light" });
                    state::bump();
                }),
            ),
            row(
                "Свои темы",
                &format!("{}/<имя>/theme.toml + shell.mss", home_short(&theme::user_dir())),
                button("Открыть папку", || {
                    let dir = theme::user_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    if let Err(e) = std::process::Command::new("xdg-open").arg(&dir).spawn() {
                        state::toast(format!("xdg-open: {e}"));
                    }
                }),
            ),
        ],
    )];
    if missing {
        body.push(note(&format!("Тема «{}» не найдена — используется стандартная.", a.theme)));
    }
    body.push(boxed(grid));
    body.push(note(
        "Тема задаёт палитру, обои без картинки, стиль панелей, меню и окна настроек. \
         Акцент и отдельные цвета можно переопределить на странице «Внешний вид» поверх темы.",
    ));
    page("Темы", "Готовые оформления рабочего стола: панели, меню, уведомления, рамки окон.", body)
}
