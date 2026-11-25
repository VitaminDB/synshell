//! Раскладка док-панелей и пресеты видов (как в hexspire):
//! - текущая раскладка и активный вид сохраняются в `~/.config/syn-video-editor/layout.toml`
//!   при каждом изменении структуры, активной вкладки или после перетаскивания разделителя;
//! - встроенные виды (`Монтаж`, `Моушн`, `Граф`, `ИИ-монтаж`) и пользовательские пресеты
//!   в `~/.config/syn-video-editor/layouts/*.toml`: применить, сохранить текущий вид,
//!   перезаписать, переименовать, дублировать, удалить/сбросить.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use syngui::signal::{use_effect, RwSignal};
use syngui::widgets::{DockSpec, DockState};

/// Все панели редактора: id и заголовок вкладки.
pub const PANELS: &[(&str, &str)] = &[
    ("media", "Медиатека"),
    ("templates", "Шаблоны"),
    ("effects", "Эффекты"),
    ("layers", "Слои"),
    ("viewer", "Просмотр"),
    ("timeline", "Таймлайн"),
    ("inspector", "Инспектор"),
    ("ai", "ИИ"),
    ("graph", "Граф"),
    ("curves", "Кривые"),
];

pub fn panel_title(id: &str) -> String {
    PANELS.iter().find(|(p, _)| *p == id).map(|(_, t)| *t).unwrap_or(id).to_string()
}

pub fn is_known_panel(id: &str) -> bool {
    PANELS.iter().any(|(p, _)| *p == id)
}

/// Встроенные пресеты видов: (ключ, название, описание, порядок).
pub const BUILTINS: &[(&str, &str, &str, i32)] = &[
    ("edit", "Монтаж", "Классический монтаж: медиатека и эффекты слева, просмотр в центре, инспектор справа, таймлайн снизу", 10),
    ("motion", "Моушн", "Моушн-графика: дерево слоёв слева, просмотр и инспектор сверху, таймлайн и кривые рядом снизу", 20),
    ("graph", "Граф", "Узловой граф эффектов в центре, просмотр и инспектор справа", 30),
    ("ai", "ИИ-монтаж", "Широкая панель ИИ-агента справа для монтажа и правок текстом", 40),
];

pub const DEFAULT_VIEW: &str = "edit";

/// Встроенная раскладка для ключа вида.
pub fn builtin_spec(key: &str) -> DockSpec {
    let left = DockSpec::tabs(["media", "templates", "effects"]);
    let right = DockSpec::tabs(["inspector", "ai"]);
    match key {
        "motion" => DockSpec::column(
            DockSpec::row(
                DockSpec::row(DockSpec::tabs(["layers", "templates", "effects", "media"]), DockSpec::tabs(["viewer"]).named("viewer"), 0.26),
                right,
                0.74,
            ),
            DockSpec::row(DockSpec::tabs(["timeline"]), DockSpec::tabs(["curves"]), 0.56),
            0.56,
        ),
        "graph" => DockSpec::column(
            DockSpec::row(
                DockSpec::tabs(["media", "templates", "effects"]),
                DockSpec::row(DockSpec::tabs(["graph"]).named("graph"), DockSpec::tabs(["viewer", "inspector", "ai"]), 0.7),
                0.2,
            ),
            DockSpec::tabs(["timeline", "curves"]),
            0.7,
        ),
        "ai" => DockSpec::row(
            DockSpec::column(DockSpec::row(left, DockSpec::tabs(["viewer"]).named("viewer"), 0.32), DockSpec::tabs(["timeline", "curves"]), 0.58),
            DockSpec::tabs(["ai", "inspector"]),
            0.68,
        ),
        _ => DockSpec::column(
            DockSpec::row(DockSpec::row(left, DockSpec::tabs(["viewer", "graph"]).named("viewer"), 0.26), right, 0.74),
            DockSpec::tabs(["timeline", "curves"]),
            0.58,
        ),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub key: String,
    pub name: String,
    pub description: String,
    pub order: i32,
    pub spec: DockSpec,
    /// Файл пользовательского пресета (или переопределения встроенного).
    pub path: Option<PathBuf>,
    pub builtin: bool,
    /// Встроенный пресет был перезаписан пользователем.
    pub modified: bool,
}

const LAYOUT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Node {
    Split { vertical: bool, ratio: f32, first: Box<Node>, second: Box<Node> },
    Tabs {
        #[serde(default)]
        panels: Vec<String>,
        #[serde(default)]
        active: usize,
        #[serde(default)]
        name: String,
    },
}

#[derive(Serialize, Deserialize)]
struct File {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    order: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    view: String,
    #[serde(default)]
    version: u32,
    root: Node,
}

fn is_zero(v: &i32) -> bool {
    *v == 0
}

fn to_node(spec: &DockSpec) -> Node {
    match spec {
        DockSpec::Split { vertical, ratio, first, second } => Node::Split {
            vertical: *vertical,
            ratio: ratio.clamp(0.05, 0.95),
            first: Box::new(to_node(first)),
            second: Box::new(to_node(second)),
        },
        DockSpec::Tabs { panels, active, name } => {
            let kept: Vec<String> = panels.iter().filter(|p| is_known_panel(p)).cloned().collect();
            let active = panels.get(*active).and_then(|a| kept.iter().position(|p| p == a)).unwrap_or(0);
            Node::Tabs { panels: kept, active, name: name.clone() }
        }
    }
}

fn to_spec(node: Node) -> Option<DockSpec> {
    match node {
        Node::Split { vertical, ratio, first, second } => match (to_spec(*first), to_spec(*second)) {
            (Some(a), Some(b)) => Some(DockSpec::Split {
                vertical,
                ratio: if ratio.is_finite() { ratio.clamp(0.05, 0.95) } else { 0.5 },
                first: Box::new(a),
                second: Box::new(b),
            }),
            (a, b) => a.or(b),
        },
        Node::Tabs { panels, active, name } => {
            let panels: Vec<String> = panels.into_iter().filter(|p| is_known_panel(p)).collect();
            let active = active.min(panels.len().saturating_sub(1));
            (!panels.is_empty() || !name.is_empty()).then_some(DockSpec::Tabs { panels, active, name })
        }
    }
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("syn-video-editor")
}

pub fn layout_file() -> PathBuf {
    config_dir().join("layout.toml")
}

pub fn presets_dir() -> PathBuf {
    config_dir().join("layouts")
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

pub fn encode_current(spec: &DockSpec, view: &str) -> String {
    toml::to_string_pretty(&File {
        name: String::new(),
        description: String::new(),
        order: 0,
        view: view.to_string(),
        version: LAYOUT_VERSION,
        root: to_node(spec),
    })
    .unwrap_or_default()
}

pub fn decode_current(text: &str) -> Option<(String, DockSpec)> {
    let f: File = toml::from_str(text).ok()?;
    let spec = to_spec(f.root)?;
    let view = if f.view.trim().is_empty() { DEFAULT_VIEW.to_string() } else { f.view };
    Some((view, spec))
}

pub fn encode_preset(spec: &DockSpec, name: &str, description: &str, order: i32) -> String {
    toml::to_string_pretty(&File {
        name: name.to_string(),
        description: description.to_string(),
        order,
        view: String::new(),
        version: LAYOUT_VERSION,
        root: to_node(spec),
    })
    .unwrap_or_default()
}

pub fn decode_preset(text: &str) -> Option<(String, String, i32, DockSpec)> {
    let f: File = toml::from_str(text).ok()?;
    let spec = to_spec(f.root)?;
    Some((f.name, f.description, f.order, spec))
}

/// Загрузить сохранённую раскладку (или вид по умолчанию «Монтаж»).
pub fn load_current() -> (String, DockSpec) {
    load_current_in(&config_dir())
}

fn load_current_in(cfg: &Path) -> (String, DockSpec) {
    let path = cfg.join("layout.toml");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| decode_current(&t))
        .unwrap_or_else(|| (DEFAULT_VIEW.to_string(), builtin_spec(DEFAULT_VIEW)))
}

/// Сохранить текущую раскладку и активный ключ вида.
pub fn save_current(spec: &DockSpec, view: &str) {
    let _ = save_current_in(&config_dir(), spec, view);
}

fn save_current_in(cfg: &Path, spec: &DockSpec, view: &str) -> Result<(), String> {
    write_atomic(&cfg.join("layout.toml"), &encode_current(spec, view))
}

/// Автоматически сохранять раскладку при переносе/закрытии/переключении вкладок,
/// изменении долей разделителей и переключении пресета вида.
pub fn autosave(dock: DockState, view: RwSignal<String>) {
    let cfg = config_dir();
    use_effect(move || {
        let cur_view = view.get();
        let Some(spec) = dock.spec_tracked() else { return };
        if let Err(e) = save_current_in(&cfg, &spec, &cur_view) {
            tracing::warn!("сохранение раскладки: {e}");
        }
    });
}

/// Все пресеты: встроенные + пользовательские (переопределения перекрывают встроенные).
pub fn list() -> Vec<Preset> {
    list_in(&presets_dir())
}

fn list_in(dir: &Path) -> Vec<Preset> {
    let mut by_key: BTreeMap<String, Preset> = BTreeMap::new();
    for &(key, name, desc, order) in BUILTINS {
        by_key.insert(
            key.to_string(),
            Preset {
                key: key.to_string(),
                name: name.to_string(),
                description: desc.to_string(),
                order,
                spec: builtin_spec(key),
                path: None,
                builtin: true,
                modified: false,
            },
        );
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "toml") {
                continue;
            }
            let key = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if key.is_empty() {
                continue;
            }
            let Some((name, description, order, spec)) = std::fs::read_to_string(&p).ok().and_then(|t| decode_preset(&t)) else { continue };
            let builtin = BUILTINS.iter().any(|b| b.0 == key);
            let default_order = BUILTINS.iter().find(|b| b.0 == key).map(|b| b.3).unwrap_or(100);
            let name = if name.trim().is_empty() { key.clone() } else { name };
            by_key.insert(
                key.clone(),
                Preset {
                    key,
                    name,
                    description,
                    order: if order == 0 { default_order } else { order },
                    spec,
                    path: Some(p),
                    builtin,
                    modified: builtin,
                },
            );
        }
    }
    let mut out: Vec<Preset> = by_key.into_values().collect();
    out.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

pub fn find(key: &str) -> Option<Preset> {
    list().into_iter().find(|p| p.key == key)
}

fn new_key(name: &str, dir: &Path) -> String {
    let base: String = name.trim().chars().map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { '-' }).collect();
    let base = base.trim_matches('-').to_string();
    let base = if base.is_empty() { "вид".to_string() } else { base };
    let taken = |k: &str| dir.join(format!("{k}.toml")).exists() || BUILTINS.iter().any(|b| b.0 == k);
    let mut k = base.clone();
    let mut i = 2;
    while taken(&k) {
        k = format!("{base}-{i}");
        i += 1;
    }
    k
}

/// Сохранить текущую раскладку как новый пользовательский пресет.
pub fn save_new(spec: &DockSpec, name: &str, description: &str) -> Result<Preset, String> {
    save_new_in(&presets_dir(), spec, name, description)
}

fn save_new_in(dir: &Path, spec: &DockSpec, name: &str, description: &str) -> Result<Preset, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("укажите название пресета".into());
    }
    let key = new_key(name, dir);
    let order = list_in(dir).iter().map(|t| t.order).max().unwrap_or(40) + 10;
    let path = dir.join(format!("{key}.toml"));
    write_atomic(&path, &encode_preset(spec, name, description.trim(), order))?;
    Ok(Preset {
        key,
        name: name.to_string(),
        description: description.trim().to_string(),
        order,
        spec: spec.clone(),
        path: Some(path),
        builtin: false,
        modified: false,
    })
}

/// Перезаписать существующий пресет текущей раскладкой (встроенный сохраняется как переопределение).
pub fn overwrite(key: &str, spec: &DockSpec) -> Result<(), String> {
    overwrite_in(&presets_dir(), key, spec)
}

fn overwrite_in(dir: &Path, key: &str, spec: &DockSpec) -> Result<(), String> {
    let cur = list_in(dir).into_iter().find(|p| p.key == key).ok_or_else(|| format!("пресет «{key}» не найден"))?;
    let path = cur.path.unwrap_or_else(|| dir.join(format!("{key}.toml")));
    write_atomic(&path, &encode_preset(spec, &cur.name, &cur.description, cur.order))
}

/// Переименовать пресет (и обновить описание).
pub fn rename(key: &str, name: &str, description: &str) -> Result<(), String> {
    rename_in(&presets_dir(), key, name, description)
}

fn rename_in(dir: &Path, key: &str, name: &str, description: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("укажите название пресета".into());
    }
    let cur = list_in(dir).into_iter().find(|p| p.key == key).ok_or_else(|| format!("пресет «{key}» не найден"))?;
    let path = cur.path.unwrap_or_else(|| dir.join(format!("{key}.toml")));
    write_atomic(&path, &encode_preset(&cur.spec, name, description.trim(), cur.order))
}

/// Дублировать пресет (встроенный или пользовательский).
pub fn duplicate(key: &str) -> Result<Preset, String> {
    duplicate_in(&presets_dir(), key)
}

fn duplicate_in(dir: &Path, key: &str) -> Result<Preset, String> {
    let cur = list_in(dir).into_iter().find(|p| p.key == key).ok_or_else(|| format!("пресет «{key}» не найден"))?;
    let name = format!("{} (копия)", cur.name);
    let k = new_key(&name, dir);
    let path = dir.join(format!("{k}.toml"));
    write_atomic(&path, &encode_preset(&cur.spec, &name, &cur.description, cur.order + 1))?;
    Ok(Preset {
        key: k,
        name,
        description: cur.description,
        order: cur.order + 1,
        spec: cur.spec,
        path: Some(path),
        builtin: false,
        modified: false,
    })
}

/// Удалить пользовательский пресет (или сбросить переопределение встроенного).
pub fn delete(key: &str) -> Result<(), String> {
    delete_in(&presets_dir(), key)
}

fn delete_in(dir: &Path, key: &str) -> Result<(), String> {
    let cur = list_in(dir).into_iter().find(|p| p.key == key).ok_or_else(|| format!("пресет «{key}» не найден"))?;
    let Some(path) = cur.path else {
        return Err("встроенный пресет нельзя удалить".into());
    };
    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_filter_unknown() {
        let spec = DockSpec::column(
            DockSpec::row(
                DockSpec::Tabs { panels: vec!["media".into(), "effects".into()], active: 1, name: String::new() },
                DockSpec::tabs(["viewer", "graph"]).named("viewer"),
                0.31,
            ),
            DockSpec::tabs(["timeline", "curves"]),
            0.62,
        );
        let text = encode_current(&spec, "motion");
        let (view, back) = decode_current(&text).unwrap();
        assert_eq!(view, "motion");
        assert_eq!(back, spec);
        // неизвестная панель отбрасывается
        let with_unknown = text.replace("\"curves\"", "\"curves\", \"ghost_panel\"");
        let (_, clean) = decode_current(&with_unknown).unwrap();
        assert_eq!(clean, spec);
        assert!(decode_current("некорректный toml :::").is_none());
    }

    #[test]
    fn current_and_presets_crud() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path();
        let presets = cfg.join("layouts");

        // по умолчанию — «edit»
        let (v, s) = load_current_in(cfg);
        assert_eq!(v, "edit");
        assert_eq!(s, builtin_spec("edit"));

        // сохранение и восстановление текущей раскладки с изменённой долей и вкладкой
        let custom = DockSpec::row(DockSpec::tabs(["layers", "media"]), DockSpec::tabs(["viewer", "timeline"]), 0.42);
        save_current_in(cfg, &custom, "custom-1").unwrap();
        let (v2, s2) = load_current_in(cfg);
        assert_eq!(v2, "custom-1");
        assert_eq!(s2, custom);

        // список по умолчанию содержит 4 встроенных
        let base = list_in(&presets);
        assert_eq!(base.len(), BUILTINS.len());
        assert!(base.iter().all(|p| p.builtin && !p.modified));

        // сохранить новый пресет
        let created = save_new_in(&presets, &custom, "Мой монтаж", "Две колонки").unwrap();
        assert!(!created.builtin);
        assert_eq!(created.name, "Мой монтаж");
        assert_eq!(list_in(&presets).len(), BUILTINS.len() + 1);

        // перезаписать и переименовать
        let updated = DockSpec::tabs(["viewer", "timeline"]);
        overwrite_in(&presets, &created.key, &updated).unwrap();
        rename_in(&presets, &created.key, "Монтаж 2", "Обновлён").unwrap();
        let found = list_in(&presets).into_iter().find(|p| p.key == created.key).unwrap();
        assert_eq!(found.name, "Монтаж 2");
        assert_eq!(found.description, "Обновлён");
        assert_eq!(found.spec, updated);

        // дублировать
        let copy = duplicate_in(&presets, &created.key).unwrap();
        assert_ne!(copy.key, created.key);
        assert_eq!(copy.name, "Монтаж 2 (копия)");
        assert_eq!(copy.spec, updated);

        // перезаписать встроенный → становится modified, delete возвращает его к встроенному
        overwrite_in(&presets, "edit", &updated).unwrap();
        let edit_mod = list_in(&presets).into_iter().find(|p| p.key == "edit").unwrap();
        assert!(edit_mod.builtin && edit_mod.modified);
        assert_eq!(edit_mod.spec, updated);
        delete_in(&presets, "edit").unwrap();
        let edit_rst = list_in(&presets).into_iter().find(|p| p.key == "edit").unwrap();
        assert!(edit_rst.builtin && !edit_rst.modified);
        assert_eq!(edit_rst.spec, builtin_spec("edit"));

        // удалить пользовательские
        delete_in(&presets, &created.key).unwrap();
        delete_in(&presets, &copy.key).unwrap();
        assert_eq!(list_in(&presets).len(), BUILTINS.len());
    }
}
