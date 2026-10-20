//! Библиотека игр gameshell: игры из .desktop (категории `[gameshell]
//! game_categories`), библиотека Steam, свои ярлыки `[[gameshell.game]]`;
//! отдельно — все приложения. Избранное и время запуска — в
//! `~/.local/state/synshell/gameshell.json`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use synshell_common::config::Gameshell;
use synshell_common::xdg::{self, DesktopEntry};

use crate::steam;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Steam,
    Desktop,
    Custom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    /// `steam:<appid>`, id .desktop, `custom:<имя>`.
    pub id: String,
    pub name: String,
    pub source: Source,
    pub command: String,
    pub terminal: bool,
    /// Обложка 2:3.
    pub cover: Option<PathBuf>,
    /// Широкая афиша.
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    /// Широкая шапка (если нет афиши).
    pub header: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub tags: Vec<String>,
    /// Окна игры: app_id (точно) — `steam_app_<id>`, свой `app_id`.
    pub app_id: String,
    /// .desktop, по которому узнаётся окно.
    pub desktop_id: Option<String>,
    pub is_game: bool,
    /// Последний запуск (Unix-время, наш или Steam).
    pub last_played: u64,
}

impl Game {
    /// Афиша для фона и главной: hero, иначе шапка, иначе обложка.
    pub fn backdrop(&self) -> Option<PathBuf> {
        self.hero.clone().or_else(|| self.header.clone()).or_else(|| self.cover.clone())
    }

    pub fn source_label(&self) -> String {
        match self.source {
            Source::Steam => "Steam".into(),
            Source::Desktop if self.is_game => syngui::t!("Игра").to_string(),
            Source::Desktop => syngui::t!("Приложение").to_string(),
            Source::Custom => syngui::t!("Свой ярлык").to_string(),
        }
    }
}

/// Состояние пользователя: избранное и последние запуски.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserState {
    pub favorites: Vec<String>,
    pub played: BTreeMap<String, u64>,
}

fn state_file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| synshell_common::paths::expand_tilde("~/.local/state"));
    base.join("synshell/gameshell.json")
}

impl UserState {
    pub fn load() -> Self {
        std::fs::read_to_string(state_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = state_file();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|f| f == id)
    }

    pub fn toggle_favorite(&mut self, id: &str) -> bool {
        if self.is_favorite(id) {
            self.favorites.retain(|f| f != id);
            false
        } else {
            self.favorites.push(id.to_string());
            true
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn icon_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') || name.starts_with('~') {
        let p = synshell_common::paths::expand_tilde(name);
        return p.is_file().then_some(p);
    }
    xdg::lookup_icon(name)
}

fn opt_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let p = synshell_common::paths::expand_tilde(s);
    p.is_file().then_some(p)
}

fn from_desktop(e: &DesktopEntry, is_game: bool) -> Game {
    Game {
        id: e.id.clone(),
        name: e.name.clone(),
        source: Source::Desktop,
        command: e.command(),
        terminal: e.terminal,
        cover: None,
        hero: None,
        logo: None,
        header: None,
        icon: icon_path(&e.icon),
        tags: Vec::new(),
        app_id: String::new(),
        desktop_id: Some(e.id.clone()),
        is_game,
        last_played: 0,
    }
}

/// Найденные игры и все приложения.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Library {
    pub games: Vec<Game>,
    pub apps: Vec<Game>,
    /// Приложения Android (syndroid) — своя вкладка.
    pub android: Vec<Game>,
}

impl Library {
    pub fn scan(cfg: &Gameshell, state: &UserState) -> Library {
        let hidden = |id: &str| cfg.hide.iter().any(|h| h.trim() == id);
        let cats: Vec<String> = cfg.game_categories.iter().map(|c| c.trim().to_lowercase()).collect();
        let apps = xdg::apps();
        let mut games: Vec<Game> = Vec::new();

        // Steam.
        let steam_games = if cfg.steam { steam::games(cfg) } else { Vec::new() };
        let steam_ids: Vec<u32> = steam_games.iter().map(|g| g.appid).collect();
        let launch = if cfg.steam_launch.trim().is_empty() { "steam steam://rungameid/{appid}".to_string() } else { cfg.steam_launch.clone() };
        for g in steam_games {
            let id = format!("steam:{}", g.appid);
            games.push(Game {
                name: g.name,
                source: Source::Steam,
                command: launch.replace("{appid}", &g.appid.to_string()),
                terminal: false,
                cover: g.cover,
                hero: g.hero,
                logo: g.logo,
                header: g.header,
                icon: xdg::lookup_icon(&format!("steam_icon_{}", g.appid)),
                tags: Vec::new(),
                app_id: format!("steam_app_{}", g.appid),
                desktop_id: None,
                is_game: true,
                last_played: g.last_played,
                id,
            });
        }

        // .desktop.
        let mut all_apps: Vec<Game> = Vec::new();
        for e in apps.iter().filter(|e| !e.no_display && e.android.is_none()) {
            let steam_dup = steam::appid_in_command(&e.exec).filter(|a| steam_ids.contains(a));
            let is_game = cfg.include_apps.iter().any(|i| i.trim() == e.id || i.trim().trim_end_matches(".desktop") == e.id)
                || e.categories.iter().any(|c| cats.contains(&c.to_lowercase()));
            let g = from_desktop(e, is_game);
            if steam_dup.is_none() {
                all_apps.push(g.clone());
            }
            if cfg.desktop_games && is_game && steam_dup.is_none() {
                games.push(g);
            }
        }
        // Android-приложения — своя вкладка; несколько экземпляров — подпись в тегах.
        let mut android: Vec<Game> = Vec::new();
        for e in apps.iter().filter(|e| !e.no_display && e.android.is_some()) {
            let mut g = from_desktop(e, false);
            if let Some(a) = &e.android {
                g.tags.push(format!("android:{}", a.title));
            }
            android.push(g);
        }

        // Свои ярлыки и правка найденного.
        for c in &cfg.games {
            let id = if c.id.trim().is_empty() { format!("custom:{}", c.name.trim()) } else { c.id.trim().trim_end_matches(".desktop").to_string() };
            let target = games.iter_mut().find(|g| g.id == id || g.id.trim_end_matches(".desktop") == id);
            let g = match target {
                Some(g) => g,
                None => {
                    if c.command.trim().is_empty() {
                        log::warn!("[[gameshell.game]] «{id}»: нет command и такой игры не найдено");
                        continue;
                    }
                    games.push(Game {
                        id: id.clone(),
                        name: if c.name.trim().is_empty() { id.clone() } else { c.name.clone() },
                        source: Source::Custom,
                        command: c.command.clone(),
                        terminal: false,
                        cover: None,
                        hero: None,
                        logo: None,
                        header: None,
                        icon: None,
                        tags: Vec::new(),
                        app_id: String::new(),
                        desktop_id: None,
                        is_game: true,
                        last_played: 0,
                    });
                    games.last_mut().unwrap()
                }
            };
            if !c.name.trim().is_empty() {
                g.name = c.name.clone();
            }
            if !c.command.trim().is_empty() {
                g.command = c.command.clone();
            }
            if let Some(p) = opt_path(&c.cover) {
                g.cover = Some(p);
            }
            if let Some(p) = opt_path(&c.hero) {
                g.hero = Some(p);
            }
            if let Some(p) = opt_path(&c.logo) {
                g.logo = Some(p);
            }
            if let Some(p) = icon_path(c.icon.trim()) {
                g.icon = Some(p);
            }
            if !c.app_id.trim().is_empty() {
                g.app_id = c.app_id.trim().to_string();
            }
            for t in &c.tags {
                if !g.tags.contains(t) {
                    g.tags.push(t.clone());
                }
            }
            if c.hidden {
                g.tags.push("\u{0}hidden".into());
            }
        }

        let played = |g: &mut Game| {
            if let Some(t) = state.played.get(&g.id) {
                g.last_played = g.last_played.max(*t);
            }
        };
        games.retain(|g| !hidden(&g.id) && !g.tags.iter().any(|t| t == "\u{0}hidden"));
        all_apps.retain(|g| !hidden(&g.id));
        android.retain(|g| !hidden(&g.id));
        games.iter_mut().for_each(played);
        all_apps.iter_mut().for_each(played);
        android.iter_mut().for_each(played);
        let by_name = |a: &Game, b: &Game| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        games.sort_by(by_name);
        all_apps.sort_by(by_name);
        android.sort_by(by_name);
        Library { games, apps: all_apps, android }
    }

    pub fn find(&self, id: &str) -> Option<&Game> {
        self.games.iter().chain(self.apps.iter()).chain(self.android.iter()).find(|g| g.id == id)
    }

    /// Недавние игры (и приложения, запущенные из gameshell).
    pub fn recent(&self, n: usize) -> Vec<Game> {
        let mut seen: HashMap<&str, ()> = HashMap::new();
        let mut v: Vec<&Game> = self.games.iter().chain(self.apps.iter()).chain(self.android.iter()).filter(|g| g.last_played > 0 && seen.insert(g.id.as_str(), ()).is_none()).collect();
        v.sort_by(|a, b| b.last_played.cmp(&a.last_played));
        v.into_iter().take(n).cloned().collect()
    }
}

/// Окно, принадлежащее игре (`app_id` окна).
pub fn window_matches(g: &Game, window_app_id: &str) -> bool {
    if window_app_id.is_empty() {
        return false;
    }
    if !g.app_id.is_empty() {
        return g.app_id.eq_ignore_ascii_case(window_app_id);
    }
    match &g.desktop_id {
        Some(d) => xdg::app_for_window(window_app_id).is_some_and(|e| &e.id == d),
        None => false,
    }
}
