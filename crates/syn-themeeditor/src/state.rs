//! Состояние редактора: черновик темы, история правок, режимы, пипетка,
//! примерка на всей системе и пересборка стилей предпросмотра.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use syngui::prelude::*;
use synshell_common::theme::{self, Theme};
use synshell_common::Config;

use crate::draft::{self, Draft, Origin, Snapshot};

/// Режим правой панели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Генератор: акцент, окраска фонов, контраст, обои.
    Quick,
    /// Все токены по группам.
    Palette,
    /// Форма: скругления, прозрачность, панель, заголовки окон.
    Shape,
    /// MSS-слои.
    Code,
    /// Название, описание, автор, экспорт.
    About,
}

/// Сцена предпросмотра.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Desktop,
    Settings,
}

/// Что выбрано пипеткой: подпись, токены и селекторы элемента.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub label: String,
    pub tokens: Vec<&'static str>,
    pub selectors: Vec<&'static str>,
}

/// Строка списка тем.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeItem {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    /// Своя тема заменяет встроенную с тем же id.
    pub overrides_builtin: bool,
    pub swatches: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct St {
    pub themes: RwSignal<Vec<ThemeItem>>,
    /// Изменения черновика (сам черновик — в [`DRAFT`]).
    pub rev: RwSignal<u64>,
    /// Изменения «снаружи» поля ввода (отмена, генератор, открытие темы):
    /// по нему перестраиваются пикеры цвета. Правка самим пикером его не
    /// трогает — иначе окно пикера закрывалось бы посреди перетаскивания.
    pub ext: RwSignal<u64>,
    pub current: RwSignal<String>,
    pub variant: RwSignal<&'static str>,
    pub mode: RwSignal<Mode>,
    pub scene: RwSignal<Scene>,
    pub tryon: RwSignal<bool>,
    pub dirty: RwSignal<bool>,
    pub pipette: RwSignal<bool>,
    pub pick: RwSignal<Option<Pick>>,
    pub filter: RwSignal<String>,
    pub layer: RwSignal<String>,
    pub toast: RwSignal<String>,
    pub confirm: RwSignal<Option<Confirm>>,
    pub styles: RwSignal<String>,
    pub window: RwSignal<syngui::window::WindowState>,
    /// Ошибка разбора темы (`theme.toml`), если есть.
    pub error: RwSignal<Option<String>>,
}

/// Вопрос с подтверждением.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Delete,
    DiscardAndOpen,
}

struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Ключ и время последней правки: серия правок одного поля (ползунок,
    /// перетаскивание в палитре) — один шаг отмены.
    last: Option<(String, Instant)>,
}

thread_local! {
    static DRAFT: RefCell<Draft> = RefCell::new(Draft::new_from(None, "my-theme".into(), "Моя тема"));
    static HISTORY: RefCell<History> = const { RefCell::new(History { undo: Vec::new(), redo: Vec::new(), last: None }) };
    /// Открытая тема, когда её выбрали при несохранённых правках.
    static PENDING_OPEN: RefCell<Option<String>> = const { RefCell::new(None) };
    static SAVED: RefCell<Option<Snapshot>> = const { RefCell::new(None) };
}

/// Прочитать черновик.
pub fn with_draft<R>(f: impl FnOnce(&Draft) -> R) -> R {
    DRAFT.with(|d| f(&d.borrow()))
}

/// Поколение примерки: запись в фоне только последнего состояния.
static TRYON_GEN: AtomicU64 = AtomicU64::new(0);
static STYLE_GEN: AtomicU64 = AtomicU64::new(0);

impl St {
    pub fn new() -> Self {
        Self {
            themes: use_signal(Vec::new()),
            rev: use_signal(0),
            ext: use_signal(0),
            current: use_signal(String::new()),
            variant: use_signal("dark"),
            mode: use_signal(Mode::Quick),
            scene: use_signal(Scene::Desktop),
            tryon: use_signal(false),
            dirty: use_signal(false),
            pipette: use_signal(false),
            pick: use_signal(None),
            filter: use_signal(String::new()),
            layer: use_signal("shell".to_string()),
            toast: use_signal(String::new()),
            confirm: use_signal(None),
            styles: use_signal(String::new()),
            window: use_signal(syngui::window::WindowState::default()),
            error: use_signal(None),
        }
    }

    pub fn dark(&self) -> bool {
        self.variant.get_untracked() == "dark"
    }

    // ── Список тем ───────────────────────────────────────────────────────

    pub fn reload_list(&self) {
        let builtin: Vec<&str> = theme::builtin_ids().collect();
        let items = theme::list()
            .into_iter()
            .map(|t| {
                let (v, _) = t.variant(self.dark());
                let sw = [&v.bg, &v.surface, &v.accent, &v.fg].iter().map(|c| (*c).clone().unwrap_or_default()).collect();
                let is_builtin = t.source == theme::Source::Builtin;
                ThemeItem {
                    overrides_builtin: !is_builtin && builtin.contains(&t.id.as_str()),
                    id: t.id,
                    name: t.name,
                    builtin: is_builtin,
                    swatches: sw,
                }
            })
            .collect();
        self.themes.set(items);
    }

    // ── Открыть, создать, сохранить ─────────────────────────────────────

    /// Открыть тему `id`. При несохранённых правках — спросить.
    pub fn open(&self, id: &str) {
        if self.dirty.get_untracked() && self.current.get_untracked() != id {
            PENDING_OPEN.with(|p| *p.borrow_mut() = Some(id.to_string()));
            self.confirm.set(Some(Confirm::DiscardAndOpen));
            return;
        }
        self.open_now(id);
    }

    pub fn open_pending(&self) {
        if let Some(id) = PENDING_OPEN.with(|p| p.borrow_mut().take()) {
            self.open_now(&id);
        }
    }

    fn open_now(&self, id: &str) {
        match Theme::find(id) {
            Ok(t) => {
                let d = Draft::from_theme(&t);
                // Схема: та, что есть у темы и выбрана в системе.
                let sys_dark = Config::load().0.appearance.color_scheme == synshell_common::config::ColorScheme::Dark;
                self.variant.set(d.effective_variant(sys_dark));
                self.load(d);
            }
            Err(e) => self.toast.set(e),
        }
    }

    fn load(&self, d: Draft) {
        let id = d.id.clone();
        DRAFT.with(|x| *x.borrow_mut() = d);
        HISTORY.with(|h| *h.borrow_mut() = History { undo: Vec::new(), redo: Vec::new(), last: None });
        SAVED.with(|s| *s.borrow_mut() = Some(with_draft(Draft::snapshot)));
        self.current.set(id);
        self.dirty.set(false);
        self.pick.set(None);
        self.ext.set(self.ext.get_untracked() + 1);
        self.after_change();
    }

    /// Новая тема: копия `base` (или текущей) под свободным именем.
    pub fn create(&self, base: Option<&str>, name: &str) {
        let base_theme = base.and_then(|b| Theme::find(b).ok());
        let id = draft::free_id(&draft::safe_id(name));
        let d = Draft::new_from(base_theme.as_ref().or(None), id, name);
        let d = if base_theme.is_none() {
            // Пустая тема — сразу с палитрой из генератора.
            let mut d = d;
            for (k, v) in crate::color::generate(crate::color::GenParams {
                accent: synshell_common::config::Rgba::rgb(0x3d, 0x8b, 0xfd),
                dark: true,
                tint: 0.4,
                hue: None,
                contrast: 0.5,
            }) {
                d.set("dark", k, Some(&v));
            }
            d
        } else {
            d
        };
        self.variant.set(d.effective_variant(self.dark()));
        self.load(d);
        self.dirty.set(true);
        self.save();
    }

    /// Дублировать текущую тему.
    pub fn duplicate(&self) {
        let (name, files) = with_draft(|d| (d.name(), d.files()));
        let new_name = synshell_common::t!("{name} (копия)", name = name);
        let id = draft::free_id(&draft::safe_id(&new_name));
        match Theme::from_files(&id, theme::Source::Builtin, &files) {
            Ok(t) => {
                let mut d = Draft::from_theme(&t);
                d.origin = Origin::New;
                d.set_meta("name", &new_name);
                self.load(d);
                self.dirty.set(true);
                self.save();
            }
            Err(e) => self.toast.set(e),
        }
    }

    /// Сохранить в `~/.config/synshell/themes/<id>`.
    pub fn save(&self) {
        let r = DRAFT.with(|d| d.borrow_mut().save());
        match r {
            Ok(dir) => {
                SAVED.with(|s| *s.borrow_mut() = Some(with_draft(Draft::snapshot)));
                self.dirty.set(false);
                self.reload_list();
                self.current.set(with_draft(|d| d.id.clone()));
                self.toast.set(synshell_common::t!("Сохранено: {dir}", dir = dir.display()));
            }
            Err(e) => self.toast.set(synshell_common::t!("Не сохранилось: {e}", e = e)),
        }
    }

    /// Сохранить и сделать темой системы.
    pub fn apply(&self) {
        self.save();
        if self.dirty.get_untracked() {
            return;
        }
        let id = with_draft(|d| d.id.clone());
        let st = *self;
        std::thread::spawn(move || {
            let r = std::process::Command::new("synsettings").args(["--apply-theme", &id]).status();
            run_on_main_thread(move || match r {
                Ok(s) if s.success() => st.toast.set(synshell_common::t!("Тема применена")),
                Ok(_) | Err(_) => st.toast.set(synshell_common::t!("Не удалось применить: synsettings --apply-theme")),
            });
        });
    }

    /// Удалить свою тему с диска.
    pub fn delete(&self) {
        let (dir, builtin_twin) = with_draft(|d| (d.save_dir(), theme::builtin_ids().any(|b| b == d.id)));
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                self.toast.set(e.to_string());
                return;
            }
        }
        self.reload_list();
        self.dirty.set(false);
        let next = if builtin_twin { with_draft(|d| d.id.clone()) } else { theme::builtin_ids().next().unwrap_or("nord").to_string() };
        self.open_now(&next);
        self.toast.set(if builtin_twin { synshell_common::t!("Своя версия удалена — снова встроенная") } else { synshell_common::t!("Тема удалена") });
    }

    /// Вернуть сохранённое состояние.
    pub fn revert(&self) {
        if let Some(s) = SAVED.with(|s| s.borrow().clone()) {
            self.edit("revert", |d| d.restore(&s));
            self.dirty.set(false);
        }
    }

    // ── Правки ───────────────────────────────────────────────────────────

    /// Изменить черновик. `key` — что меняется: серия правок одного ключа
    /// подряд (< 0.8 с) — один шаг отмены. Встроенная тема при первой
    /// правке становится своей копией с тем же именем каталога (заменит
    /// встроенную после сохранения; «Удалить» вернёт встроенную).
    pub fn edit(&self, key: &str, f: impl FnOnce(&mut Draft)) {
        self.edit_inner(key, false, f);
    }

    /// Правка из поля, которое само показывает новое значение (пикер цвета,
    /// ползунок): поля ввода не перестраиваются.
    pub fn edit_quiet(&self, key: &str, f: impl FnOnce(&mut Draft)) {
        self.edit_inner(key, true, f);
    }

    fn edit_inner(&self, key: &str, quiet: bool, f: impl FnOnce(&mut Draft)) {
        let before = with_draft(Draft::snapshot);
        HISTORY.with(|h| {
            let mut h = h.borrow_mut();
            let now = Instant::now();
            let merge = h.last.as_ref().is_some_and(|(k, t)| k == key && now.duration_since(*t) < Duration::from_millis(800));
            if !merge {
                h.undo.push(before.clone());
                if h.undo.len() > 200 {
                    h.undo.remove(0);
                }
            }
            h.redo.clear();
            h.last = Some((key.to_string(), now));
        });
        DRAFT.with(|d| {
            let mut d = d.borrow_mut();
            if d.origin == Origin::Builtin {
                d.origin = Origin::New;
            }
            f(&mut d);
        });
        let changed = with_draft(Draft::snapshot) != before;
        if changed {
            let saved = SAVED.with(|s| s.borrow().clone());
            self.dirty.set(saved.as_ref() != Some(&with_draft(Draft::snapshot)));
            if !quiet {
                self.ext.set(self.ext.get_untracked() + 1);
            }
            self.after_change();
        }
    }

    pub fn can_undo(&self) -> bool {
        HISTORY.with(|h| !h.borrow().undo.is_empty())
    }

    pub fn can_redo(&self) -> bool {
        HISTORY.with(|h| !h.borrow().redo.is_empty())
    }

    pub fn undo(&self) {
        self.step(true);
    }

    pub fn redo(&self) {
        self.step(false);
    }

    fn step(&self, back: bool) {
        let cur = with_draft(Draft::snapshot);
        let target = HISTORY.with(|h| {
            let mut h = h.borrow_mut();
            h.last = None;
            let s = if back { h.undo.pop() } else { h.redo.pop() }?;
            if back {
                h.redo.push(cur);
            } else {
                h.undo.push(cur);
            }
            Some(s)
        });
        if let Some(s) = target {
            DRAFT.with(|d| d.borrow_mut().restore(&s));
            let saved = SAVED.with(|s| s.borrow().clone());
            self.dirty.set(saved.as_ref() != Some(&s));
            self.current.set(s.id);
            self.ext.set(self.ext.get_untracked() + 1);
            self.after_change();
        }
    }

    // ── Пересборка ──────────────────────────────────────────────────────

    /// После любой правки: стили предпросмотра, примерка, счётчик.
    pub fn after_change(&self) {
        self.rev.set(self.rev.get_untracked() + 1);
        self.rebuild_styles();
        self.push_tryon();
    }

    /// Пересобрать таблицу окна (сам редактор + предпросмотр) в фоне —
    /// разбор тяжёлых таблиц оболочки не держит ввод.
    pub fn rebuild_styles(&self) {
        let st = *self;
        let gen = STYLE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let files = with_draft(|d| (d.id.clone(), d.files()));
        let dark = self.dark();
        std::thread::spawn(move || {
            let (id, files) = files;
            let base = Config::load().0;
            let mss = match Theme::from_files(&id, theme::Source::Builtin, &files) {
                Ok(t) => {
                    let cfg = crate::preview_style::preview_config(&base, &t, dark);
                    let mut s = crate::editor_styles(&base);
                    s.push_str(&crate::preview_style::build(&cfg));
                    run_on_main_thread(move || st.error.set(None));
                    s
                }
                Err(e) => {
                    run_on_main_thread(move || st.error.set(Some(e)));
                    return;
                }
            };
            if STYLE_GEN.load(Ordering::SeqCst) == gen {
                run_on_main_thread(move || {
                    if STYLE_GEN.load(Ordering::SeqCst) == gen {
                        st.styles.set(mss);
                    }
                });
            }
        });
    }

    /// Примерка на всей системе: записать черновик (с задержкой — серия
    /// правок даёт одну запись).
    pub fn push_tryon(&self) {
        if !self.tryon.get_untracked() {
            return;
        }
        let gen = TRYON_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let (id, files) = with_draft(|d| (d.id.clone(), d.files()));
        let scheme = self.variant.get_untracked();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            if TRYON_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let mut over = toml::Table::new();
            over.insert("color_scheme".into(), toml::Value::String(scheme.into()));
            over.insert("accent".into(), toml::Value::String(String::new()));
            over.insert("colors".into(), toml::Value::Table(toml::Table::new()));
            if let Err(e) = synshell_common::preview::write(&id, &files, &over) {
                tracing::warn!("примерка: {e}");
            }
        });
    }

    pub fn set_tryon(&self, on: bool) {
        self.tryon.set(on);
        if on {
            self.push_tryon();
            self.toast.set(synshell_common::t!("Примерка: вся система показывает черновик, пока открыт редактор"));
        } else {
            TRYON_GEN.fetch_add(1, Ordering::SeqCst);
            synshell_common::preview::clear();
        }
    }

    pub fn set_variant(&self, v: &'static str) {
        if self.variant.get_untracked() == v {
            return;
        }
        self.variant.set(v);
        self.ext.set(self.ext.get_untracked() + 1);
        self.reload_list();
        self.rebuild_styles();
        self.push_tryon();
    }
}

/// Убрать примерку при выходе (и при панике — через Drop).
pub struct TryonGuard;

impl Drop for TryonGuard {
    fn drop(&mut self) {
        synshell_common::preview::clear();
    }
}

/// Действующие значения токенов черновика (с учётом формул и значений по
/// умолчанию) для текущего варианта: имя без `--` → значение. Кэш по `rev`.
pub fn values(st: &St) -> Arc<std::collections::BTreeMap<String, String>> {
    thread_local! {
        static CACHE: RefCell<Option<(u64, bool, Arc<std::collections::BTreeMap<String, String>>)>> = const { RefCell::new(None) };
        static BASE: RefCell<Option<Config>> = const { RefCell::new(None) };
    }
    let rev = st.rev.get_untracked();
    let dark = st.dark();
    if let Some(v) = CACHE.with(|c| c.borrow().as_ref().filter(|(r, d, _)| *r == rev && *d == dark).map(|x| x.2.clone())) {
        return v;
    }
    let base = BASE.with(|b| b.borrow_mut().get_or_insert_with(|| Config::load().0).clone());
    let mut map = std::collections::BTreeMap::new();
    if let Ok(t) = with_draft(Draft::to_theme) {
        let cfg = crate::preview_style::preview_config(&base, &t, dark);
        let a = &cfg.appearance;
        let ctx = synshell_common::tokens::Ctx::new(a);
        let (variant, _) = t.variant(dark);
        for tok in synshell_common::tokens::TOKENS {
            let v = match tok.source {
                synshell_common::tokens::Source::Variant(k) => match k {
                    "shadow" => variant.shadow.clone().or(Some(if a.is_dark() { "#00000080".into() } else { "#00000038".into() })),
                    "scrim" => variant.scrim.clone().or(Some(ctx.p.bg.with_alpha(0.35).hex())),
                    _ => None,
                },
                _ => tok.value(&ctx),
            };
            if let Some(v) = v {
                map.insert(tok.name.to_string(), v);
            }
        }
        for (k, v) in &variant.vars {
            map.insert(k.trim_start_matches('-').to_string(), v.clone());
        }
        let (ta, ti) = a.titlebar_colors(&cfg.decorations);
        map.insert("@titlebar".into(), ta.hex());
        map.insert("@titlebar_inactive".into(), ti.hex());
        map.insert("@wallpaper".into(), a.wallpaper_background(&Default::default()));
    }
    let v = Arc::new(map);
    CACHE.with(|c| *c.borrow_mut() = Some((rev, dark, v.clone())));
    v
}
