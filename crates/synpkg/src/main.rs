//! synpkg — «Программы»: каталог (обзор, категории, снимки экрана), поиск,
//! AUR, установка, удаление и обновление программ из репозиториев pacman и
//! AUR. Одна программа для рабочего стола (три колонки: разделы, список,
//! подробности) и телефона (стек «список → пакет» с нижней навигацией).
//! Бэкенд — `synsystem::packages`.
//!
//! Изменения не выполняются сразу: «Установить», «Удалить», «Обновить» и
//! флажки собирают очередь ([`queue`]: задачи сливаются, противоположные
//! гасят друг друга), «Применить» выполняет её одним заданием (удаление →
//! транзакция pacman → AUR). Мешают зависимости при удалении — спрашивается,
//! удалить ли каскадом или принудительно.
//!
//! `synpkg [запрос]` — открыть поиск с запросом.

mod desk;
mod media;
mod queue;
mod shot;
mod ui;
#[cfg(test)]
mod ui_tests;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use synshell_common::Config;
use synsystem::packages::{self as pk, CatalogApp, Details, JobEvent, Op, Pkg, RemoveCheck, RemoveMode, Source, Update};
use synsystem::polkit_agent::{self, AuthRequest, Prompter};
use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::MenuItem;

use media::Media;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Explore,
    Aur,
    Installed,
    Updates,
    Jobs,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Explore, Tab::Aur, Tab::Installed, Tab::Updates, Tab::Jobs];
    pub fn label(self) -> &'static str {
        match self {
            Tab::Explore => "Обзор",
            Tab::Aur => "AUR",
            Tab::Installed => "Установленные",
            Tab::Updates => "Обновления",
            Tab::Jobs => "Задачи",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Tab::Explore => "\u{E87A}",
            Tab::Aur => "\u{E869}",
            Tab::Installed => "\u{E1DB}",
            Tab::Updates => "\u{E923}",
            Tab::Jobs => "\u{E8B5}",
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct JobView {
    pub id: u64,
    pub title: String,
    pub stage: String,
    pub log: Vec<String>,
    pub done: Option<std::result::Result<(), String>>,
    pub cancel: pk::JobCancel,
    /// Нажата «Отменить», задание ещё не закончилось.
    pub cancelling: bool,
}

pub use queue::{Merged, Plan, QAct, QItem};

/// Удаление упёрлось в зависимости: что спросить и что выполнить потом.
#[derive(Clone, PartialEq)]
pub struct RemoveAsk {
    pub names: Vec<String>,
    pub check: RemoveCheck,
    /// Остальные шаги (установка из очереди) — после удаления.
    pub then: Vec<Op>,
    pub title: String,
    /// Задачи очереди, которые выполнит задание: после запуска снимаются с очереди.
    pub from_queue: Vec<QItem>,
}

/// Разделы каталога: ключ — главная категория freedesktop (и её синонимы).
pub struct Category {
    pub key: &'static str,
    pub also: &'static [&'static str],
    pub label: &'static str,
    pub icon: &'static str,
}

pub const CATEGORIES: &[Category] = &[
    Category { key: "Network", also: &[], label: "Интернет", icon: "\u{E80B}" },
    Category { key: "Office", also: &[], label: "Офис", icon: "\u{E873}" },
    Category { key: "Graphics", also: &[], label: "Графика", icon: "\u{E3F4}" },
    Category { key: "AudioVideo", also: &["Audio", "Video"], label: "Мультимедиа", icon: "\u{E02C}" },
    Category { key: "Game", also: &[], label: "Игры", icon: "\u{EA28}" },
    Category { key: "Development", also: &[], label: "Разработка", icon: "\u{E86F}" },
    Category { key: "Education", also: &[], label: "Образование", icon: "\u{E80C}" },
    Category { key: "Science", also: &[], label: "Наука", icon: "\u{EA4B}" },
    Category { key: "System", also: &["Settings"], label: "Система", icon: "\u{E8B8}" },
    Category { key: "Utility", also: &[], label: "Утилиты", icon: "\u{E869}" },
];

impl Category {
    pub fn by_key(key: &str) -> Option<&'static Category> {
        CATEGORIES.iter().find(|c| c.key == key)
    }
    pub fn contains(&self, a: &CatalogApp) -> bool {
        a.categories.iter().any(|c| c == self.key || self.also.contains(&c.as_str()))
    }
}

/// Раздел каталога «Метапакеты и группы» (ключ вместо категории в [`St::category`]).
pub const META_KEY: &str = "@meta";

/// Описания известных групп pacman.
pub const GROUP_INFO: &[(&str, &str)] = &[
    ("gnome", "Рабочая среда GNOME"),
    ("gnome-extra", "Дополнительные программы GNOME"),
    ("gnome-circle", "Программы сообщества GNOME Circle"),
    ("plasma", "Рабочая среда KDE Plasma"),
    ("kde-applications", "Все программы KDE"),
    ("xfce4", "Рабочая среда Xfce"),
    ("xfce4-goodies", "Дополнения и модули Xfce"),
    ("lxqt", "Лёгкая рабочая среда LXQt"),
    ("lxde", "Лёгкая рабочая среда LXDE"),
    ("mate", "Рабочая среда MATE"),
    ("mate-extra", "Дополнительные программы MATE"),
    ("budgie", "Рабочая среда Budgie"),
    ("cosmic", "Рабочая среда COSMIC"),
    ("deepin", "Рабочая среда Deepin"),
    ("deepin-extra", "Дополнительные программы Deepin"),
    ("pantheon", "Рабочая среда Pantheon (elementary OS)"),
    ("ukui", "Рабочая среда UKUI"),
    ("i3", "Мозаичный оконный менеджер i3"),
    ("xorg", "Графический сервер X.Org целиком"),
    ("xorg-apps", "Утилиты X.Org"),
    ("xorg-drivers", "Драйверы X.Org"),
    ("xorg-fonts", "Шрифты X.Org"),
    ("pro-audio", "Профессиональная работа со звуком"),
    ("texlive", "Вёрстка TeX Live"),
    ("texlive-lang", "Языки TeX Live"),
    ("nerd-fonts", "Шрифты Nerd Fonts со значками"),
    ("vulkan-devel", "Разработка под Vulkan"),
    ("qt6", "Библиотеки Qt 6"),
    ("qt5", "Библиотеки Qt 5"),
    ("kf6", "KDE Frameworks 6"),
    ("libretro", "Эмуляторы libretro"),
    ("mingw-w64", "Кросс-компиляция под Windows"),
];

/// Известные программы для витрины «Обзора» (показываются те, что есть в каталоге).
pub const FEATURED: &[&str] = &[
    "firefox", "thunderbird", "libreoffice-fresh", "gimp", "inkscape", "krita", "blender", "obs-studio", "kdenlive", "vlc", "mpv",
    "audacity", "telegram-desktop", "steam", "keepassxc", "qbittorrent", "chromium", "darktable", "shotcut", "okular", "gwenview",
    "kate", "freecad", "signal-desktop", "element-desktop", "transmission-qt", "digikam", "handbrake", "ardour", "godot",
];

/// Открытое окно пароля polkit (агент — [`polkit_agent`]).
#[derive(Clone)]
pub struct AuthView(pub Arc<AuthRequest>);

impl PartialEq for AuthView {
    fn eq(&self, o: &Self) -> bool {
        self.0.id == o.0.id
    }
}

/// Окно пароля для pkexec: запросы агента приходят из его потока — в сигнал через главный поток.
struct AuthPrompter(RwSignal<Option<AuthView>>);

impl Prompter for AuthPrompter {
    fn ask(&self, req: AuthRequest) {
        let s = self.0;
        let v = AuthView(Arc::new(req));
        run_on_main_thread(move || s.set(Some(v)));
    }
    fn cancel(&self, id: u64) {
        let s = self.0;
        run_on_main_thread(move || {
            if s.get_untracked().is_some_and(|a| a.0.id == id) {
                s.set(None);
            }
        });
    }
}

/// Фильтр «Установленных».
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InstFilter {
    All,
    Apps,
    Foreign,
    Meta,
}

#[derive(Clone, Copy)]
pub struct St {
    pub tab: RwSignal<Tab>,
    pub query: RwSignal<String>,
    pub results: RwSignal<Vec<Pkg>>,
    pub searching: RwSignal<bool>,
    pub aur_query: RwSignal<String>,
    pub aur_results: RwSignal<Vec<Pkg>>,
    pub aur_searching: RwSignal<bool>,
    /// Обновления пакетов AUR (`None` — не проверялись или проверяются).
    pub aur_updates: RwSignal<Option<Vec<Update>>>,
    pub aur_checking: RwSignal<bool>,
    pub installed: RwSignal<Vec<Pkg>>,
    pub filter: RwSignal<String>,
    pub inst_filter: RwSignal<InstFilter>,
    pub updates: RwSignal<Option<Vec<Update>>>,
    pub selected: RwSignal<Option<Pkg>>,
    pub details: RwSignal<Option<Details>>,
    pub media: RwSignal<Option<Media>>,
    /// Открытый на весь экран снимок.
    pub shot: RwSignal<Option<usize>>,
    pub pkgbuild: RwSignal<Option<String>>,
    pub jobs: RwSignal<Vec<JobView>>,
    /// Задание, чей лог развёрнут в «Задачах» (`None` — последнее).
    pub job_open: RwSignal<Option<u64>>,
    /// Каталог AppStream (`None` — ещё загружается).
    pub catalog: RwSignal<Option<Vec<CatalogApp>>>,
    /// Открытая категория каталога (ключ [`Category`]).
    pub category: RwSignal<Option<&'static str>>,
    pub toast: RwSignal<String>,
    pub cfg: RwSignal<Config>,
    pub auth: RwSignal<Option<AuthView>>,
    pub queue: RwSignal<Vec<QItem>>,
    pub remove_ask: RwSignal<Option<RemoveAsk>>,
    pub menu_items: RwSignal<Vec<MenuItem>>,
    pub menu_open: RwSignal<bool>,
    pub menu_pos: RwSignal<syngui::core::Point>,
    /// Растёт, когда перекодированы значки каталога.
    pub icons_rev: RwSignal<u64>,
    /// Состояние окна (развёрнуто, в фокусе) — для своего заголовка.
    pub window: RwSignal<syngui::window::WindowState>,
    /// Растёт, когда поле поиска в заголовке надо пересоздать (запрос сброшен из кода).
    pub search_rev: RwSignal<u64>,
    /// Подробности: список файлов развёрнут.
    pub files_open: RwSignal<bool>,
    /// Списки пакетов карточками (иначе строками) — `packages.view`.
    pub cards: RwSignal<bool>,
    /// Метапакеты репозиториев (`None` — ещё загружаются).
    pub metas: RwSignal<Option<Vec<Pkg>>>,
    /// Группы пакетов pacman.
    pub groups: RwSignal<Vec<pk::Group>>,
    /// Группа, чей состав развёрнут.
    pub group_open: RwSignal<Option<String>>,
    /// «Метапакеты и группы»: показаны группы (иначе метапакеты).
    pub show_groups: RwSignal<bool>,
}

static SEARCH_GEN: AtomicU64 = AtomicU64::new(0);
static AUR_GEN: AtomicU64 = AtomicU64::new(0);
static JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Сведения каталога по имени пакета: название, значок — для строк любого списка.
#[derive(Clone)]
pub struct AppMeta {
    pub title: String,
    pub icon: Option<String>,
    pub icon64: Option<PathBuf>,
    pub icon128: Option<PathBuf>,
}

thread_local! {
    static CAT_INDEX: RefCell<HashMap<String, AppMeta>> = RefCell::new(HashMap::new());
    /// Действие выбранного пункта контекстного меню.
    static MENU_ACTION: RefCell<Option<Box<dyn Fn(&str)>>> = const { RefCell::new(None) };
}

pub fn app_meta(name: &str) -> Option<AppMeta> {
    CAT_INDEX.with(|m| m.borrow().get(name).cloned())
}

/// Открыть контекстное меню в точке окна; `f` получит id пункта.
pub fn show_menu(st: St, items: Vec<MenuItem>, at: syngui::core::Point, f: impl Fn(&str) + 'static) {
    MENU_ACTION.with(|m| *m.borrow_mut() = Some(Box::new(f)));
    st.menu_items.set_always(items);
    st.menu_pos.set(at);
    st.menu_open.set(true);
}

pub fn menu_selected(id: &str) {
    let f = MENU_ACTION.with(|m| m.borrow_mut().take());
    if let Some(f) = f {
        f(id);
    }
}

/// Значение ключа командной строки (`--size 1240x820`).
pub fn arg_value(key: &str) -> Option<String> {
    let mut it = std::env::args().skip_while(|a| a != key);
    it.next()?;
    it.next()
}

/// Состояние приложения (сигналы) — для окна и снимка без окна.
pub fn new_state(cfg: Config, query: String, window: RwSignal<syngui::window::WindowState>) -> St {
    let cards = cfg.packages.view != "list";
    St {
        tab: use_signal(Tab::Explore),
        query: use_signal(query),
        results: use_signal(Vec::new()),
        searching: use_signal(false),
        aur_query: use_signal(String::new()),
        aur_results: use_signal(Vec::new()),
        aur_searching: use_signal(false),
        aur_updates: use_signal(None),
        aur_checking: use_signal(false),
        installed: use_signal(Vec::new()),
        filter: use_signal(String::new()),
        inst_filter: use_signal(InstFilter::All),
        updates: use_signal(None),
        selected: use_signal(None),
        details: use_signal(None),
        media: use_signal(None),
        shot: use_signal(None),
        pkgbuild: use_signal(None),
        jobs: use_signal(Vec::new()),
        job_open: use_signal(None),
        catalog: use_signal(None),
        category: use_signal(None),
        toast: use_signal(String::new()),
        cfg: use_signal(cfg),
        auth: use_signal(None),
        queue: use_signal(Vec::new()),
        remove_ask: use_signal(None),
        menu_items: use_signal(Vec::new()),
        menu_open: use_signal(false),
        menu_pos: use_signal(syngui::core::Point::zero()),
        icons_rev: use_signal(0),
        window,
        search_rev: use_signal(0),
        files_open: use_signal(false),
        cards: use_signal(cards),
        metas: use_signal(None),
        groups: use_signal(Vec::new()),
        group_open: use_signal(None),
        show_groups: use_signal(false),
    }
}

fn main() {
    if let Some(out) = arg_value("--screenshot") {
        if let Err(e) = shot::screenshot(&out) {
            eprintln!("synpkg: снимок: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    let query = std::env::args().skip(1).find(|a| !a.starts_with('-')).unwrap_or_default();
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let window = use_signal(syngui::window::WindowState::default());
    App::new()
        .title("Программы")
        .app_id("synpkg")
        .size(1240, 820)
        .min_size(340, 480)
        .frameless()
        // Углы за скруглением рамки (`.window-frame`) — прозрачные.
        .transparent(true)
        .background(syngui::core::Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(window)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = new_state(cfg.clone(), query.clone(), window);
            polkit_agent::set_prompter(AuthPrompter(st.auth));
            if !query.is_empty() {
                search(st, query.clone());
            }
            load_installed(st);
            load_catalog(st);
            load_metas(st);
            ui::root(st)
        });
}

pub fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    // Производные цвета полей и кнопок — как в «Параметрах», чтобы окна были одного вида.
    let p = a.palette();
    let dark = a.is_dark();
    s.push_str(&format!(
        ":root {{\n  --input-bg: {};\n  --accent-hover: {};\n  --danger-soft: {};\n  --titlebar: {};\n  --sidebar: {};\n  --content: {};\n  --window-radius: {}px;\n}}\n",
        if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex(),
        p.accent.mix(p.fg, 0.15).hex(),
        p.danger.with_alpha(0.16).hex(),
        if dark { p.bg.mix(synshell_common::config::Rgba::rgb(0, 0, 0), 0.25) } else { p.surface_alt.mix(p.bg, 0.5) }.hex(),
        if dark { p.bg.mix(synshell_common::config::Rgba::rgb(0, 0, 0), 0.12) } else { p.surface_alt.mix(p.bg, 0.7) }.hex(),
        if dark { p.bg.mix(p.surface, 0.2) } else { p.bg.mix(p.surface, 0.35) }.hex(),
        cfg.decorations.corner_radius.max(0.0),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synpkg.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

// ─── Фоновые запросы ─────────────────────────────────────────────────────────

pub fn search(st: St, q: String) {
    let gen = SEARCH_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let aur = st.cfg.get_untracked().packages.aur;
    if q.trim().len() < 2 {
        st.results.set(Vec::new());
        st.searching.set(false);
        return;
    }
    st.searching.set(true);
    std::thread::spawn(move || {
        // Набор ещё идёт — подождать паузы.
        std::thread::sleep(Duration::from_millis(350));
        if SEARCH_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let r = pk::search(&q, aur);
        run_on_main_thread(move || {
            if SEARCH_GEN.load(Ordering::SeqCst) == gen {
                let r = mark_metas(st, r);
                st.results.set(r);
                st.searching.set(false);
            }
        });
    });
}

pub fn search_aur(st: St, q: String) {
    let gen = AUR_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if q.trim().len() < 2 {
        st.aur_results.set(Vec::new());
        st.aur_searching.set(false);
        return;
    }
    st.aur_searching.set(true);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(350));
        if AUR_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let r = pk::search_aur(&q);
        run_on_main_thread(move || {
            if AUR_GEN.load(Ordering::SeqCst) == gen {
                st.aur_results.set(r);
                st.aur_searching.set(false);
            }
        });
    });
}

pub fn load_installed(st: St) {
    std::thread::spawn(move || {
        let v = pk::installed();
        run_on_main_thread(move || st.installed.set(v));
    });
}

/// Метапакеты и группы репозиториев.
pub fn load_metas(st: St) {
    std::thread::spawn(move || {
        let (m, g) = (pk::metapackages(), pk::groups());
        run_on_main_thread(move || {
            st.metas.set(Some(m));
            st.groups.set(g);
        });
    });
}

/// Пометка «метапакет» у найденных (`pacman -Ss` размеров не знает).
pub fn mark_metas(st: St, mut v: Vec<Pkg>) -> Vec<Pkg> {
    if let Some(m) = st.metas.get_untracked() {
        for p in &mut v {
            p.meta = p.meta || m.iter().any(|x| x.name == p.name && p.source != Source::Aur);
        }
    }
    v
}

fn load_catalog(st: St) {
    std::thread::spawn(move || {
        let v = pk::catalog();
        run_on_main_thread(move || set_catalog(st, v));
        // Значки каталога (JPEG XL) — в PNG; готово — строки перерисуются со значками.
        if media::convert_catalog_icons() {
            run_on_main_thread(move || st.icons_rev.set(st.icons_rev.get_untracked() + 1));
        }
    });
}

/// Каталог загружен: сведения по именам для строк и карточек.
pub fn set_catalog(st: St, v: Vec<CatalogApp>) {
    CAT_INDEX.with(|m| {
        let mut m = m.borrow_mut();
        for a in &v {
            m.insert(a.pkg.name.clone(), AppMeta { title: a.title.clone(), icon: a.icon.clone(), icon64: a.cached_icon(64).cloned(), icon128: a.cached_icon(128).cloned() });
        }
    });
    st.catalog.set(Some(v));
    st.icons_rev.set(st.icons_rev.get_untracked() + 1);
}

/// Пометки «установлен» в каталоге после изменений.
fn refresh_catalog_installed(st: St) {
    std::thread::spawn(move || {
        let map = pk::installed_map();
        run_on_main_thread(move || {
            if let Some(mut v) = st.catalog.get_untracked() {
                for a in &mut v {
                    a.pkg.installed = map.get(&a.pkg.name).cloned();
                }
                st.catalog.set(Some(v));
            }
        });
    });
}

pub fn check_updates(st: St) {
    st.updates.set(None);
    let aur = st.cfg.get_untracked().packages.aur;
    std::thread::spawn(move || {
        let v = pk::updates(aur);
        run_on_main_thread(move || st.updates.set(Some(v)));
    });
}

pub fn check_aur_updates(st: St) {
    if st.aur_checking.get_untracked() {
        return;
    }
    st.aur_checking.set(true);
    std::thread::spawn(move || {
        let v = pk::aur_updates();
        run_on_main_thread(move || {
            st.aur_updates.set(Some(v));
            st.aur_checking.set(false);
        });
    });
}

pub fn select(st: St, p: Pkg) {
    st.details.set(None);
    st.pkgbuild.set(None);
    st.shot.set(None);
    st.files_open.set(false);
    let same = st.media.get_untracked().is_some_and(|m| m.name == p.name);
    if !same {
        st.media.set(None);
    }
    st.selected.set(Some(p.clone()));
    let p2 = p.clone();
    std::thread::spawn(move || {
        let d = pk::details(&p2);
        run_on_main_thread(move || {
            if st.selected.get_untracked().is_some_and(|s| s.name == p2.name) {
                st.details.set(Some(d));
            }
        });
    });
    if same {
        return;
    }
    // Снимки экрана и описание: каталог AppStream, иначе Flathub.
    let app = st.catalog.get_untracked().and_then(|c| c.into_iter().find(|a| a.pkg.name == p.name));
    let known = media::Known {
        name: p.name.clone(),
        title: app.as_ref().map(|a| a.title.clone()).unwrap_or_else(|| p.name.clone()),
        appstream_id: app.as_ref().map(|a| a.id.clone()).unwrap_or_default(),
        screenshots: app.as_ref().map(|a| a.screenshots.clone()).unwrap_or_default(),
        has_description: app.as_ref().is_some_and(|a| !a.description.is_empty()),
    };
    let icon128 = app_meta(&p.name).and_then(|m| m.icon128);
    std::thread::spawn(move || {
        let big = icon128.and_then(|j| media::icon_png_now(&j));
        let name = p.name.clone();
        media::load(known, move |mut m| {
            if m.icon.is_none() {
                m.icon = big.clone();
            }
            let name = name.clone();
            run_on_main_thread(move || {
                if st.selected.get_untracked().is_some_and(|s| s.name == name) {
                    st.media.set(Some(m));
                }
            });
        });
    });
}

pub fn show_pkgbuild(st: St, name: String) {
    std::thread::spawn(move || {
        let url = format!("https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h={name}");
        let text = std::process::Command::new("curl")
            .args(["-sfL", "--max-time", "20", &url])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_else(|| "Не удалось загрузить PKGBUILD".into());
        run_on_main_thread(move || st.pkgbuild.set(Some(text)));
    });
}

// ─── Очередь ─────────────────────────────────────────────────────────────────

/// Действие очереди для пакета: установленный — удалить, иначе установить.
pub fn default_act(p: &Pkg) -> QAct {
    if p.installed.is_some() {
        QAct::Remove
    } else if p.source == Source::Aur {
        QAct::InstallAur
    } else {
        QAct::Install
    }
}

/// Задача обновления: из репозитория или пересборка из AUR.
pub fn upgrade_act(u: &Update) -> QAct {
    if u.source == Source::Aur {
        QAct::UpgradeAur
    } else {
        QAct::Upgrade
    }
}

pub fn queued(st: St, name: &str) -> Option<QAct> {
    st.queue.get_untracked().iter().find(|q| q.name == name).map(|q| q.act)
}

/// Добавить задачу (слияние — [`queue::merge`]).
pub fn enqueue(st: St, it: QItem) -> Merged {
    let mut q = st.queue.get_untracked();
    let r = queue::merge(&mut q, it);
    if r != Merged::Same {
        st.queue.set(q);
    }
    r
}

/// Флажок: пакет в очереди — снять, иначе поставить с действием `act`.
pub fn toggle_act(st: St, name: &str, act: QAct) {
    if queued(st, name).is_some() {
        unqueue(st, name);
    } else {
        enqueue(st, QItem::new(name, act));
    }
}

/// Поставить пакет в очередь или убрать из неё.
pub fn toggle_queue(st: St, p: &Pkg) {
    toggle_act(st, &p.name, default_act(p));
}

pub fn set_queued(st: St, items: impl IntoIterator<Item = QItem>) {
    let mut q = st.queue.get_untracked();
    for it in items {
        queue::merge(&mut q, it);
    }
    st.queue.set(q);
}

pub fn unqueue(st: St, name: &str) {
    let mut q = st.queue.get_untracked();
    q.retain(|x| x.name != name);
    st.queue.set(q);
}

/// Снять с очереди выполненные задачи (добавленные после запуска остаются).
fn take_from_queue(st: St, done: &[QItem]) {
    let mut q = st.queue.get_untracked();
    q.retain(|x| !done.contains(x));
    st.queue.set(q);
}

/// «1 пакет», «3 пакета», «5 пакетов».
pub fn packages_word(n: usize) -> String {
    let w = match (n % 10, n % 100) {
        (1, r) if r != 11 => "пакет",
        (2..=4, r) if !(12..=14).contains(&r) => "пакета",
        _ => "пакетов",
    };
    format!("{n} {w}")
}

/// Выполнить очередь одним заданием: удаление (с проверкой зависимостей),
/// транзакция pacman, проход AUR ([`Plan::steps`]). Задачи, добавленные, пока
/// задание идёт, ждут следующего «Применить».
pub fn apply_queue(st: St) {
    let q = st.queue.get_untracked();
    let plan = Plan::of(&q);
    if plan.is_empty() {
        return;
    }
    // pacman — одна транзакция за раз: очередь ждёт, пока идёт задание.
    if let Some(j) = st.jobs.get_untracked().iter().find(|j| j.done.is_none()) {
        st.toast.set(format!("Идёт «{}» — очередь ждёт, примените её после окончания", j.title));
        return;
    }
    let updates: Vec<String> = st.updates.get_untracked().unwrap_or_default().into_iter().filter(|u| u.source != Source::Aur).map(|u| u.name).collect();
    let then = plan.steps(&updates, &st.cfg.get_untracked().packages.ignore);
    let title = plan.title();
    if plan.remove.is_empty() {
        take_from_queue(st, &q);
        start_ops(st, title, then);
        return;
    }
    remove_checked(st, plan.remove, then, title, q);
}

/// Удалить `names` (и потом выполнить `then`): сначала проверка, нужны ли
/// они другим пакетам; если да — окно выбора (каскадом / принудительно).
pub fn remove_checked(st: St, names: Vec<String>, then: Vec<Op>, title: String, from_queue: Vec<QItem>) {
    st.toast.set("Проверка зависимостей…".into());
    std::thread::spawn(move || {
        let check = pk::remove_check(&names);
        run_on_main_thread(move || {
            if check.blockers.is_empty() {
                st.toast.set(String::new());
                run_remove(st, RemoveAsk { names, check, then, title, from_queue }, RemoveMode::Normal);
            } else {
                st.toast.set(String::new());
                st.remove_ask.set(Some(RemoveAsk { names, check, then, title, from_queue }));
            }
        });
    });
}

/// Запустить удаление выбранным способом (и шаги после него).
pub fn run_remove(st: St, ask: RemoveAsk, mode: RemoveMode) {
    let mut ops = vec![Op::Remove { names: ask.names.clone(), mode }];
    ops.extend(ask.then);
    take_from_queue(st, &ask.from_queue);
    st.remove_ask.set(None);
    start_ops(st, ask.title, ops);
}

fn start_ops(st: St, title: String, mut ops: Vec<Op>) {
    let op = if ops.len() == 1 { ops.remove(0) } else { Op::Batch(ops) };
    run_op(st, title, op);
}

/// Не обновлять пакет никогда (`[packages] ignore`) или снова обновлять.
/// Вид списков пакетов (карточки или строки) — сразу и в конфиг.
pub fn set_cards(st: St, cards: bool) {
    st.cards.set(cards);
    let view = if cards { "cards" } else { "list" };
    match synshell_common::config_edit::set_value(&["packages", "view"], toml_edit::Value::from(view)) {
        Ok(_) => {
            let mut cfg = st.cfg.get_untracked();
            cfg.packages.view = view.into();
            st.cfg.set(cfg);
        }
        Err(e) => st.toast.set(format!("Не удалось сохранить вид: {e}")),
    }
}

pub fn set_ignored(st: St, name: &str, ignore: bool) {
    let mut list = st.cfg.get_untracked().packages.ignore.clone();
    list.retain(|n| n != name);
    if ignore {
        list.push(name.to_string());
    }
    let arr = toml_edit::Value::Array(list.iter().map(|a| toml_edit::Value::from(a.as_str())).collect());
    match synshell_common::config_edit::set_value(&["packages", "ignore"], arr) {
        Ok(_) => {
            let mut cfg = st.cfg.get_untracked();
            cfg.packages.ignore = list;
            st.cfg.set(cfg);
            st.toast.set(if ignore { format!("{name} больше не обновляется") } else { format!("{name} снова обновляется") });
        }
        Err(e) => st.toast.set(format!("Не удалось сохранить: {e}")),
    }
}

/// Запустить операцию: задание появляется в «Задачах», лог идёт вживую.
pub fn run_op(st: St, title: String, op: Op) {
    let id = JOB_ID.fetch_add(1, Ordering::SeqCst);
    let build_user = st.cfg.get_untracked().packages.build_user.clone();
    let job = pk::start(op, build_user);
    let mut jobs = st.jobs.get_untracked();
    jobs.insert(0, JobView { id, title: title.clone(), stage: "Запуск…".into(), log: Vec::new(), done: None, cancel: job.cancel.clone(), cancelling: false });
    st.jobs.set(jobs);
    st.job_open.set(Some(id));
    st.toast.set(format!("{title} — в «Задачах»"));
    std::thread::spawn(move || {
        let mut batch: Vec<JobEvent> = Vec::new();
        loop {
            match job.rx.recv_timeout(Duration::from_millis(150)) {
                Ok(e) => {
                    let end = matches!(e, JobEvent::Done(_));
                    batch.push(e);
                    if !end {
                        continue;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            if batch.is_empty() {
                continue;
            }
            let events = std::mem::take(&mut batch);
            let finished = events.iter().any(|e| matches!(e, JobEvent::Done(_)));
            run_on_main_thread(move || {
                let mut jobs = st.jobs.get_untracked();
                if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
                    for e in events {
                        match e {
                            JobEvent::Line(l) => j.log.push(l),
                            JobEvent::Stage(s) if !j.cancelling => j.stage = s,
                            JobEvent::Stage(_) => {}
                            JobEvent::Done(r) => {
                                let cancelled = r.as_ref().is_err_and(|e| e == pk::CANCELLED);
                                st.toast.set(match &r {
                                    Ok(()) => format!("Готово: {}", j.title),
                                    Err(_) if cancelled => format!("Отменено: {}", j.title),
                                    Err(e) => format!("Ошибка: {} — {e}", j.title),
                                });
                                j.stage = match &r {
                                    Ok(()) => "Готово".into(),
                                    Err(_) if cancelled => "Отменено".into(),
                                    Err(_) => "Ошибка".into(),
                                };
                                j.cancelling = false;
                                j.done = Some(r);
                            }
                        }
                    }
                    let n = j.log.len();
                    if n > 600 {
                        j.log.drain(..n - 600);
                    }
                }
                st.jobs.set(jobs);
                if finished {
                    load_installed(st);
                    refresh_catalog_installed(st);
                    let q = st.query.get_untracked();
                    if !q.is_empty() {
                        search(st, q);
                    }
                    let q = st.aur_query.get_untracked();
                    if !q.is_empty() {
                        search_aur(st, q);
                    }
                    if let Some(p) = st.selected.get_untracked() {
                        let mut p = p;
                        p.installed = pk::installed_map().get(&p.name).cloned();
                        select(st, p);
                    }
                    if st.updates.get_untracked().is_some() {
                        check_updates(st);
                    }
                    if st.aur_updates.get_untracked().is_some() {
                        check_aur_updates(st);
                    }
                }
            });
            if finished {
                break;
            }
        }
    });
}

/// Отменить задание: процессу — SIGINT, следующие шаги не начнутся.
pub fn cancel_job(st: St, id: u64, c: &pk::JobCancel) {
    c.cancel();
    let mut jobs = st.jobs.get_untracked();
    if let Some(j) = jobs.iter_mut().find(|j| j.id == id && j.done.is_none()) {
        j.cancelling = true;
        j.stage = "Отмена…".into();
        j.log.push("— отмена задания —".into());
    }
    st.jobs.set(jobs);
}
