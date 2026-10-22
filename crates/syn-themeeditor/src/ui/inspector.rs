//! Справа: инспектор темы — «Быстро» (генератор), «Палитра» (все токены),
//! «Форма», «Код» (MSS-слои) и «О теме».

use syngui::prelude::*;
use syngui::widgets::buttons::Segment;
use syngui::widgets::input::code_editor::syntax::Language;
use syngui::widgets::input::code_editor::{CodeEditor, CompletionItem, CompletionList, Diagnostic, DiagnosticSeverity as Severity, EditorCommand, HoverInfo};
use syngui::GestureDetector;
use synshell_common::config::Rgba;
use synshell_common::tokens::{self, Group, Kind, Source};
use synshell_common::t;

use super::widgets::{card, color_row, group_title, slider_row, Target};
use super::{boxed, icon_button, tip, W};
use crate::color::{self, GenParams};
use crate::icons;
use crate::state::{values, with_draft, Mode, St};

/// Фон для оценки контраста текстовых токенов.
fn contrast_bg(name: &str) -> Option<&'static str> {
    Some(match name {
        "fg" => "bg",
        "muted" => "surface",
        "accent-fg" => "accent",
        "accent" => "surface",
        "danger" | "success" | "warning" => "surface",
        _ => return None,
    })
}

/// Строка цвета для токена реестра.
fn token_row(st: St, tok: &'static tokens::TokenSpec) -> Option<W> {
    if tok.kind != Kind::Color {
        return None;
    }
    let target = match tok.source {
        Source::Palette(k) | Source::Variant(k) => Target::Field(k),
        Source::Derived(_) | Source::App => Target::Var(tok.name.to_string()),
    };
    let sub = match tok.source {
        Source::Derived(_) => format!("--{} · {}", tok.name, t!("вычисляется")),
        Source::App => format!("--{} · {}", tok.name, t!("свой у программ")),
        _ => format!("--{}", tok.name),
    };
    Some(color_row(st, target, synshell_tr::t(tok.label), sub, Some(tok.name.to_string()), contrast_bg(tok.name)))
}

// ── Быстро ──────────────────────────────────────────────────────────────

fn quick(st: St) -> W {
    // Параметры генератора — из текущих цветов темы.
    let start = {
        let vals = values(&st);
        let accent = vals.get("accent").and_then(|v| Rgba::parse(v)).unwrap_or(Rgba::rgb(0x3d, 0x8b, 0xfd));
        let bg = vals.get("bg").and_then(|v| Rgba::parse(v)).map(color::to_oklch);
        GenParams { accent, dark: st.dark(), tint: bg.map(|o| (o.c / 0.045).clamp(0.0, 1.0)).unwrap_or(0.4), hue: bg.filter(|o| o.c > 0.008).map(|o| o.h), contrast: 0.5 }
    };
    let params = use_signal(start);
    // Что пересобирать: акцент меняет только акцент, ползунки — только
    // нейтральные фоны и текст; «Всё заново» и обои — всю палитру.
    const ACCENT_KEYS: &[&str] = &["accent", "accent_fg"];
    const NEUTRAL_KEYS: &[&str] = &["bg", "surface", "surface_alt", "fg", "muted", "border", "titlebar", "titlebar_inactive", "wallpaper_color"];
    let regen = move |p: GenParams, keys: Option<&'static [&'static str]>, label: &'static str| {
        params.set(p);
        let v = st.variant.get_untracked();
        st.edit_quiet(label, move |d| {
            for (k, val) in color::generate(GenParams { dark: v == "dark", ..p }) {
                if keys.is_none_or(|ks| ks.contains(&k)) {
                    d.set(v, k, Some(&val));
                }
            }
        });
        // Без `st.ext`: ползунки и пикер генератора не перестраиваются,
        // пока их тянут.
    };
    // Щелчок по готовому акценту — пикер «Свой акцент» показывает его.
    let preset_rev = use_signal(0u32);
    let apply_accent = move |p: GenParams| regen(p, Some(ACCENT_KEYS), "gen-accent");
    let apply = move |p: GenParams| regen(p, Some(NEUTRAL_KEYS), "gen-neutral");
    let apply_all = move |p: GenParams| regen(p, None, "gen-all");
    let accent_presets: [&str; 10] = ["#3d8bfd", "#7c5cff", "#d946ef", "#f43f5e", "#f97316", "#eab308", "#22c55e", "#14b8a6", "#06b6d4", "#94a3b8"];
    let presets = Reactive::new(move || -> Vec<W> {
        let p = params.get();
        let mut row = Flex::row().wrap().gap(6.0).class("te-presets");
        for hex in accent_presets {
            let c = Rgba::parse(hex).unwrap();
            let on = p.accent == c;
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        apply_accent(GenParams { accent: c, ..params.get_untracked() });
                        preset_rev.set(preset_rev.get_untracked() + 1);
                    })
                    .child(DecoratedBox::new().class(if on { "te-preset te-preset-on" } else { "te-preset" }).style("background-color", Color::from_srgb(c.r, c.g, c.b, 1.0))),
            );
        }
        vec![boxed(row)]
    });
    let accent_picker = Reactive::new(move || -> Vec<W> {
        let _ = (st.ext.get(), preset_rev.get());
        let c = params.get_untracked().accent;
        vec![boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(syngui::widgets::ColorPicker::new().color(syngui::widgets::input::color_picker::ColorValue::new(c.r, c.g, c.b)).width(40.0).on_change(move |cv| {
                    apply_accent(GenParams { accent: Rgba::rgb(cv.r, cv.g, cv.b), ..params.get_untracked() });
                }))
                .child(Text::new(t!("Свой акцент")).class("te-row-label")),
        )]
    });
    let sliders = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.ext.get();
        let p = params.get_untracked();
        let hue_auto = p.hue.is_none();
        vec![boxed(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(slider_row(t!("Окраска фонов"), p.tint, (0.0, 1.0), 0.01, 2, move |v| apply(GenParams { tint: v, ..params.get_untracked() })))
                .child(slider_row(t!("Контраст"), p.contrast, (0.0, 1.0), 0.01, 2, move |v| apply(GenParams { contrast: v, ..params.get_untracked() })))
                .child(slider_row(
                    if hue_auto { t!("Оттенок фонов: как у акцента") } else { t!("Оттенок фонов") },
                    p.hue.unwrap_or_else(|| color::to_oklch(p.accent).h),
                    (0.0, 360.0),
                    1.0,
                    0,
                    move |v| apply(GenParams { hue: Some(v), ..params.get_untracked() }),
                )),
        )]
    });
    let wallpaper_btn = tip(
        Button::new(t!("Из картинки обоев…")).icon(icons::IMAGE).class("te-btn te-wide").on_click(move || {
            std::thread::spawn(move || {
                let cfg = synshell_common::Config::load().0;
                let current = cfg.wallpaper.path.trim().to_string();
                let path = crate::files::choose(&t!("Картинка для палитры"), &t!("Изображения"), &["*.png", "*.jpg", "*.jpeg", "*.webp"], None, "PICTURES").or(if current.is_empty() { None } else { Some(current) });
                let Some(path) = path else { return };
                let dark = st.variant.get_untracked() == "dark";
                let r = color::image_colors(std::path::Path::new(&synshell_common::paths::expand_tilde(&path)));
                run_on_main_thread(move || match r.ok().and_then(|cs| color::params_from_image(&cs, dark)) {
                    Some(p) => {
                        apply_all(p);
                        st.toast.set(t!("Палитра собрана по цветам картинки"));
                    }
                    None => st.toast.set(t!("Не удалось прочитать картинку")),
                });
            });
        }),
        t!("Акцент и оттенок фонов — по основным цветам картинки"),
    );
    let regen_all = tip(
        Button::new(t!("Собрать всю палитру заново")).icon(icons::MAGIC).class("te-btn te-wide").on_click(move || apply_all(params.get_untracked())),
        t!("Фоны, текст, акцент, цвета состояний и обои — из текущих настроек генератора"),
    );
    let flip = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let v = st.variant.get();
        let other = if v == "dark" { "light" } else { "dark" };
        let label = if other == "light" { t!("Пересобрать светлый вариант из тёмного") } else { t!("Пересобрать тёмный вариант из светлого") };
        vec![tip(
            Button::new(label).icon(icons::SWAP).class("te-btn te-wide").on_click(move || {
                st.edit("derive", move |d| d.derive_variant(v, other));
                st.toast.set(t!("Готово — переключитесь на другой вариант вверху, чтобы посмотреть"));
            }),
            t!("Копия текущего варианта с перевёрнутой светлотой цветов"),
        )]
    });
    boxed(
        Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(boxed(Text::new(t!("Акцент меняет только акцент; ползунки пересобирают фоны и текст под него. Точная правка — во вкладке «Палитра».")).max_lines(4).class("te-hint")))
            .child(group_title(t!("Акцент")))
            .child(card(vec![boxed(presets), boxed(accent_picker)]))
            .child(group_title(t!("Фоны")))
            .child(card(vec![boxed(sliders)]))
            .child(group_title(t!("Ещё")))
            .child(card(vec![regen_all, wallpaper_btn, boxed(flip)])),
    )
}

// ── Палитра ─────────────────────────────────────────────────────────────

fn picked(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(p) = st.pick.get() else { return vec![] };
        let mut rows: Vec<W> = Vec::new();
        for name in &p.tokens {
            if let Some(tok) = tokens::find(name) {
                if let Some(r) = token_row(st, tok) {
                    rows.push(r);
                }
            } else {
                // Поля варианта вне реестра: titlebar, wallpaper.
                rows.push(field_row(st, name));
            }
        }
        let mut sels = Flex::row().wrap().gap(4.0).class("te-sel-row");
        for s in &p.selectors {
            let sel = *s;
            sels = sels.child(tip(
                Button::new(sel).class("te-btn te-chip-btn").on_click(move || {
                    add_rule(st, sel);
                }),
                t!("Добавить правило в слой темы и открыть его в «Коде»"),
            ));
        }
        let head = Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(super::icon(icons::PIPETTE).class("te-ic te-pick-ic"))
            .child(DecoratedBox::new().class("grow").child(Text::new(p.label.clone()).class("te-group-title")))
            .child(icon_button(icons::CLOSE, t!("Снять выбор"), "te-mini", move || st.pick.set(None)));
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-picked").child(head).children(rows);
        if !p.selectors.is_empty() {
            col = col.child(Text::new(t!("Правила MSS")).class("te-row-sub")).child(sels);
        }
        vec![boxed(col)]
    }))
}

/// Поля варианта, которых нет в реестре токенов.
fn field_row(st: St, key: &str) -> W {
    match key {
        "titlebar" => color_row(st, Target::Field("titlebar"), t!("Заголовок окна"), t!("цвет или имя: accent, surface, bg"), Some("@titlebar".into()), None),
        "titlebar_inactive" => color_row(st, Target::Field("titlebar_inactive"), t!("Заголовок неактивного окна"), t!("цвет или имя: accent, surface, bg"), Some("@titlebar_inactive".into()), None),
        "wallpaper_color" => color_row(st, Target::Field("wallpaper_color"), t!("Цвет под обоями"), "wallpaper_color".into(), Some("bg".into()), None),
        "wallpaper" => wallpaper_row(st),
        other => boxed(Text::new(other.to_string()).class("te-row-sub")),
    }
}

/// Обои темы — любое значение `background` (градиент).
fn wallpaper_row(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let v = st.variant.get();
        let cur = with_draft(|d| d.get(v, "wallpaper")).unwrap_or_default();
        vec![boxed(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class("te-row")
                .child(Text::new(t!("Обои без картинки")).class("te-row-label"))
                .child(DecoratedBox::new().class("te-scope-shell").child(DecoratedBox::new().class("wallpaper-color te-wall-preview")))
                .child(
                    TextField::new()
                        .text(cur)
                        .placeholder("linear-gradient(170deg, #101420 0%, #2a2050 100%)")
                        .submit_on_focus_lost(true)
                        .on_submit(move |s| {
                            let s = s.trim().to_string();
                            st.edit("f:wallpaper", move |d| d.set(v, "wallpaper", if s.is_empty() { None } else { Some(&s) }));
                        })
                        .class("te-hex te-wide-field"),
                ),
        )]
    }))
}

/// Добавить в слой темы заготовку правила и перейти к коду.
fn add_rule(st: St, selector: &'static str) {
    let layer = if [".sidebar", ".nav-item", ".nav-item.active", ".nav-icon", ".nav-label", ".group-card", ".setting-row", ".row-label", ".row-sub", ".row-sep", ".page-title", ".page-subtitle", "Button", "Button.primary", "Button.danger"].contains(&selector) { "settings" } else { "shell" };
    st.edit("add-rule", move |d| {
        let text = d.mss.entry(layer.to_string()).or_default();
        if !text.contains(&format!("{selector} {{")) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&format!("\n{selector} {{\n    \n}}\n"));
        }
    });
    st.layer.set(layer.to_string());
    st.mode.set(Mode::Code);
}

fn palette(st: St) -> W {
    let search = TextField::new()
        .placeholder(t!("Найти цвет: акцент, рамка, --hover…"))
        .on_change(move |s| st.filter.set(s.trim().to_lowercase()))
        .class("te-search");
    let list = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.variant.get();
        let _ = st.current.get();
        let f = st.filter.get();
        let matches = |label: &str, name: &str, doc: &str| f.is_empty() || label.to_lowercase().contains(&f) || name.contains(f.trim_start_matches('-')) || doc.to_lowercase().contains(&f);
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for g in Group::ALL {
            let mut rows: Vec<W> = Vec::new();
            for tok in tokens::TOKENS.iter().filter(|t| t.group == g) {
                if !matches(&synshell_tr::t(tok.label), tok.name, &synshell_tr::t(tok.doc)) {
                    continue;
                }
                if let Some(r) = token_row(st, tok) {
                    rows.push(r);
                }
            }
            if g == Group::Windows {
                for key in ["titlebar", "titlebar_inactive", "wallpaper_color"] {
                    if matches(key, key, "") || matches(&t!("заголовок обои"), key, "") {
                        rows.push(field_row(st, key));
                    }
                }
                if matches(&t!("обои"), "wallpaper", "") {
                    rows.push(wallpaper_row(st));
                }
            }
            if rows.is_empty() {
                continue;
            }
            col = col.child(group_title(synshell_tr::t(g.label()))).child(card(rows));
        }
        // Свои переменные темы, которых нет в реестре.
        let v = st.variant.get_untracked();
        let custom: Vec<(String, String)> = with_draft(|d| d.vars(v)).into_iter().filter(|(k, _)| tokens::find(k).is_none()).filter(|(k, _)| matches(k, k, "")).collect();
        let mut rows: Vec<W> = custom
            .into_iter()
            .map(|(k, _)| color_row(st, Target::Var(k.clone()), k.clone(), format!("--{k} · {}", t!("своя переменная")), Some(k), None))
            .collect();
        rows.push(new_var_row(st));
        col = col.child(group_title(t!("Свои переменные темы"))).child(card(rows));
        vec![boxed(col)]
    });
    boxed(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(picked(st)).child(search).child(list))
}

fn new_var_row(st: St) -> W {
    boxed(
        Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).class("te-row").child(
            TextField::new()
                .placeholder(t!("Новая переменная: имя и Enter (например, glow)"))
                .on_submit(move |s| {
                    let name = s.trim().trim_start_matches('-').to_string();
                    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                        st.toast.set(t!("Имя — латиница, цифры и «-»"));
                        return;
                    }
                    let v = st.variant.get_untracked();
                    let accent = values(&st).get("accent").cloned().unwrap_or_else(|| "#3d8bfd".into());
                    st.edit("new-var", move |d| d.set_var(v, &name, Some(&accent)));
                })
                .class("te-wide-field"),
        ),
    )
}

// ── Форма ───────────────────────────────────────────────────────────────

fn shape(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.ext.get();
        let cfg = synshell_common::Config::load().0;
        let (radius, opacity, deco_r, title_h, panel, align) = with_draft(|d| {
            (
                d.rec("appearance", "corner_radius").map(|v| v as f32).unwrap_or(cfg.appearance.corner_radius),
                d.rec("appearance", "panel_opacity").map(|v| v as f32).unwrap_or(cfg.appearance.panel_opacity),
                d.rec("decorations", "corner_radius").map(|v| v as f32).unwrap_or(cfg.decorations.corner_radius),
                d.rec("decorations", "title_height").map(|v| v as f32).unwrap_or(cfg.decorations.title_height as f32),
                d.rec("panel", "size").map(|v| v as f32).unwrap_or(44.0),
                d.rec_str("decorations", "title_align").unwrap_or_else(|| "center".into()),
            )
        });
        let num = |table: &'static str, key: &'static str, int: bool| {
            move |v: f32| {
                let val = if int { toml_edit::Value::from(v.round() as i64) } else { toml_edit::Value::from(((v * 100.0).round() / 100.0) as f64) };
                st.edit_quiet(&format!("rec:{table}.{key}"), move |d| d.set_rec(table, key, Some(val)));
            }
        };
        let aligns = ["left", "center", "right"];
        let align_i = aligns.iter().position(|a| *a == align).unwrap_or(1);
        vec![boxed(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(boxed(Text::new(t!("Рекомендации темы: «Параметры системы» выставят их при выборе темы.")).max_lines(3).class("te-hint")))
                .child(group_title(t!("Оболочка")))
                .child(card(vec![
                    slider_row(t!("Скругление панелей и меню"), radius, (0.0, 24.0), 1.0, 0, num("appearance", "corner_radius", false)),
                    slider_row(t!("Непрозрачность панелей"), opacity, (0.3, 1.0), 0.01, 2, num("appearance", "panel_opacity", false)),
                    slider_row(t!("Толщина панели"), panel, (28.0, 64.0), 1.0, 0, num("panel", "size", true)),
                ]))
                .child(group_title(t!("Окна")))
                .child(card(vec![
                    slider_row(t!("Скругление окон"), deco_r, (0.0, 24.0), 1.0, 0, num("decorations", "corner_radius", false)),
                    slider_row(t!("Высота заголовка"), title_h, (22.0, 48.0), 1.0, 0, num("decorations", "title_height", true)),
                    boxed(
                        Column::new().gap(4.0).class("te-slider-row").child(Text::new(t!("Название окна")).class("te-row-label")).child(
                            SegmentedButton::new(vec![Segment::new(t!("Слева")), Segment::new(t!("По центру")), Segment::new(t!("Справа"))])
                                .selected(align_i)
                                .on_change(move |i| {
                                    let a = aligns[i.min(2)];
                                    st.edit("rec:title_align", move |d| d.set_rec("decorations", "title_align", Some(toml_edit::Value::from(a))));
                                })
                                .class("te-seg"),
                        ),
                    ),
                ])),
        )]
    }))
}

// ── Код ─────────────────────────────────────────────────────────────────

/// Слои, которые можно добавить: оболочка, настройки, проводник и программы.
const LAYERS: &[(&str, &str)] = &[
    ("shell", "Оболочка"),
    ("settings", "Параметры системы"),
    ("files", "Проводник и терминал"),
    ("synpkg", "Пакеты"),
    ("syn-remote", "Пульт"),
    ("synassistant", "Ассистент"),
    ("syn-video-player", "Видео"),
    ("syn-audio-player", "Музыка"),
    ("synphone", "Телефон"),
    ("synsms", "Сообщения"),
    ("syncamera", "Камера"),
    ("syn-maps", "Карты"),
    ("syn-vm", "Виртуальные машины"),
];

fn layer_label(id: &str) -> String {
    LAYERS.iter().find(|(l, _)| *l == id).map(|(_, n)| synshell_tr::t(n)).unwrap_or_else(|| id.to_string())
}

/// Диагностики `mss-analysis` в формате `CodeEditor`.
fn diagnostics(text: &str, ctx: &mss_analysis::Context) -> Vec<Diagnostic> {
    mss_analysis::diagnostics(text, ctx)
        .into_iter()
        .map(|d| Diagnostic {
            line: d.range.start.line,
            col: d.range.start.col,
            end_line: d.range.end.line,
            end_col: d.range.end.col,
            severity: match d.severity {
                mss_analysis::Severity::Error => Severity::Error,
                mss_analysis::Severity::Warning => Severity::Warning,
                mss_analysis::Severity::Info => Severity::Info,
                mss_analysis::Severity::Hint => Severity::Hint,
            },
            message: d.message,
        })
        .collect()
}

fn code(st: St) -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let cur = st.layer.get();
        // Оболочка и «Параметры» — всегда первыми, затем остальные слои темы.
        let mut layers: Vec<String> = vec!["shell".into(), "settings".into()];
        layers.extend(with_draft(|d| d.mss.keys().filter(|k| *k != "shell" && *k != "settings").cloned().collect::<Vec<_>>()));
        if !layers.contains(&cur) {
            layers.push(cur.clone());
        }
        let mut row = Flex::row().wrap().gap(4.0).class("te-layer-tabs");
        for l in layers {
            let id = l.clone();
            let has = with_draft(|d| d.mss.get(&l).is_some_and(|t| !t.trim().is_empty()));
            row = row.child(tip(
                Button::new(layer_label(&l)).class(match (l == cur, has) {
                    (true, _) => "te-btn te-chip-btn te-chip-on",
                    (false, true) => "te-btn te-chip-btn",
                    (false, false) => "te-btn te-chip-btn te-chip-empty",
                }).on_click(move || st.layer.set(id.clone())),
                synshell_common::theme::file_of_layer(&l),
            ));
        }
        vec![boxed(row)]
    });
    let add = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let open = st.filter.get() == "\u{1}layers";
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Button::new(t!("Слой программы…")).icon(icons::LAYERS).class("te-btn te-flat").on_click(move || st.filter.set(if st.filter.get_untracked() == "\u{1}layers" { String::new() } else { "\u{1}layers".into() })));
        if open {
            let mut wrap = Flex::row().wrap().gap(4.0).class("te-layer-tabs");
            for (id, name) in LAYERS.iter().skip(2) {
                let id = *id;
                wrap = wrap.child(Button::new(synshell_tr::t(name)).class("te-btn te-chip-btn").on_click(move || {
                    st.layer.set(id.to_string());
                    st.filter.set(String::new());
                }));
            }
            col = col.child(wrap);
        }
        vec![boxed(col)]
    });
    let cmd = use_signal(None::<EditorCommand>);
    let editor = Reactive::new(move || -> Vec<W> {
        let _ = st.ext.get();
        let layer = st.layer.get();
        let text = with_draft(|d| d.mss.get(&layer).cloned().unwrap_or_default());
        let ctx = crate::lang::context(&values(&st));
        let diags = use_signal(diagnostics(&text, &ctx));
        let completion = use_signal(None::<CompletionList>);
        let hover = use_signal(None::<HoverInfo>);
        // Текущий текст — для дополнения и подсказок между правками.
        let cur = std::sync::Arc::new(std::sync::Mutex::new(text.clone()));
        let l2 = layer.clone();
        let (cur_ch, cur_co, cur_ho, cur_go) = (cur.clone(), cur.clone(), cur.clone(), cur.clone());
        let (ctx_ch, ctx_co, ctx_ho) = (ctx.clone(), ctx.clone(), ctx.clone());
        let inner = Reactive::new(move || -> Vec<W> {
            let d = diags.get();
            let errors = d.iter().filter(|x| x.severity == Severity::Error).count();
            let count = d.len();
            let text = if count == 0 { t!("Без ошибок") } else { t!("Ошибок: {e}, замечаний: {w}", e = errors, w = count - errors) };
            vec![boxed(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class("grow").child(Text::new(text).class(if count == 0 { "te-row-sub" } else { "te-error-text" })))
                    .child(Text::new(t!("Ctrl+Space — подсказки, F12 — к определению")).class("te-row-sub")),
            )]
        });
        let ed = CodeEditor::new()
            .text(text)
            .language(Language::Css)
            .show_line_numbers(true)
            .tab_width(4)
            .insert_spaces(true)
            .on_change(move |ch| {
                let text = ch.full_text.to_string();
                if let Ok(mut c) = cur_ch.lock() {
                    c.clone_from(&text);
                }
                diags.set(diagnostics(&text, &ctx_ch));
                let l = l2.clone();
                st.edit_quiet(&format!("code:{l}"), move |d| {
                    if text.trim().is_empty() {
                        d.mss.remove(&l);
                    } else {
                        d.mss.insert(l.clone(), text);
                    }
                });
            })
            .on_save(move |_| st.save())
            .diagnostics_signal(diags)
            .completion(completion)
            .completion_triggers(vec!['-', ':', '.', '(', '@'])
            .on_completion_request(move |tr| {
                let text = cur_co.lock().map(|c| c.clone()).unwrap_or_default();
                let res = mss_analysis::completion(&text, mss_analysis::Position { line: tr.line, col: tr.col }, &ctx_co);
                completion.set(res.filter(|c| !c.items.is_empty()).map(|c| CompletionList {
                    line: c.range.start.line,
                    start_col: c.range.start.col,
                    items: c
                        .items
                        .into_iter()
                        .map(|i| CompletionItem { label: i.label, detail: i.detail, insert_text: i.insert_text, kind: i.kind.short().to_string() })
                        .collect(),
                }));
            })
            .hover(hover)
            .on_hover(move |line, col| {
                let text = cur_ho.lock().map(|c| c.clone()).unwrap_or_default();
                if let Some(h) = mss_analysis::hover(&text, mss_analysis::Position { line, col }, &ctx_ho) {
                    hover.set(Some(HoverInfo { line, col, text: h.markdown }));
                }
            })
            .on_goto(move |line, col| {
                let text = cur_go.lock().map(|c| c.clone()).unwrap_or_default();
                if let Some(r) = mss_analysis::definition(&text, mss_analysis::Position { line, col }) {
                    cmd.set(Some(EditorCommand::SetCursor { line: r.start.line, col: r.start.col }));
                }
            })
            .command_signal(cmd)
            .class("te-code");
        vec![boxed(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("grow").child(DecoratedBox::new().class("grow te-code-wrap").child(ed)).child(inner))]
    });
    // Причесать: форматирование без потери комментариев.
    let format_btn = tip(
        Button::new(t!("Причесать")).icon(icons::BRUSH).class("te-btn te-flat").on_click(move || {
            let layer = st.layer.get_untracked();
            let text = with_draft(|d| d.mss.get(&layer).cloned().unwrap_or_default());
            let formatted = mss_analysis::format(&text);
            if formatted != text {
                st.edit(&format!("format:{layer}"), move |d| {
                    d.mss.insert(layer.clone(), formatted);
                });
            }
        }),
        t!("Отступы и переносы по правилам; документ с ошибками не меняется"),
    );
    let hint = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let l = st.layer.get();
        vec![boxed(Text::new(t!("Правила поверх встроенных стилей ({app}). Цвета берите из переменных: var(--accent), var(--surface)…", app = layer_label(&l))).max_lines(3).class("te-hint"))]
    });
    boxed(
        Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow")
            .child(tabs)
            .child(Row::new().gap(6.0).child(add).child(DecoratedBox::new().class("grow")).child(format_btn))
            .child(hint)
            .child(DecoratedBox::new().class("grow").child(editor)),
    )
}

// ── О теме ──────────────────────────────────────────────────────────────

fn about(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.ext.get();
        let (name, desc, author, id, dir) = with_draft(|d| (d.meta("name"), d.meta("description"), d.meta("author"), d.id.clone(), d.save_dir()));
        let field = |label: String, value: String, key: &'static str, placeholder: String| -> W {
            boxed(
                Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-slider-row").child(Text::new(label).class("te-row-label")).child(
                    TextField::new()
                        .text(value)
                        .placeholder(placeholder)
                        .submit_on_focus_lost(true)
                        .on_submit(move |s| {
                            let s = s.trim().to_string();
                            st.edit(&format!("meta:{key}"), move |d| d.set_meta(key, &s));
                        })
                        .class("te-wide-field"),
                ),
            )
        };
        vec![boxed(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(card(vec![
                    field(t!("Название"), name, "name", t!("Как тема видна в списке")),
                    field(t!("Описание"), desc, "description", t!("Одна-две фразы о настроении темы")),
                    field(t!("Автор"), author, "author", t!("Ваше имя")),
                ]))
                .child(card(vec![
                    boxed(Text::new(t!("Каталог: {dir}", dir = dir.display())).max_lines(3).class("te-row-sub")),
                    boxed(Text::new(t!("Имя каталога: {id}", id = id)).class("te-row-sub")),
                    boxed(
                        Row::new()
                            .gap(8.0)
                            .child(Button::new(t!("Экспорт")).icon(icons::DOWNLOAD).class("te-btn").on_click(move || super::sidebar::export(st)))
                            .child(Button::new(t!("Импорт")).icon(icons::UPLOAD).class("te-btn").on_click(move || super::sidebar::import(st)))
                            .child(Button::new(t!("Вернуть сохранённое")).icon(icons::RESET).class("te-btn").on_click(move || st.revert())),
                    ),
                ])),
        )]
    }))
}

// ── Каркас ──────────────────────────────────────────────────────────────

fn mode_switch(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let m = st.mode.get();
        let modes = [Mode::Quick, Mode::Palette, Mode::Shape, Mode::Code, Mode::About];
        vec![boxed(
            SegmentedButton::new(vec![
                Segment::with_icon(t!("Быстро"), icons::MAGIC),
                Segment::with_icon(t!("Палитра"), icons::PALETTE),
                Segment::with_icon(t!("Форма"), icons::SHAPES),
                Segment::with_icon(t!("Код"), icons::CODE),
                Segment::icon_only(icons::INFO),
            ])
            .selected(modes.iter().position(|x| *x == m).unwrap_or(0))
            .on_change(move |i| st.mode.set(modes[i.min(4)]))
            .class("te-seg te-modes"),
        )]
    }))
}

pub fn inspector(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        let m = st.mode.get();
        let _ = st.current.get();
        match m {
            Mode::Code => vec![code(st)],
            _ => {
                let inner = match m {
                    Mode::Quick => quick(st),
                    Mode::Palette => palette(st),
                    Mode::Shape => shape(st),
                    _ => about(st),
                };
                vec![boxed(ScrollView::new().vertical().child(DecoratedBox::new().class("te-insp-scroll").child(inner)))]
            }
        }
    });
    boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-inspector").child(mode_switch(st)).child(DecoratedBox::new().class("grow").child(body)))
}
