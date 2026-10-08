//! Программы по умолчанию: браузер, почта, терминал, проводник, видео,
//! музыка, картинки… и отдельные типы файлов. Выбор пишется в
//! `~/.config/mimeapps.list` (его читают проводник, оболочка и `xdg-open`);
//! терминал, проводник и браузер — ещё и в `[general]` (Super+Return, Super+E).
//!
//! Страница — список категорий; категория или тип открывается подстраницей
//! выбора (`Ctx::sub`, «назад» — обратно к списку).

use std::cell::RefCell;

use synshell_common::mime;
use synshell_common::xdg::{self, DesktopEntry};
use syngui::prelude::*;

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

/// Категория программ: набор типов, которые меняются вместе.
struct Category {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    /// Главный тип: по нему — текущая программа и подходящие программы.
    main: &'static str,
    mimes: &'static [&'static str],
    /// Ключ `[general]`, который дублирует выбор (команда запуска).
    config: Option<&'static str>,
}

const TERMINAL: &str = "terminal";

const CATEGORIES: &[Category] = &[
    Category {
        id: "browser",
        title: "Браузер",
        icon: "\u{e80b}",
        main: "x-scheme-handler/https",
        mimes: &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"],
        config: Some("browser"),
    },
    Category {
        id: "mail",
        title: "Почта",
        icon: "\u{e158}",
        main: "x-scheme-handler/mailto",
        mimes: &["x-scheme-handler/mailto", "message/rfc822"],
        config: None,
    },
    Category { id: TERMINAL, title: "Терминал", icon: "\u{eb8e}", main: "", mimes: &[], config: Some("terminal") },
    Category {
        id: "files",
        title: "Проводник",
        icon: "\u{e2c7}",
        main: mime::DIRECTORY,
        mimes: &[mime::DIRECTORY],
        config: Some("file_manager"),
    },
    Category {
        id: "video",
        title: "Видео",
        icon: "\u{e02c}",
        main: "video/mp4",
        mimes: &[
            "video/mp4", "video/x-matroska", "video/webm", "video/x-msvideo", "video/quicktime", "video/mpeg",
            "video/ogg", "video/x-flv", "video/3gpp", "video/x-ms-wmv", "video/mp2t",
        ],
        config: None,
    },
    Category {
        id: "music",
        title: "Музыка",
        icon: "\u{e405}",
        main: "audio/mpeg",
        mimes: &[
            "audio/mpeg", "audio/flac", "audio/x-flac", "audio/ogg", "audio/x-vorbis+ogg", "audio/x-opus+ogg",
            "audio/x-wav", "audio/wav", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/x-ms-wma",
        ],
        config: None,
    },
    Category {
        id: "images",
        title: "Изображения",
        icon: "\u{e3f4}",
        main: "image/png",
        mimes: &[
            "image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp", "image/tiff", "image/svg+xml",
            "image/heic", "image/heif", "image/avif", "image/jxl",
        ],
        config: None,
    },
    Category {
        id: "text",
        title: "Текст",
        icon: "\u{e873}",
        main: "text/plain",
        mimes: &["text/plain", "text/markdown", "text/x-log", "application/json", "application/x-yaml", "application/toml"],
        config: None,
    },
    Category {
        id: "pdf",
        title: "Документы PDF",
        icon: "\u{e415}",
        main: "application/pdf",
        mimes: &["application/pdf", "image/vnd.djvu", "application/epub+zip"],
        config: None,
    },
    Category {
        id: "archives",
        title: "Архивы",
        icon: "\u{e149}",
        main: "application/zip",
        mimes: &[
            "application/zip", "application/x-tar", "application/x-compressed-tar", "application/x-xz-compressed-tar",
            "application/x-zstd-compressed-tar", "application/gzip", "application/x-7z-compressed",
            "application/vnd.rar", "application/x-rar",
        ],
        config: None,
    },
];

/// Ключи подстраниц.
const SUB_CAT: &str = "default-apps/cat/";
const SUB_MIME: &str = "default-apps/mime/";

thread_local! {
    /// Строка поиска типов: страница пересобирается после выбора, а набранное
    /// должно остаться.
    static TYPE_QUERY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Типов в списке без поиска и с поиском (весь список — сотни строк).
const TYPES_SHOWN: usize = 60;

// ─── Данные ─────────────────────────────────────────────────────────────────

fn cat(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id)
}

/// Первое слово команды без пути: `/usr/bin/konsole -e` → `konsole`.
fn program(cmd: &str) -> String {
    let w = cmd.split_whitespace().find(|w| !w.contains('=') && *w != "env").unwrap_or("");
    w.rsplit('/').next().unwrap_or(w).to_string()
}

fn terminals() -> Vec<DesktopEntry> {
    let mut v: Vec<DesktopEntry> =
        xdg::apps().iter().filter(|e| e.android.is_none() && e.categories.iter().any(|c| c == "TerminalEmulator")).cloned().collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

/// Текущая программа категории (терминал — по команде из `[general]`).
fn current(c: &Category) -> Option<DesktopEntry> {
    if c.id == TERMINAL {
        let p = program(&store::config().general.terminal);
        return terminals().into_iter().find(|e| program(&e.exec) == p);
    }
    mime::default_app(c.main)
}

/// Подходящие программы: заявившие тип (или терминалы).
fn candidates(main: &str, terminal: bool) -> Vec<DesktopEntry> {
    if terminal {
        return terminals();
    }
    let mut v: Vec<DesktopEntry> =
        mime::apps_for(main).into_iter().filter(|e| e.android.is_none() && (e.takes_files() || main == mime::DIRECTORY || main.starts_with("x-scheme-handler/"))).collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

/// Все программы из меню — для «Показать все».
fn all_apps() -> Vec<DesktopEntry> {
    let mut v: Vec<DesktopEntry> = xdg::apps().iter().filter(|e| !e.no_display && e.android.is_none()).cloned().collect();
    v.sort_by_key(|e| e.name.to_lowercase());
    v
}

fn assign_category(c: &Category, e: &DesktopEntry) {
    if !c.mimes.is_empty() {
        let entries: Vec<(String, Option<String>)> = c.mimes.iter().map(|m| (m.to_string(), Some(e.id.clone()))).collect();
        if let Err(err) = mime::set_default_apps(&entries) {
            state::toast(format!("Не удалось записать mimeapps.list: {err}"));
            return;
        }
    }
    if let Some(key) = c.config {
        set(&op!["general", key], e.command());
    }
    state::toast(format!("{}: {}", c.title, e.name));
    state::bump();
}

fn assign_mime(m: &str, id: Option<&str>) {
    match mime::set_default_apps(&[(m.to_string(), id.map(String::from))]) {
        Ok(()) => state::toast(match id {
            Some(_) => format!("{}: программа выбрана", mime::description(m)),
            None => format!("{}: как в системе", mime::description(m)),
        }),
        Err(err) => state::toast(format!("Не удалось записать mimeapps.list: {err}")),
    }
    state::bump();
}

// ─── Элементы ───────────────────────────────────────────────────────────────

/// Значок программы из темы значков (нет — общий значок программ).
fn app_icon(e: Option<&DesktopEntry>, class: &'static str) -> W {
    match e.and_then(|e| xdg::lookup_icon(&e.icon)) {
        Some(p) => boxed(Image::new(p.to_string_lossy()).placeholder(false).class(class)),
        None => boxed(Icon::new(icons::APPS).class(format!("{class} da-icon-fallback"))),
    }
}

fn badge(icon: &'static str) -> W {
    boxed(DecoratedBox::new().class("hw-badge").child(Icon::new(icon).class("hw-icon")))
}

/// Нажимаемая строка списка.
fn item(content: impl Widget + 'static, on_click: impl Fn() + Send + 'static) -> W {
    boxed(syngui::GestureDetector::new().on_click(on_click).child(DecoratedBox::new().class("da-item").child(content)))
}

fn card(rows: Vec<W>) -> W {
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        col = col.child(r);
        if i + 1 < n {
            col = col.child(DecoratedBox::new().class("row-sep"));
        }
    }
    // overflow обрезает по скруглению только DecoratedBox: иначе подсветка
    // крайней строки при наведении вылезала за углы карточки.
    boxed(DecoratedBox::new().class("group-card da-card").child(col))
}

fn titled(title: &str, w: W) -> W {
    boxed(Column::new().gap(8.0).child(Text::new(title).class("group-title")).child(w))
}

fn category_row(ctx: state::Ctx, c: &'static Category) -> W {
    let cur = current(c);
    let name = match (&cur, c.id) {
        (Some(e), _) => e.name.clone(),
        (None, TERMINAL) => store::config().general.terminal.clone(),
        (None, _) => "Не выбрана".into(),
    };
    let app = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(app_icon(cur.as_ref(), "da-app-icon-sm"))
        .child(Text::new(name).max_lines(1).class(if cur.is_some() { "da-app-name" } else { "da-app-name da-none" }));
    item(
        Row::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(badge(c.icon))
            .child(Column::new().gap(4.0).class("grow").child(Text::new(c.title).class("da-title")).child(app))
            .child(Icon::new(icons::CHEVRON_RIGHT).class("phone-nav-chevron")),
        move || ctx.sub.set(Some((format!("{SUB_CAT}{}", c.id), c.title.to_string()))),
    )
}

fn mime_row(ctx: state::Ctx, m: String) -> W {
    let app = mime::default_app(&m);
    let own = mime::user_default(&m).is_some();
    let desc = mime::description(&m);
    let icon: W = match mime::icon_path(&m) {
        Some(p) => boxed(Image::new(p.to_string_lossy()).placeholder(false).class("da-type-icon")),
        None => boxed(Icon::new(icons::FILE).class("da-type-icon da-icon-fallback")),
    };
    let mut right = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !narrow() {
        right = right
            .child(app_icon(app.as_ref(), "da-app-icon-sm"))
            .child(Text::new(app.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| "—".into())).max_lines(1).class(if own { "da-app-name da-own" } else { "da-app-name" }));
    }
    let mut text = Column::new().gap(2.0).class("grow").child(Text::new(desc.clone()).max_lines(1).class("da-title"));
    text = text.child(Text::new(if narrow() { format!("{m} · {}", app.as_ref().map(|e| e.name.as_str()).unwrap_or("—")) } else { m.clone() }).max_lines(1).class("row-hint"));
    let key = format!("{SUB_MIME}{m}");
    item(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon)
            .child(text)
            .child(right)
            .child(Icon::new(icons::CHEVRON_RIGHT).class("phone-nav-chevron")),
        move || ctx.sub.set(Some((key.clone(), desc.clone()))),
    )
}

/// Строка выбора программы: значок, имя, описание, отметка текущей.
fn choice_row(e: &DesktopEntry, selected: bool, on_pick: impl Fn() + Send + 'static) -> W {
    let hint = if !e.comment.is_empty() { e.comment.clone() } else { e.generic_name.clone() };
    let mut text = Column::new().gap(2.0).class("grow").child(Text::new(e.name.clone()).max_lines(1).class("da-title"));
    if !hint.is_empty() {
        text = text.child(Text::new(hint).max_lines(1).class("row-hint"));
    }
    let mark: W = if selected {
        boxed(Icon::new(icons::CHECK).class("da-check"))
    } else {
        boxed(Icon::new("\u{e836}").class("da-radio"))
    };
    item(
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center).child(app_icon(Some(e), "da-app-icon")).child(text).child(mark),
        on_pick,
    )
}

// ─── Подстраница выбора ─────────────────────────────────────────────────────

/// Выбор программы: подходящие, по кнопке — все остальные.
fn picker(ctx: state::Ctx, title: String, subtitle: String, main: String, terminal: bool, current_id: Option<String>, top: Vec<W>, pick: impl Fn(DesktopEntry) + Clone + Send + Sync + 'static) -> W {
    let show_all = use_signal(false);
    let fit = candidates(&main, terminal);
    let fit_ids: Vec<String> = fit.iter().map(|e| e.id.clone()).collect();
    let mut parts: Vec<W> = Vec::new();
    if !narrow() {
        parts.push(boxed(Row::new().child(button("‹ Программы по умолчанию", move || ctx.sub.set(None)))));
    }
    parts.extend(top);
    let cur = current_id.clone();
    let rows: Vec<W> = fit
        .iter()
        .map(|e| {
            let (p, e2) = (pick.clone(), e.clone());
            choice_row(e, cur.as_deref() == Some(e.id.as_str()), move || p(e2.clone()))
        })
        .collect();
    parts.push(titled("Подходящие программы", if rows.is_empty() { note("Установленных программ для этого типа нет — можно выбрать из всех.") } else { card(rows) }));
    let rest = Reactive::new(move || -> Vec<W> {
        if !show_all.get() {
            return vec![boxed(Row::new().child(button("Показать все программы", move || show_all.set(true))))];
        }
        let rows: Vec<W> = all_apps()
            .into_iter()
            .filter(|e| !fit_ids.contains(&e.id))
            .map(|e| {
                let p = pick.clone();
                let sel = current_id.as_deref() == Some(e.id.as_str());
                let e2 = e.clone();
                choice_row(&e, sel, move || p(e2.clone()))
            })
            .collect();
        vec![titled("Другие программы", card(rows))]
    });
    parts.push(boxed(rest));
    page(&title, &subtitle, parts)
}

fn category_page(ctx: state::Ctx, c: &'static Category) -> W {
    let cur = current(c).map(|e| e.id);
    let subtitle = match c.id {
        TERMINAL => "Открывается по Super+Return и Ctrl+Alt+T.".to_string(),
        "files" => "Открывает папки и запускается по Super+E.".to_string(),
        _ => format!("Типы: {}.", c.mimes.iter().map(|m| mime::description(m)).collect::<Vec<_>>().join(", ")),
    };
    picker(ctx, c.title.to_string(), subtitle, c.main.to_string(), c.id == TERMINAL, cur, Vec::new(), move |e| assign_category(c, &e))
}

fn mime_page(ctx: state::Ctx, m: String) -> W {
    let cur = mime::default_app(&m).map(|e| e.id);
    let own = mime::user_default(&m).is_some();
    let mut top: Vec<W> = Vec::new();
    if own {
        let m2 = m.clone();
        top.push(card(vec![row_inline(
            "Выбрано вами",
            "Вернуть программу, которую назначает система",
            button("Как в системе", move || assign_mime(&m2, None)),
        )]));
    }
    let m3 = m.clone();
    picker(ctx, mime::description(&m), m.clone(), m.clone(), false, cur, top, move |e| assign_mime(&m3, Some(&e.id)))
}

// ─── Страница ───────────────────────────────────────────────────────────────

fn types_group(ctx: state::Ctx) -> W {
    let query = use_signal(TYPE_QUERY.with(|q| q.borrow().clone()));
    let search = TextField::with_text(query.get_untracked())
        .placeholder("Найти тип: pdf, mp4, markdown…")
        .prefix_icon(icons::SEARCH)
        .on_change(move |s| {
            TYPE_QUERY.with(|q| *q.borrow_mut() = s.to_string());
            query.set(s.to_string());
        });
    let types = mime::known_types();
    let list = Reactive::new(move || -> Vec<W> {
        let q = query.get().trim().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let matches = |m: &str| {
            if words.is_empty() {
                // Без поиска — только типы со своим выбором и самые частые.
                return mime::user_default(m).is_some();
            }
            let hay = format!("{m} {}", mime::description(m)).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        };
        let found: Vec<&String> = types.iter().filter(|m| matches(m)).collect();
        if found.is_empty() {
            return vec![note(if words.is_empty() { "Своих назначений для отдельных типов пока нет — найдите тип поиском." } else { "Ничего не найдено." })];
        }
        let total = found.len();
        let rows: Vec<W> = found.into_iter().take(TYPES_SHOWN).map(|m| mime_row(ctx, m.clone())).collect();
        let mut out = vec![card(rows)];
        if total > TYPES_SHOWN {
            out.push(note(&format!("Показано {TYPES_SHOWN} из {total} — уточните поиск.")));
        }
        out
    });
    let hint = if narrow() { "" } else { "Без поиска — типы, для которых программа выбрана вами." };
    boxed(
        Column::new()
            .gap(8.0)
            .child(Text::new("Типы файлов").class("group-title"))
            .child(search)
            .child(if hint.is_empty() { boxed(DecoratedBox::new()) } else { note(hint) })
            .child(list),
    )
}

fn overview(ctx: state::Ctx) -> W {
    let rows: Vec<W> = CATEGORIES.iter().map(|c| category_row(ctx, c)).collect();
    page(
        "Программы по умолчанию",
        "Чем открывать ссылки, папки и файлы. Действует в проводнике, оболочке и других программах (mimeapps.list).",
        vec![titled("Основные", card(rows)), types_group(ctx)],
    )
}

pub fn default_apps() -> W {
    let ctx = state::ctx();
    let view = Reactive::new(move || -> Vec<W> {
        let sub = ctx.sub.get().map(|(k, _)| k);
        let key = sub.clone().unwrap_or_default();
        vec![boxed(
            AnimatedSwitcher::new(if sub.is_some() { 2u64 } else { 1 }, move || {
                if let Some(c) = key.strip_prefix(SUB_CAT).and_then(cat) {
                    category_page(ctx, c)
                } else if let Some(m) = key.strip_prefix(SUB_MIME) {
                    mime_page(ctx, m.to_string())
                } else {
                    overview(ctx)
                }
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(240)
            .exit_duration_ms(160)
            .animate_size(false)
            .class("grow"),
        )]
    });
    boxed(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name() {
        assert_eq!(program("/usr/bin/konsole --separate"), "konsole");
        assert_eq!(program("env GDK_BACKEND=x11 foot"), "foot");
        assert_eq!(program("kitty"), "kitty");
    }
}
