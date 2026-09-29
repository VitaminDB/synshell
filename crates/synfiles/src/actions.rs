//! Команды: открыть, буфер обмена, удалить, переименовать, создать; меню
//! и сочетания клавиш — всё сводится к вызовам отсюда.

use std::path::{Path, PathBuf};

use synshell_common::{mime, xdg};
use syngui::prelude::*;
use syngui::data::ItemSelection;
use syngui::widgets::MenuItem;

use crate::loc::Location;
use crate::model::{Entry, SortKey};
use crate::ops::{self, Op};
use crate::state::{self, Clip, Dialog, Pane, ViewMode};
use crate::ui::icons;

// ---------------------------------------------------------------- запуск

/// Запустить команду оболочки отвязанно от окна.
pub fn spawn_shell(cmd: &str, cwd: Option<&Path>) {
    use std::os::unix::process::CommandExt;
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(cmd);
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Err(e) = c.spawn() {
        state::toast_error(format!("Не удалось запустить: {e}"));
    }
}

fn terminal_cmd() -> String {
    let t = state::ctx().cfg.get_untracked().general.terminal.clone();
    if t.trim().is_empty() {
        "konsole".into()
    } else {
        t
    }
}

pub fn launch(app: &xdg::DesktopEntry, files: &[PathBuf]) {
    let cwd = files.first().and_then(|f| f.parent()).map(Path::to_path_buf);
    for cmd in app.commands_for(files) {
        let cmd = if app.terminal { format!("{} -e {cmd}", terminal_cmd()) } else { cmd };
        spawn_shell(&cmd, cwd.as_deref());
    }
}

/// Открыть терминал в папке.
pub fn open_terminal(dir: &Path) {
    spawn_shell(&terminal_cmd(), Some(dir));
}

/// Открыть запись: папку — в панели (или новой вкладке), файл — программой.
pub fn open_entry(p: Pane, e: &Entry, new_tab: bool) {
    if e.is_dir {
        let loc = Location::Dir(e.path.clone());
        if new_tab {
            state::new_tab(loc, false);
        } else {
            state::navigate(p, loc, true);
        }
        return;
    }
    if e.broken {
        state::toast_error(format!("Ссылка «{}» ведёт в никуда", e.name));
        return;
    }
    open_files(&[e.path.clone()], &e.mime);
}

pub fn open_files(files: &[PathBuf], mime_type: &str) {
    // .desktop — запуск программы.
    if mime_type == "application/x-desktop" {
        for f in files {
            let id = f.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(app) = xdg::parse_desktop_file(f, id, &[]) {
                launch(&app, &[]);
            }
        }
        return;
    }
    // Исполняемые ELF — запускаем, как Проводник .exe.
    if matches!(mime_type, "application/x-executable" | "application/x-pie-executable")
        && files.iter().all(|f| std::fs::metadata(f).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0).unwrap_or(false))
    {
        for f in files {
            spawn_shell(&format!("'{}'", f.to_string_lossy().replace('\'', "'\\''")), f.parent());
        }
        return;
    }
    // Картинки — встроенным просмотрщиком (листает всю папку, поэтому
    // хватает первой); другая программа — через «Открыть с помощью».
    if crate::viewer::handles(mime_type) {
        if let Some(f) = files.first() {
            crate::viewer::open(f);
        }
        return;
    }
    match mime::default_app(mime_type) {
        Some(app) => launch(&app, files),
        None => state::ctx().dialog.set(Some(Dialog::OpenWith { paths: files.to_vec(), mime: mime_type.to_string() })),
    }
}

/// Открыть выделенное (Enter): папки — первую в панели, остальные во вкладках;
/// файлы — группами по типу.
pub fn open_selected(p: Pane) {
    let sel = state::selected_entries(p);
    let (dirs, files): (Vec<Entry>, Vec<Entry>) = sel.into_iter().partition(|e| e.is_dir);
    let mut by_mime: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for f in files {
        match by_mime.iter_mut().find(|(m, _)| *m == f.mime) {
            Some((_, v)) => v.push(f.path),
            None => by_mime.push((f.mime, vec![f.path])),
        }
    }
    for (m, v) in by_mime {
        open_files(&v, &m);
    }
    for (i, d) in dirs.iter().enumerate() {
        open_entry(p, d, i > 0);
    }
}

// ---------------------------------------------------------------- буфер

/// Положить пути в буфер (свой и системный: GNOME, KDE, uri-list, текст).
pub fn set_clipboard(paths: Vec<PathBuf>, cut: bool) {
    if paths.is_empty() {
        return;
    }
    let uris = syngui::clipboard::uri_list(&paths);
    let gnome = format!("{}\n{}", if cut { "cut" } else { "copy" }, uris.replace("\r\n", "\n"));
    let text = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
    let kde_cut: &[u8] = if cut { b"1" } else { b"0" };
    syngui::clipboard::copy_rich(
        &text,
        &[
            ("text/uri-list", uris.as_bytes()),
            ("x-special/gnome-copied-files", gnome.as_bytes()),
            ("application/x-kde-cutselection", kde_cut),
        ],
    );
    let n = paths.len();
    state::ctx().clip.set(Some(Clip { paths, cut }));
    state::toast(if cut {
        format!("Вырезано: {}", crate::model::format_count(n as u32))
    } else {
        format!("Скопировано: {}", crate::model::format_count(n as u32))
    });
}

/// Что сейчас в буфере: пути и «вырезано».
pub fn read_clipboard() -> Option<(Vec<PathBuf>, bool)> {
    let parse_uris = |s: &str| -> Vec<PathBuf> {
        s.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(xdg::path_from_uri).collect()
    };
    if let Some((m, data)) = syngui::clipboard::paste_mime(&["x-special/gnome-copied-files", "text/uri-list"]) {
        let s = String::from_utf8_lossy(&data).to_string();
        if m == "x-special/gnome-copied-files" {
            let mut lines = s.lines();
            let cut = lines.next().map(|l| l.trim() == "cut").unwrap_or(false);
            let paths = parse_uris(&lines.collect::<Vec<_>>().join("\n"));
            return (!paths.is_empty()).then_some((paths, cut));
        }
        let paths = parse_uris(&s);
        let cut = syngui::clipboard::paste_mime(&["application/x-kde-cutselection"])
            .map(|(_, d)| d.first() == Some(&b'1'))
            .unwrap_or(false);
        return (!paths.is_empty()).then_some((paths, cut));
    }
    // Без Wayland data-control — свой буфер или текст с путями.
    if let Some(c) = state::ctx().clip.get_untracked() {
        return Some((c.paths, c.cut));
    }
    let text = syngui::clipboard::paste()?;
    let paths: Vec<PathBuf> = text
        .lines()
        .map(str::trim)
        .filter_map(|l| if l.starts_with("file://") { xdg::path_from_uri(l) } else { Some(PathBuf::from(l)) })
        .filter(|p| p.is_absolute() && p.exists())
        .collect();
    (!paths.is_empty()).then_some((paths, false))
}

pub fn can_paste() -> bool {
    state::ctx().clip.get_untracked().is_some()
        || syngui::clipboard::available_mimes().iter().any(|m| m == "text/uri-list" || m == "x-special/gnome-copied-files")
}

pub fn paste_into(dir: &Path) {
    let Some((paths, cut)) = read_clipboard() else {
        state::toast("Буфер обмена пуст");
        return;
    };
    let op = if cut { Op::Move { srcs: paths, dest: dir.to_path_buf() } } else { Op::Copy { srcs: paths, dest: dir.to_path_buf() } };
    if cut {
        state::ctx().clip.set(None);
        syngui::clipboard::copy("");
    }
    ops::start(op);
}

pub fn copy_paths_text(paths: &[PathBuf]) {
    let text = paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
    syngui::clipboard::copy(&text);
    state::toast(if paths.len() == 1 { "Путь скопирован".to_string() } else { format!("Скопировано путей: {}", paths.len()) });
}

// ---------------------------------------------------------------- удаление

pub fn trash_selected(p: Pane) {
    let paths = state::selected_paths(p);
    if paths.is_empty() {
        return;
    }
    if matches!(p.loc.get_untracked(), Location::Trash) {
        // Из корзины — только навсегда.
        state::ctx().dialog.set(Some(Dialog::ConfirmDelete { paths }));
        return;
    }
    if state::ctx().cfg.get_untracked().files.confirm_trash {
        state::ctx().dialog.set(Some(Dialog::ConfirmDelete { paths }));
        return;
    }
    let n = paths.len();
    ops::start(Op::Trash { srcs: paths });
    state::toast_undo(format!("В корзину: {}", crate::model::format_count(n as u32)));
}

pub fn delete_selected(p: Pane) {
    let paths = state::selected_paths(p);
    if !paths.is_empty() {
        state::ctx().dialog.set(Some(Dialog::ConfirmDelete { paths }));
    }
}

pub fn undo() {
    match ops::undo() {
        None => state::toast("Нечего отменять"),
        Some(Ok(t)) => {
            state::toast(t.replace("Отменить", "Отменено:").replace("Отменено: ", "Отменено — "));
            state::reload_all();
        }
        Some(Err(e)) => state::toast_error(format!("Не удалось отменить: {e}")),
    }
}

// ---------------------------------------------------------------- создание и имена

pub fn start_rename(p: Pane) {
    let sel = p.sel.get_untracked();
    let entries = p.entries.get_untracked();
    let i = sel.cursor.filter(|c| sel.is_selected(*c)).or_else(|| sel.selected.iter().next().copied());
    if let Some(e) = i.and_then(|i| entries.get(i)) {
        if e.trash.is_some() {
            return;
        }
        p.renaming.set(Some(e.path.clone()));
        if let Some(i) = i {
            state::scroll_to(p, i);
        }
    }
}

pub fn finish_rename(p: Pane, from: &Path, new_name: &str) {
    p.renaming.set(None);
    if ops::name_of(from) == new_name.trim() {
        return;
    }
    match ops::rename(from, new_name) {
        Ok(to) => {
            if let Some(dir) = to.parent() {
                state::select_later(p, to.clone());
                state::reload_dir(dir);
            }
        }
        Err(e) => state::toast_error(e),
    }
}

pub fn new_folder(p: Pane) {
    let Some(dir) = p.loc.get_untracked().dir().map(Path::to_path_buf) else { return };
    // Как в Проводнике: сразу создать «Новая папка» и включить переименование.
    match ops::new_folder(&dir, "Новая папка") {
        Ok(path) => after_create(p, path),
        Err(e) => state::toast_error(e),
    }
}

pub fn new_file(p: Pane, name: &str, content: &[u8]) {
    let Some(dir) = p.loc.get_untracked().dir().map(Path::to_path_buf) else { return };
    match ops::new_file(&dir, name, content) {
        Ok(path) => after_create(p, path),
        Err(e) => state::toast_error(e),
    }
}

fn after_create(p: Pane, path: PathBuf) {
    state::select_later(p, path.clone());
    p.renaming.set(Some(path.clone()));
    if let Some(d) = path.parent() {
        state::reload_dir(d);
    }
}

/// Шаблоны из `~/Templates` (XDG_TEMPLATES_DIR) для меню «Создать».
pub fn templates() -> Vec<PathBuf> {
    let dir = synshell_common::paths::user_dir_or_default("TEMPLATES");
    let mut v: Vec<PathBuf> =
        std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    v.sort();
    v
}

pub fn toggle_pin(path: &Path) {
    let cfg = state::ctx().cfg.get_untracked();
    let mut pinned = cfg.files.pinned.clone();
    let home = synshell_common::paths::home();
    let shown = match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    };
    let expanded: Vec<PathBuf> = pinned.iter().map(|p| synshell_common::paths::expand_tilde(p)).collect();
    if let Some(i) = expanded.iter().position(|p| p == path) {
        pinned.remove(i);
    } else {
        pinned.push(shown);
    }
    let arr: toml_edit::Array = pinned.iter().map(|s| s.as_str()).collect();
    if let Err(e) = synshell_common::config_edit::set_value(&["files", "pinned"], toml_edit::Value::Array(arr)) {
        state::toast_error(format!("config.toml: {e}"));
        return;
    }
    let mut c = (*cfg).clone();
    c.files.pinned = pinned;
    state::config_changed(c);
}

pub fn is_pinned(path: &Path) -> bool {
    state::ctx().cfg.get_untracked().files.pinned.iter().any(|p| synshell_common::paths::expand_tilde(p) == path)
}

/// Записать настройку `[files]` в config.toml.
pub fn save_setting(key: &str, v: impl Into<toml_edit::Value>) {
    if let Err(e) = synshell_common::config_edit::set_value(&["files", key], v.into()) {
        tracing::warn!("config.toml: {e}");
    }
}

// ---------------------------------------------------------------- вид

pub fn set_view(p: Pane, v: ViewMode) {
    p.view.set(v);
    save_setting("view", v.id());
}

pub fn set_sort(p: Pane, key: SortKey) {
    let mut s = p.sort.get_untracked();
    if s.key == key {
        s.descending = !s.descending;
    } else {
        s.key = key;
        s.descending = matches!(key, SortKey::Modified | SortKey::Size);
    }
    p.sort.set(s);
    state::refilter(p);
    save_setting("sort_by", key.id());
    save_setting("sort_descending", s.descending);
}

pub fn toggle_hidden() {
    let ctx = state::ctx();
    let v = !ctx.show_hidden.get_untracked();
    ctx.show_hidden.set(v);
    state::refilter_all();
    save_setting("show_hidden", v);
    state::toast(if v { "Скрытые файлы показаны" } else { "Скрытые файлы спрятаны" });
}

/// Ступени размера значков для Ctrl+±/колеса и кнопок строки состояния.
pub const ZOOM_STEPS: [u32; 7] = [32, 48, 64, 96, 128, 192, 256];

pub fn zoom(p: Pane, dir: i32) {
    let cur = p.icon_size.get_untracked();
    let n = if dir > 0 {
        ZOOM_STEPS.iter().copied().find(|&s| s > cur).unwrap_or(256)
    } else {
        ZOOM_STEPS.iter().rev().copied().find(|&s| s < cur).unwrap_or(32)
    };
    set_icon_size(p, n);
    let ctx = state::ctx();
    ctx.zoom_rev.update(|r| *r += 1);
}

/// Размер значков (ползунок строки состояния — любой, кратный 8). Вид
/// переключается на «Значки»; в config.toml пишется с задержкой, чтобы
/// перетаскивание ползунка не переписывало файл на каждом шаге.
pub fn set_icon_size(p: Pane, n: u32) {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    static PENDING: AtomicU32 = AtomicU32::new(0);
    static SCHEDULED: AtomicBool = AtomicBool::new(false);
    let n = n.clamp(32, 256);
    if p.icon_size.get_untracked() != n {
        p.icon_size.set(n);
    }
    if p.view.get_untracked() != ViewMode::Icons {
        p.view.set(ViewMode::Icons);
        save_setting("view", ViewMode::Icons.id());
    }
    PENDING.store(n, Ordering::Relaxed);
    if !SCHEDULED.swap(true, Ordering::AcqRel) {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(600));
            SCHEDULED.store(false, Ordering::Release);
            save_setting("icon_size", PENDING.load(Ordering::Relaxed) as i64);
        });
    }
}

pub fn select_all(p: Pane) {
    let n = p.entries.get_untracked().len();
    p.sel.set(ItemSelection::all(n));
}

pub fn invert_selection(p: Pane) {
    let n = p.entries.get_untracked().len();
    let s = p.sel.get_untracked();
    p.sel.set(ItemSelection { selected: (0..n).filter(|i| !s.is_selected(*i)).collect(), cursor: s.cursor, anchor: None });
}

/// Поиск в текущей папке.
pub fn search(p: Pane, query: &str) {
    let q = query.trim();
    let loc = p.loc.get_untracked();
    let root = match &loc {
        Location::Search { root, .. } => root.clone(),
        Location::Dir(d) => d.clone(),
        Location::Trash => return,
    };
    if q.is_empty() {
        if matches!(loc, Location::Search { .. }) {
            state::navigate(p, Location::Dir(root), false);
        }
        return;
    }
    let push = !matches!(loc, Location::Search { .. });
    state::navigate(p, Location::Search { root, query: q.to_string() }, push);
    p.loading.set(true);
}

// ---------------------------------------------------------------- меню

fn item(id: &str, label: &str, glyph: &str) -> MenuItem {
    MenuItem::new(id, label).icon(glyph)
}

fn check(id: &str, label: &str, on: bool) -> MenuItem {
    let m = MenuItem::new(id, label);
    if on {
        m.icon(icons::CHECK)
    } else {
        m
    }
}

pub fn view_menu(p: Pane) -> Vec<MenuItem> {
    let v = p.view.get_untracked();
    let ctx = state::ctx();
    vec![
        check("view:icons", "Значки", v == ViewMode::Icons).shortcut("Ctrl+1"),
        check("view:tiles", "Плитка", v == ViewMode::Tiles).shortcut("Ctrl+2"),
        check("view:list", "Список", v == ViewMode::List).shortcut("Ctrl+3"),
        check("view:details", "Таблица", v == ViewMode::Details).shortcut("Ctrl+4"),
        MenuItem::separator(),
        item("zoom:in", "Крупнее", icons::ZOOM_IN).shortcut("Ctrl++"),
        item("zoom:out", "Мельче", icons::ZOOM_OUT).shortcut("Ctrl+-"),
        MenuItem::separator(),
        check("hidden", "Скрытые файлы", ctx.show_hidden.get_untracked()).shortcut("Ctrl+H"),
        check("split", "Две панели", state::tab().split.get_untracked()).shortcut("F3"),
        check("sidebar", "Панель навигации", ctx.sidebar.get_untracked()).shortcut("F9"),
    ]
}

pub fn sort_menu(p: Pane) -> Vec<MenuItem> {
    let s = p.sort.get_untracked();
    let mut v: Vec<MenuItem> = [SortKey::Name, SortKey::Modified, SortKey::Type, SortKey::Size]
        .iter()
        .map(|k| check(&format!("sort:{}", k.id()), k.title(), s.key == *k))
        .collect();
    v.push(MenuItem::separator());
    v.push(check("sort:asc", "По возрастанию", !s.descending));
    v.push(check("sort:desc", "По убыванию", s.descending));
    v.push(MenuItem::separator());
    v.push(check("sort:folders", "Папки сверху", s.folders_first));
    v
}

pub fn new_menu() -> Vec<MenuItem> {
    let mut v = vec![
        item("new:folder", "Папку", icons::NEW_FOLDER).shortcut("Ctrl+Shift+N"),
        item("new:text", "Текстовый документ", icons::DOCUMENT),
        item("new:empty", "Пустой файл", icons::NEW_FILE),
    ];
    let t = templates();
    if !t.is_empty() {
        v.push(MenuItem::separator());
        for (i, p) in t.iter().enumerate().take(20) {
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            v.push(item(&format!("new:tpl:{i}"), &name, icons::FILE));
        }
    }
    v
}

/// Меню по правой кнопке: по выделенному (`on_item`) или по фону.
pub fn context_menu(p: Pane, on_item: bool) -> Vec<MenuItem> {
    let loc = p.loc.get_untracked();
    let in_trash = matches!(loc, Location::Trash);
    let sel = state::selected_entries(p);
    if !on_item || sel.is_empty() {
        let mut v = Vec::new();
        if in_trash {
            v.push(item("trash:empty", "Очистить корзину", icons::DELETE_FOREVER).disabled(crate::trash::is_empty()));
            v.push(MenuItem::separator());
        }
        v.push(MenuItem::new("menu:view", "Вид").icon(icons::VIEW_TILES).children(view_menu(p)));
        v.push(MenuItem::new("menu:sort", "Сортировка").icon(icons::SORT).children(sort_menu(p)));
        if loc.dir().is_some() {
            v.push(MenuItem::separator());
            v.push(MenuItem::new("menu:new", "Создать").icon(icons::ADD).children(new_menu()));
            v.push(item("paste", "Вставить", icons::PASTE).shortcut("Ctrl+V").disabled(!can_paste()));
            v.push(item("undo", ops::undo_title().as_deref().unwrap_or("Отменить"), icons::UNDO).shortcut("Ctrl+Z").disabled(ops::undo_title().is_none()));
            v.push(MenuItem::separator());
            v.push(item("terminal", "Открыть в терминале", icons::TERMINAL).shortcut("Shift+F4"));
            if let Some(d) = loc.dir() {
                v.push(item("pin-here", if is_pinned(d) { "Открепить от панели" } else { "Закрепить на панели" }, icons::PIN));
            }
            v.push(item("copy-path-here", "Копировать путь", icons::COPY_PATH));
        }
        v.push(MenuItem::separator());
        v.push(item("select-all", "Выделить всё", icons::SELECT_ALL).shortcut("Ctrl+A"));
        if loc.dir().is_some() {
            v.push(item("props-here", "Свойства", icons::INFO).shortcut("Alt+Enter"));
        }
        return v;
    }
    let one = sel.len() == 1;
    let first = &sel[0];
    let all_dirs = sel.iter().all(|e| e.is_dir);
    let mut v = Vec::new();
    if in_trash {
        v.push(item("trash:restore", "Восстановить", icons::RESTORE));
        v.push(item("delete", "Удалить навсегда", icons::DELETE_FOREVER).shortcut("Delete"));
        v.push(MenuItem::separator());
        v.push(item("props", "Свойства", icons::INFO).shortcut("Alt+Enter"));
        return v;
    }
    v.push(item("open", "Открыть", icons::OPEN).shortcut("Enter"));
    if all_dirs {
        v.push(item("open-tab", "Открыть в новой вкладке", icons::TAB));
        if one {
            v.push(item("open-terminal", "Открыть в терминале", icons::TERMINAL));
        }
    } else if sel.iter().all(|e| e.mime == first.mime) {
        let apps = mime::apps_for(&first.mime);
        let mut sub: Vec<MenuItem> = apps
            .iter()
            .filter(|a| a.takes_files())
            .take(10)
            .enumerate()
            .map(|(i, a)| {
                let m = MenuItem::new(format!("with:{i}"), a.name.clone());
                m
            })
            .collect();
        if !sub.is_empty() {
            sub.push(MenuItem::separator());
        }
        sub.push(item("with:other", "Выбрать другую программу…", icons::OPEN_WITH));
        v.push(MenuItem::new("menu:with", "Открыть с помощью").icon(icons::OPEN_WITH).children(sub));
    }
    v.push(MenuItem::separator());
    v.push(item("cut", "Вырезать", icons::CUT).shortcut("Ctrl+X"));
    v.push(item("copy", "Копировать", icons::COPY).shortcut("Ctrl+C"));
    if one && first.is_dir {
        v.push(item("paste-into", "Вставить в папку", icons::PASTE).disabled(!can_paste()));
    }
    v.push(item("copy-path", if one { "Копировать путь" } else { "Копировать пути" }, icons::COPY_PATH).shortcut("Ctrl+Shift+C"));
    v.push(item("link", "Создать ссылку", icons::LINK));
    v.push(MenuItem::separator());
    if one {
        v.push(item("rename", "Переименовать", icons::RENAME).shortcut("F2"));
    }
    v.push(item("trash", "Удалить в корзину", icons::DELETE).shortcut("Delete"));
    v.push(item("delete", "Удалить навсегда", icons::DELETE_FOREVER).shortcut("Shift+Delete"));
    if one && first.is_dir {
        v.push(MenuItem::separator());
        v.push(item("pin", if is_pinned(&first.path) { "Открепить от панели" } else { "Закрепить на панели" }, icons::PIN));
    }
    v.push(MenuItem::separator());
    v.push(item("props", "Свойства", icons::INFO).shortcut("Alt+Enter"));
    v
}

/// Выполнить пункт меню или команду панели.
pub fn run(p: Pane, id: &str) {
    let ctx = state::ctx();
    let loc = p.loc.get_untracked();
    let dir = loc.dir().map(Path::to_path_buf);
    let sel = state::selected_entries(p);
    let paths: Vec<PathBuf> = sel.iter().map(|e| e.path.clone()).collect();
    match id {
        "open" => open_selected(p),
        "open-tab" => {
            for e in sel.iter().filter(|e| e.is_dir) {
                state::new_tab(Location::Dir(e.path.clone()), false);
            }
        }
        "open-terminal" => {
            if let Some(e) = sel.first() {
                open_terminal(&e.path);
            }
        }
        "terminal" => {
            if let Some(d) = &dir {
                open_terminal(d);
            }
        }
        "cut" => set_clipboard(paths, true),
        "copy" => set_clipboard(paths, false),
        "paste" => {
            if let Some(d) = &dir {
                paste_into(d);
            }
        }
        "paste-into" => {
            if let Some(e) = sel.first() {
                paste_into(&e.path);
            }
        }
        "copy-path" => copy_paths_text(&paths),
        "copy-path-here" => {
            if let Some(d) = &dir {
                copy_paths_text(std::slice::from_ref(d));
            }
        }
        "link" => {
            if let Some(d) = &dir {
                ops::start(Op::Link { srcs: paths, dest: d.clone() });
            }
        }
        "rename" => start_rename(p),
        "trash" => trash_selected(p),
        "delete" => delete_selected(p),
        "undo" => undo(),
        "pin" => {
            if let Some(e) = sel.first() {
                toggle_pin(&e.path);
            }
        }
        "pin-here" => {
            if let Some(d) = &dir {
                toggle_pin(d);
            }
        }
        "props" => ctx.dialog.set(Some(Dialog::Properties { paths })),
        "props-here" => {
            if let Some(d) = dir {
                ctx.dialog.set(Some(Dialog::Properties { paths: vec![d] }));
            }
        }
        "select-all" => select_all(p),
        "invert-selection" => invert_selection(p),
        "select-none" => p.sel.set(ItemSelection::default()),
        "trash:empty" => ctx.dialog.set(Some(Dialog::ConfirmEmptyTrash)),
        "trash:restore" => {
            ops::start(Op::Restore { files: paths });
        }
        "new:folder" => new_folder(p),
        "new:text" => new_file(p, "Новый документ.txt", b""),
        "new:empty" => new_file(p, "Новый файл", b""),
        "hidden" => toggle_hidden(),
        "split" => state::toggle_split(),
        "sidebar" => ctx.sidebar.update(|v| *v = !*v),
        "zoom:in" => zoom(p, 1),
        "zoom:out" => zoom(p, -1),
        "sort:asc" | "sort:desc" => {
            let mut s = p.sort.get_untracked();
            s.descending = id == "sort:desc";
            p.sort.set(s);
            state::refilter(p);
            save_setting("sort_descending", s.descending);
        }
        "sort:folders" => {
            let mut s = p.sort.get_untracked();
            s.folders_first = !s.folders_first;
            p.sort.set(s);
            state::refilter(p);
            save_setting("folders_first", s.folders_first);
        }
        "with:other" => {
            if let Some(f) = sel.first() {
                ctx.dialog.set(Some(Dialog::OpenWith { paths, mime: f.mime.clone() }));
            }
        }
        "refresh" => state::load(p, Some(state::selected_paths(p))),
        "back" => state::go_back(p),
        "forward" => state::go_forward(p),
        "up" => state::go_up(p),
        "home" => state::navigate(p, Location::Dir(synshell_common::paths::home()), true),
        _ => {
            if let Some(v) = id.strip_prefix("view:") {
                set_view(p, ViewMode::parse(v));
            } else if let Some(k) = id.strip_prefix("sort:") {
                let key = SortKey::parse(k);
                let mut s = p.sort.get_untracked();
                s.key = key;
                p.sort.set(s);
                state::refilter(p);
                save_setting("sort_by", key.id());
            } else if let Some(i) = id.strip_prefix("new:tpl:").and_then(|i| i.parse::<usize>().ok()) {
                if let Some(t) = templates().get(i) {
                    let content = std::fs::read(t).unwrap_or_default();
                    new_file(p, &ops::name_of(t), &content);
                }
            } else if let Some(i) = id.strip_prefix("with:").and_then(|i| i.parse::<usize>().ok()) {
                if let Some(f) = sel.first() {
                    let apps: Vec<_> = mime::apps_for(&f.mime).into_iter().filter(|a| a.takes_files()).take(10).collect();
                    if let Some(a) = apps.get(i) {
                        launch(a, &paths);
                    }
                }
            }
        }
    }
}

/// Показать меню `items` в точке и выполнять выбранное для панели `p`.
pub fn popup(p: Pane, items: Vec<MenuItem>, at: Point) {
    state::show_menu(items, at, move |id| run(p, id));
}

// ---------------------------------------------------------------- клавиатура

/// Сочетания окна. `true` — обработано.
pub fn key(k: Key, m: Modifiers) -> bool {
    let ctx = state::ctx();
    let p = state::pane();
    let ctrl = m.ctrl && !m.alt;
    match k {
        Key::T if ctrl => {
            let loc = p.loc.get_untracked();
            state::new_tab(loc, true);
        }
        Key::W if ctrl => state::close_tab(ctx.cur.get_untracked()),
        Key::Tab if m.ctrl => {
            let n = ctx.tabs.get_untracked().len();
            let c = ctx.cur.get_untracked();
            ctx.cur.set(if m.shift { (c + n - 1) % n } else { (c + 1) % n });
        }
        Key::N if ctrl && m.shift => new_folder(p),
        Key::N if ctrl => {
            let dir = p.loc.get_untracked().address();
            if let Ok(exe) = std::env::current_exe() {
                spawn_shell(&format!("'{}' '{}'", exe.display(), dir.replace('\'', "'\\''")), None);
            }
        }
        Key::C if ctrl && m.shift => copy_paths_text(&state::selected_paths(p)),
        Key::C if ctrl => set_clipboard(state::selected_paths(p), false),
        Key::X if ctrl => set_clipboard(state::selected_paths(p), true),
        Key::V if ctrl => run(p, "paste"),
        Key::Z if ctrl => undo(),
        Key::H if ctrl => toggle_hidden(),
        Key::L if ctrl => ctx.address_edit.set(true),
        Key::D if m.alt => ctx.address_edit.set(true),
        Key::F if ctrl => ctx.search_focus.update(|v| *v += 1),
        Key::I if ctrl && m.shift => invert_selection(p),
        Key::Num1 if ctrl => set_view(p, ViewMode::Icons),
        Key::Num2 if ctrl => set_view(p, ViewMode::Tiles),
        Key::Num3 if ctrl => set_view(p, ViewMode::List),
        Key::Num4 if ctrl => set_view(p, ViewMode::Details),
        Key::F2 => start_rename(p),
        Key::F3 => state::toggle_split(),
        Key::F4 if m.shift => run(p, "terminal"),
        Key::F4 => ctx.address_edit.set(true),
        Key::F5 => run(p, "refresh"),
        Key::F9 => ctx.sidebar.update(|v| *v = !*v),
        Key::Delete if m.shift => delete_selected(p),
        Key::Delete => trash_selected(p),
        Key::Left if m.alt => state::go_back(p),
        Key::Right if m.alt => state::go_forward(p),
        Key::Up if m.alt => state::go_up(p),
        Key::Home if m.alt => run(p, "home"),
        Key::Enter if m.alt => run(p, "props"),
        Key::Backspace if !m.ctrl && !m.alt => state::go_back(p),
        _ => return false,
    }
    true
}

/// Ctrl + колесо / Ctrl+= и Ctrl+- приходят символами.
pub fn char_key(c: char, m: Modifiers) -> bool {
    if !m.ctrl {
        return false;
    }
    let p = state::pane();
    match c {
        '+' | '=' => zoom(p, 1),
        '-' => zoom(p, -1),
        _ => return false,
    }
    true
}
