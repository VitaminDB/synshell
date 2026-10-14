//! Закреплённые «Пуска» с группами: значок, брошенный на другой значок,
//! объединяется с ним в группу (как на домашнем экране телефона); группа —
//! `group:<id>` в `[launcher] favorites`, состав и имя — в `launcher-groups.toml`.
//! Здесь — только правки списка; рисует их `start_menu`.

use std::collections::BTreeMap;

use synshell_common::config::{load_launcher_groups, save_launcher_groups, Launcher, LauncherGroup, GROUP_PREFIX};

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pins {
    pub fav: Vec<String>,
    pub groups: BTreeMap<String, LauncherGroup>,
}

/// id группы из метки `group:<id>`.
pub fn group_id(token: &str) -> Option<&str> {
    token.strip_prefix(GROUP_PREFIX)
}

pub fn group_token(id: &str) -> String {
    format!("{GROUP_PREFIX}{id}")
}

impl Pins {
    pub fn from_cfg(l: &Launcher) -> Self {
        let mut p = Self { fav: l.favorites.clone(), groups: load_launcher_groups() };
        p.normalize();
        p
    }

    /// Записать: сначала группы (файл), потом `favorites` (config.toml) — чтобы
    /// перечитанный конфиг уже видел новые группы.
    pub fn save(&self) {
        if let Err(e) = save_launcher_groups(&self.groups) {
            log::error!("«Пуск»: группы: {e}");
        }
        let arr = toml_edit::Value::Array(self.fav.iter().map(|a| toml_edit::Value::from(a.as_str())).collect());
        if let Err(e) = synshell_common::config_edit::set_value(&["launcher", "favorites"], arr) {
            log::error!("«Пуск»: {e:#}");
        }
        crate::reload_after_write();
        let ctx = crate::ctx::ShellCtx::get();
        ctx.pins_rev.set(ctx.pins_rev.get_untracked() + 1);
    }

    /// Приложения метки: группа — её состав, приложение — оно само.
    fn apps_of(&self, token: &str) -> Vec<String> {
        match group_id(token) {
            Some(id) => self.groups.get(id).map(|g| g.items.clone()).unwrap_or_default(),
            None => vec![token.to_string()],
        }
    }

    fn new_id(&self) -> String {
        (1..).map(|n| format!("g{n}")).find(|id| !self.groups.contains_key(id)).unwrap()
    }

    /// Бросили `src` на `dst`: на группу — `src` (или вся его группа) в неё,
    /// на приложение — новая группа на месте `dst`. `name` — имя новой группы.
    pub fn merge(&mut self, src: &str, dst: &str, name: impl FnOnce(&[String]) -> String) -> Option<String> {
        if src == dst || !self.fav.iter().any(|f| f == src) || !self.fav.iter().any(|f| f == dst) {
            return None;
        }
        let moved = self.apps_of(src);
        let gid = match group_id(dst) {
            Some(id) => {
                let g = self.groups.get_mut(id)?;
                for a in moved {
                    if !g.items.contains(&a) {
                        g.items.push(a);
                    }
                }
                id.to_string()
            }
            None => {
                // новая группа на месте цели: цель, затем перенесённое
                let id = self.new_id();
                let mut items = vec![dst.to_string()];
                items.extend(moved.into_iter().filter(|a| a != dst));
                let pos = self.fav.iter().position(|f| f == dst)?;
                self.fav[pos] = group_token(&id);
                self.groups.insert(id.clone(), LauncherGroup { name: name(&items), items });
                id
            }
        };
        if let Some(sid) = group_id(src) {
            self.groups.remove(sid);
        }
        self.fav.retain(|f| f != src);
        self.normalize();
        Some(gid)
    }

    /// Переставить `src` к `dst`: перед ним или после.
    pub fn reorder(&mut self, src: &str, dst: &str, after: bool) -> bool {
        if src == dst {
            return false;
        }
        let Some(from) = self.fav.iter().position(|f| f == src) else { return false };
        let item = self.fav.remove(from);
        let Some(to) = self.fav.iter().position(|f| f == dst) else {
            self.fav.insert(from, item);
            return false;
        };
        self.fav.insert(if after { to + 1 } else { to }, item);
        true
    }

    /// Переставить приложение внутри группы.
    pub fn reorder_in_group(&mut self, gid: &str, src: &str, dst: &str, after: bool) -> bool {
        let Some(g) = self.groups.get_mut(gid) else { return false };
        if src == dst {
            return false;
        }
        let Some(from) = g.items.iter().position(|a| a == src) else { return false };
        let item = g.items.remove(from);
        let to = g.items.iter().position(|a| a == dst).unwrap_or(g.items.len());
        g.items.insert(if after { (to + 1).min(g.items.len()) } else { to }, item);
        true
    }

    /// Вынуть приложение из группы — встаёт в закреплённые сразу за группой.
    pub fn take_out(&mut self, gid: &str, app: &str) -> bool {
        let Some(g) = self.groups.get_mut(gid) else { return false };
        let before = g.items.len();
        g.items.retain(|a| a != app);
        if g.items.len() == before {
            return false;
        }
        let token = group_token(gid);
        let at = self.fav.iter().position(|f| *f == token).map(|i| i + 1).unwrap_or(self.fav.len());
        self.fav.retain(|f| f != app);
        let at = at.min(self.fav.len());
        self.fav.insert(at, app.to_string());
        self.normalize();
        true
    }

    /// Убрать приложение из группы совсем (открепить).
    pub fn remove_from_group(&mut self, gid: &str, app: &str) {
        if let Some(g) = self.groups.get_mut(gid) {
            g.items.retain(|a| a != app);
        }
        self.normalize();
    }

    pub fn rename(&mut self, gid: &str, name: &str) -> bool {
        let name = name.trim();
        match self.groups.get_mut(gid) {
            Some(g) if !name.is_empty() && g.name != name => {
                g.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Разгруппировать: приложения встают на место группы.
    pub fn dissolve(&mut self, gid: &str) {
        let token = group_token(gid);
        let items = self.groups.remove(gid).map(|g| g.items).unwrap_or_default();
        if let Some(at) = self.fav.iter().position(|f| *f == token) {
            self.fav.remove(at);
            let mut at = at;
            for a in items {
                if !self.fav.contains(&a) {
                    self.fav.insert(at, a);
                    at += 1;
                }
            }
        }
        self.normalize();
    }

    /// Открепить метку (приложение или группу целиком).
    pub fn unpin(&mut self, token: &str) {
        self.fav.retain(|f| f != token);
        if let Some(id) = group_id(token) {
            self.groups.remove(id);
        }
    }

    /// Порядок: группа из одного приложения — само приложение, пустая — прочь,
    /// группы без метки в закреплённых и метки без группы — прочь, без повторов.
    fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.fav.retain(|f| seen.insert(f.clone()));
        let mut out = Vec::with_capacity(self.fav.len());
        for f in std::mem::take(&mut self.fav) {
            match group_id(&f) {
                Some(id) => match self.groups.get(id).map(|g| g.items.len()) {
                    Some(0) | None => {
                        self.groups.remove(id);
                    }
                    Some(1) => {
                        let a = self.groups.remove(id).unwrap().items.remove(0);
                        if !out.contains(&a) {
                            out.push(a);
                        }
                    }
                    Some(_) => out.push(f),
                },
                None => out.push(f),
            }
        }
        self.fav = out;
        let used: Vec<String> = self.fav.iter().filter_map(|f| group_id(f).map(String::from)).collect();
        self.groups.retain(|id, _| used.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(fav: &[&str]) -> Pins {
        Pins { fav: fav.iter().map(|s| s.to_string()).collect(), groups: BTreeMap::new() }
    }

    fn named(_: &[String]) -> String {
        "Группа".into()
    }

    #[test]
    fn merge_two_apps_makes_group_in_place() {
        let mut p = pins(&["a", "b", "c"]);
        let g = p.merge("c", "a", named).unwrap();
        assert_eq!(p.fav, vec![group_token(&g), "b".into()]);
        assert_eq!(p.groups[&g].items, vec!["a".to_string(), "c".into()]);
        assert_eq!(p.groups[&g].name, "Группа");
    }

    #[test]
    fn merge_into_group_and_group_into_group() {
        let mut p = pins(&["a", "b", "c", "d"]);
        let g1 = p.merge("b", "a", named).unwrap();
        p.merge("c", &group_token(&g1), named).unwrap();
        assert_eq!(p.groups[&g1].items, vec!["a".to_string(), "b".into(), "c".into()]);
        // d + ещё одна группа, потом группа в группу
        p.fav.push("e".into());
        let g2 = p.merge("e", "d", named).unwrap();
        p.merge(&group_token(&g2), &group_token(&g1), named).unwrap();
        assert_eq!(p.fav, vec![group_token(&g1)]);
        assert_eq!(p.groups.len(), 1);
        assert_eq!(p.groups[&g1].items.len(), 5);
    }

    #[test]
    fn group_onto_app_absorbs_app() {
        let mut p = pins(&["a", "b", "x"]);
        let g = p.merge("b", "a", named).unwrap();
        let g2 = p.merge(&group_token(&g), "x", named).unwrap();
        assert_eq!(p.fav, vec![group_token(&g2)]);
        assert_eq!(p.groups[&g2].items, vec!["x".to_string(), "a".into(), "b".into()]);
        assert!(!p.groups.contains_key(&g) || g == g2);
    }

    #[test]
    fn take_out_last_but_one_dissolves() {
        let mut p = pins(&["a", "b", "c"]);
        let g = p.merge("b", "a", named).unwrap();
        assert!(p.take_out(&g, "b"));
        // в группе остался один — группа стала приложением
        assert_eq!(p.fav, vec!["a".to_string(), "b".into(), "c".into()]);
        assert!(p.groups.is_empty());
    }

    #[test]
    fn reorder_and_dissolve() {
        let mut p = pins(&["a", "b", "c", "d"]);
        assert!(p.reorder("d", "a", false));
        assert_eq!(p.fav, vec!["d", "a", "b", "c"]);
        assert!(p.reorder("d", "c", true));
        assert_eq!(p.fav, vec!["a", "b", "c", "d"]);
        let g = p.merge("c", "b", named).unwrap();
        p.dissolve(&g);
        assert_eq!(p.fav, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn rename_and_in_group_order() {
        let mut p = pins(&["a", "b", "c"]);
        let g = p.merge("b", "a", named).unwrap();
        p.merge("c", &group_token(&g), named);
        assert!(p.rename(&g, "  Игры "));
        assert_eq!(p.groups[&g].name, "Игры");
        assert!(!p.rename(&g, "   "));
        assert!(p.reorder_in_group(&g, "c", "a", false));
        assert_eq!(p.groups[&g].items, vec!["c", "a", "b"]);
    }

    #[test]
    fn stale_entries_cleaned() {
        let mut groups = BTreeMap::new();
        groups.insert("g1".into(), LauncherGroup { name: "x".into(), items: vec!["a".into()] });
        groups.insert("g9".into(), LauncherGroup { name: "y".into(), items: vec!["q".into(), "w".into()] });
        let mut p = Pins { fav: vec!["group:g1".into(), "group:g2".into(), "b".into(), "b".into()], groups };
        p.normalize();
        assert_eq!(p.fav, vec!["a", "b"]);
        assert!(p.groups.is_empty());
    }
}
