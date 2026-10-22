//! Черновик темы: `theme.toml` (через toml_edit — комментарии и порядок
//! ключей сохраняются) и MSS-слои. Встроенная тема при первой правке
//! копируется в свою: оригинал не меняется.

use std::collections::BTreeMap;
use std::path::PathBuf;

use synshell_common::theme::{self, Source, Theme};
use toml_edit::{value, DocumentMut, Item, Table};

/// Откуда черновик.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// Встроенная тема (до первой правки).
    Builtin,
    /// Каталог пользователя (`~/.config/synshell/themes/<id>`) или системный.
    Dir(PathBuf),
    /// Новая, ещё не сохранённая.
    New,
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub id: String,
    pub origin: Origin,
    pub doc: DocumentMut,
    /// Слой → текст MSS (`shell`, `settings`, `files`, `synpkg`, …).
    pub mss: BTreeMap<String, String>,
}

/// Снимок для отмены.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: String,
    pub toml: String,
    pub mss: BTreeMap<String, String>,
}


impl Draft {
    pub fn from_theme(t: &Theme) -> Self {
        let doc = t.toml_text.parse::<DocumentMut>().unwrap_or_default();
        let origin = match &t.source {
            Source::Builtin => Origin::Builtin,
            Source::Dir(d) => Origin::Dir(d.clone()),
        };
        Self { id: t.id.clone(), origin, doc, mss: t.mss.clone() }
    }

    /// Новая тема по образцу `base` (или пустая).
    pub fn new_from(base: Option<&Theme>, id: String, name: &str) -> Self {
        let mut d = match base {
            Some(t) => Self::from_theme(t),
            None => Self { id: String::new(), origin: Origin::New, doc: DocumentMut::new(), mss: BTreeMap::new() },
        };
        d.id = id;
        d.origin = Origin::New;
        d.set_meta("name", name);
        if base.is_none() {
            d.doc["dark"] = Item::Table(Table::new());
        }
        d
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot { id: self.id.clone(), toml: self.doc.to_string(), mss: self.mss.clone() }
    }

    pub fn restore(&mut self, s: &Snapshot) {
        self.id = s.id.clone();
        self.doc = s.toml.parse().unwrap_or_default();
        self.mss = s.mss.clone();
    }

    pub fn is_builtin(&self) -> bool {
        self.origin == Origin::Builtin
    }

    // ── Метаданные ───────────────────────────────────────────────────────

    pub fn meta(&self, key: &str) -> String {
        self.doc.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    pub fn set_meta(&mut self, key: &str, v: &str) {
        // Метаданные — в начале файла, до таблиц: toml_edit кладёт ключи
        // корня перед таблицами сам.
        self.doc[key] = value(v);
    }

    pub fn name(&self) -> String {
        let n = self.meta("name");
        if n.is_empty() {
            self.id.clone()
        } else {
            n
        }
    }

    // ── Варианты ─────────────────────────────────────────────────────────

    pub fn has_variant(&self, variant: &str) -> bool {
        self.doc.get(variant).is_some_and(|i| i.is_table())
    }

    /// Вариант, который реально покажется для `dark` (как `Theme::variant`).
    pub fn effective_variant(&self, dark: bool) -> &'static str {
        match (dark, self.has_variant("dark"), self.has_variant("light")) {
            (true, true, _) => "dark",
            (false, _, true) => "light",
            (_, true, false) => "dark",
            (_, false, true) => "light",
            _ => "dark",
        }
    }

    fn variant_table(&mut self, variant: &str) -> &mut Table {
        if !self.has_variant(variant) {
            self.doc[variant] = Item::Table(Table::new());
        }
        self.doc[variant].as_table_mut().expect("таблица варианта")
    }

    /// Значение поля варианта (`bg`, `wallpaper`, …).
    pub fn get(&self, variant: &str, key: &str) -> Option<String> {
        self.doc.get(variant)?.get(key)?.as_str().map(str::to_string)
    }

    /// Задать поле варианта; `None` — убрать (будет значение по умолчанию).
    pub fn set(&mut self, variant: &str, key: &str, v: Option<&str>) {
        let t = self.variant_table(variant);
        match v {
            Some(v) => {
                // Сохранить комментарий в конце строки, если он был.
                if let Some(old) = t.get_mut(key).and_then(Item::as_value_mut) {
                    let decor = old.decor().clone();
                    *old = toml_edit::Value::from(v);
                    *old.decor_mut() = decor;
                } else {
                    t[key] = value(v);
                }
            }
            None => {
                t.remove(key);
            }
        }
    }

    /// Свои переменные варианта (`[dark.vars]`).
    pub fn vars(&self, variant: &str) -> Vec<(String, String)> {
        self.doc
            .get(variant)
            .and_then(|v| v.get("vars"))
            .and_then(Item::as_table_like)
            .map(|t| t.iter().filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default()
    }

    pub fn get_var(&self, variant: &str, name: &str) -> Option<String> {
        self.doc.get(variant)?.get("vars")?.get(name.trim_start_matches('-'))?.as_str().map(str::to_string)
    }

    pub fn set_var(&mut self, variant: &str, name: &str, v: Option<&str>) {
        let name = name.trim_start_matches('-');
        let t = self.variant_table(variant);
        if !t.contains_key("vars") {
            let mut vars = Table::new();
            vars.set_implicit(false);
            t.insert("vars", Item::Table(vars));
        }
        let vars = t["vars"].as_table_like_mut().expect("vars");
        match v {
            Some(v) => {
                vars.insert(name, value(v));
            }
            None => {
                vars.remove(name);
            }
        }
    }

    /// Создать вариант `to` из `from` (копия с перевёрнутой светлотой цветов).
    pub fn derive_variant(&mut self, from: &str, to: &str) {
        let to_dark = to == "dark";
        let mut fields: Vec<(String, String)> = Vec::new();
        if let Some(t) = self.doc.get(from).and_then(Item::as_table_like) {
            for (k, v) in t.iter() {
                if let Some(s) = v.as_str() {
                    fields.push((k.to_string(), s.to_string()));
                }
            }
        }
        let vars = self.vars(from);
        let flip = |s: &str| -> String {
            match synshell_common::config::Rgba::parse(s) {
                Some(c) => crate::color::flip(c, to_dark).hex(),
                None => flip_gradient(s, to_dark),
            }
        };
        self.doc.remove(to);
        for (k, v) in fields {
            let nv = match k.as_str() {
                "accent_fg" => continue,
                "shadow" => if to_dark { "#00000080".to_string() } else { "#1a203026".to_string() },
                "titlebar" | "titlebar_inactive" if synshell_common::config::Rgba::parse(&v).is_none() => v,
                _ => flip(&v),
            };
            self.set(to, &k, Some(&nv));
        }
        for (k, v) in vars {
            let nv = flip(&v);
            self.set_var(to, &k, Some(&nv));
        }
    }

    // ── Рекомендации ([appearance], [decorations], [panel]) ──────────────

    pub fn rec(&self, table: &str, key: &str) -> Option<f64> {
        let v = self.doc.get(table)?.get(key)?;
        v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
    }

    pub fn rec_str(&self, table: &str, key: &str) -> Option<String> {
        self.doc.get(table)?.get(key)?.as_str().map(str::to_string)
    }

    pub fn set_rec(&mut self, table: &str, key: &str, v: Option<toml_edit::Value>) {
        if !self.doc.contains_key(table) {
            self.doc[table] = Item::Table(Table::new());
        }
        let t = self.doc[table].as_table_like_mut().expect("таблица рекомендаций");
        match v {
            Some(v) => {
                t.insert(key, Item::Value(v));
            }
            None => {
                t.remove(key);
            }
        }
    }

    // ── Файлы ───────────────────────────────────────────────────────────

    pub fn files(&self) -> Vec<(String, String)> {
        let mut v = vec![("theme.toml".to_string(), self.doc.to_string())];
        v.extend(self.mss.iter().filter(|(_, t)| !t.trim().is_empty()).map(|(l, t)| (theme::file_of_layer(l), t.clone())));
        v
    }

    /// Тема для показа (ошибка — сообщение разбора).
    pub fn to_theme(&self) -> Result<Theme, String> {
        let src = match &self.origin {
            Origin::Dir(d) => Source::Dir(d.clone()),
            _ => Source::Builtin,
        };
        Theme::from_files(&self.id, src, &self.files())
    }

    /// Каталог сохранения: `~/.config/synshell/themes/<id>`.
    pub fn save_dir(&self) -> PathBuf {
        theme::user_dir().join(&self.id)
    }

    /// Сохранить в каталог пользователя. Слои, которых больше нет, удаляются.
    pub fn save(&mut self) -> std::io::Result<PathBuf> {
        let dir = self.save_dir();
        std::fs::create_dir_all(dir.join("apps"))?;
        let files = self.files();
        // Убрать старые .mss, которых нет в черновике.
        for sub in [dir.clone(), dir.join("apps")] {
            if let Ok(rd) = std::fs::read_dir(&sub) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "mss") {
                        let rel = p.strip_prefix(&dir).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
                        if !files.iter().any(|(n, _)| *n == rel) {
                            let _ = std::fs::remove_file(&p);
                        }
                    }
                }
            }
        }
        for (name, text) in &files {
            let p = dir.join(name);
            let tmp = p.with_extension("tmp~");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &p)?;
        }
        let _ = std::fs::remove_dir(dir.join("apps"));
        self.origin = Origin::Dir(dir.clone());
        Ok(dir)
    }

    // ── Обмен одним файлом ──────────────────────────────────────────────

    /// Тема одним файлом `.syntheme` (TOML: `id` и `[files]`).
    pub fn export_bundle(&self) -> String {
        let mut doc = DocumentMut::new();
        doc["format"] = value("syntheme 1");
        doc["id"] = value(self.id.as_str());
        let mut files = Table::new();
        for (name, text) in self.files() {
            files[name.as_str()] = value(text);
        }
        doc["files"] = Item::Table(files);
        doc.to_string()
    }

    pub fn import_bundle(text: &str) -> Result<Draft, String> {
        let doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        let id = doc.get("id").and_then(|v| v.as_str()).ok_or("нет id")?.to_string();
        let files: Vec<(String, String)> = doc
            .get("files")
            .and_then(Item::as_table_like)
            .ok_or("нет [files]")?
            .iter()
            .filter_map(|(k, v)| Some((k.to_string(), v.as_str()?.to_string())))
            .filter(|(k, _)| !k.contains("..") && !k.starts_with('/'))
            .collect();
        let t = Theme::from_files(&safe_id(&id), Source::Builtin, &files)?;
        let mut d = Draft::from_theme(&t);
        d.origin = Origin::New;
        Ok(d)
    }
}

/// Имя каталога из произвольной строки: латиница, цифры, `-`.
pub fn safe_id(s: &str) -> String {
    let mut out = String::new();
    for ch in s.trim().to_lowercase().chars() {
        let c = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'а' => Some('a'), 'б' => Some('b'), 'в' => Some('v'), 'г' => Some('g'), 'д' => Some('d'),
            'е' | 'ё' | 'э' => Some('e'), 'з' => Some('z'), 'и' | 'й' | 'ы' => Some('i'), 'к' => Some('k'),
            'л' => Some('l'), 'м' => Some('m'), 'н' => Some('n'), 'о' => Some('o'), 'п' => Some('p'),
            'р' => Some('r'), 'с' => Some('s'), 'т' => Some('t'), 'у' => Some('u'), 'ф' => Some('f'),
            'х' => Some('h'), 'ц' => Some('c'), 'ч' => Some('c'), 'ш' | 'щ' => Some('s'), 'ю' => Some('u'),
            'я' => Some('a'), 'ж' => Some('z'),
            _ => None,
        };
        match c {
            Some(c) => out.push(c),
            None if !out.ends_with('-') && !out.is_empty() => out.push('-'),
            None => {}
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "my-theme".into()
    } else {
        out
    }
}

/// Свободное имя каталога: `base`, `base-2`, `base-3`, …
pub fn free_id(base: &str) -> String {
    let taken = |id: &str| theme::user_dir().join(id).exists() || theme::builtin_ids().any(|b| b == id);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap()
}

/// Перевернуть цвета внутри градиента.
fn flip_gradient(s: &str, to_dark: bool) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let len = tail[1..].chars().take_while(|c| c.is_ascii_hexdigit()).count();
        let hex = &tail[..1 + len];
        match synshell_common::config::Rgba::parse(hex) {
            Some(c) => out.push_str(&crate::color::flip(c, to_dark).hex()),
            None => out.push_str(hex),
        }
        rest = &tail[1 + len..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_keeps_comments() {
        let t = Theme::find("tokyo-night").unwrap();
        let mut d = Draft::from_theme(&t);
        d.set("dark", "accent", Some("#ff0000"));
        d.set_var("dark", "neon", Some("#ff000066"));
        let s = d.doc.to_string();
        assert!(s.contains("# Толщина плавающей панели"), "{s}");
        assert!(s.contains("accent = \"#ff0000\""));
        let th = d.to_theme().unwrap();
        assert_eq!(th.dark.as_ref().unwrap().accent.as_deref(), Some("#ff0000"));
        assert_eq!(th.dark.as_ref().unwrap().vars.get("neon").map(String::as_str), Some("#ff000066"));
        assert!(th.app_mss("shell").contains(".ws-active"));
    }

    #[test]
    fn derive_light() {
        let t = Theme::find("dracula").unwrap();
        let mut d = Draft::from_theme(&t);
        assert!(!d.has_variant("light"));
        d.derive_variant("dark", "light");
        let th = d.to_theme().unwrap();
        let l = th.light.unwrap();
        let bg = synshell_common::config::Rgba::parse(l.bg.as_deref().unwrap()).unwrap();
        assert!(crate::color::luminance(bg) > 0.5, "{:?}", l.bg);
    }

    #[test]
    fn bundle_roundtrip() {
        let t = Theme::find("nord").unwrap();
        let d = Draft::from_theme(&t);
        let b = d.export_bundle();
        let back = Draft::import_bundle(&b).unwrap();
        assert_eq!(back.id, "nord");
        assert_eq!(back.mss, d.mss);
        assert_eq!(back.doc.to_string(), d.doc.to_string());
    }

    #[test]
    fn ids() {
        assert_eq!(safe_id("Моя Тема 2"), "moa-tema-2");
        assert_eq!(safe_id("  "), "my-theme");
    }
}
