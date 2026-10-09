//! Очередь изменений: задачи по пакетам (установить, удалить, обновить) и их
//! слияние. На пакет — не больше одной задачи: повтор не дублируется,
//! «установить» и «удалить» гасят друг друга, в остальных спорах побеждает
//! последняя. «Применить» сводит очередь в план — по транзакции на вид
//! изменений: одно удаление (`-R`), одна транзакция pacman (обновление
//! вместе с установкой — `-Syu a b`), один проход AUR.

use synsystem::packages::Op;
use syngui::t;

/// Что сделать с пакетом из очереди.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QAct {
    Install,
    InstallAur,
    Remove,
    /// Обновить из репозитория (пакет из списка обновлений).
    Upgrade,
    /// Пересобрать новую версию из AUR.
    UpgradeAur,
}

impl QAct {
    pub fn installs(self) -> bool {
        matches!(self, QAct::Install | QAct::InstallAur)
    }

    pub fn upgrades(self) -> bool {
        matches!(self, QAct::Upgrade | QAct::UpgradeAur)
    }

    /// Метка задачи и класс плашки.
    pub fn chip(self) -> (String, &'static str) {
        match self {
            QAct::Install => (t!("к установке"), "chip-q"),
            QAct::InstallAur => (t!("к сборке из AUR"), "chip-q"),
            QAct::Remove => (t!("к удалению"), "chip-rm"),
            QAct::Upgrade | QAct::UpgradeAur => (t!("к обновлению"), "chip-q"),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct QItem {
    pub name: String,
    pub act: QAct,
}

impl QItem {
    pub fn new(name: impl Into<String>, act: QAct) -> Self {
        Self { name: name.into(), act }
    }
}

/// Что стало с добавленной задачей.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Merged {
    Added,
    /// Такая (или равносильная) задача уже стоит.
    Same,
    /// Задача на пакет заменена новой.
    Replaced,
    /// Противоположные задачи погасили друг друга — пакет снят с очереди.
    Cancelled,
}

/// Добавить задачу в очередь с учётом уже стоящей на тот же пакет.
pub fn merge(q: &mut Vec<QItem>, it: QItem) -> Merged {
    let Some(i) = q.iter().position(|x| x.name == it.name) else {
        q.push(it);
        return Merged::Added;
    };
    let (old, new) = (q[i].act, it.act);
    if old == new {
        return Merged::Same;
    }
    // Установить ↔ удалить: пакет остаётся как был.
    if (old.installs() && new == QAct::Remove) || (old == QAct::Remove && new.installs()) {
        q.remove(i);
        return Merged::Cancelled;
    }
    // Установка и обновление дают одно — новую версию.
    if (old.installs() && new.upgrades()) || (old.upgrades() && new.installs()) {
        return Merged::Same;
    }
    // Удалить ↔ обновить, репозиторий ↔ AUR — по последней.
    q[i].act = new;
    Merged::Replaced
}

/// Итог очереди по видам изменений (в порядке добавления).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Plan {
    pub remove: Vec<String>,
    pub install: Vec<String>,
    pub upgrade: Vec<String>,
    pub aur_install: Vec<String>,
    pub aur_upgrade: Vec<String>,
}

impl Plan {
    pub fn of(q: &[QItem]) -> Self {
        let mut p = Plan::default();
        for it in q {
            let v = match it.act {
                QAct::Remove => &mut p.remove,
                QAct::Install => &mut p.install,
                QAct::Upgrade => &mut p.upgrade,
                QAct::InstallAur => &mut p.aur_install,
                QAct::UpgradeAur => &mut p.aur_upgrade,
            };
            if !v.contains(&it.name) {
                v.push(it.name.clone());
            }
        }
        p
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        self.remove.len() + self.install.len() + self.upgrade.len() + self.aur_install.len() + self.aur_upgrade.len()
    }

    /// Шаги после удаления (оно идёт отдельно — с проверкой зависимостей):
    /// одна транзакция pacman и один проход AUR. `updates` — все известные
    /// обновления репозиториев: невыбранные пропускаются (`--ignore`), как и
    /// `ignore` из настроек, если пакет не ставится явно.
    pub fn steps(&self, updates: &[String], ignore: &[String]) -> Vec<Op> {
        let mut ops = Vec::new();
        if !self.upgrade.is_empty() {
            let wanted = |n: &String| self.upgrade.contains(n) || self.install.contains(n);
            let mut skip: Vec<String> = Vec::new();
            for n in updates.iter().chain(ignore) {
                if !wanted(n) && !skip.contains(n) {
                    skip.push(n.clone());
                }
            }
            ops.push(Op::Upgrade { aur: false, ignore: skip, install: self.install.clone() });
        } else if !self.install.is_empty() {
            ops.push(Op::Install(self.install.clone()));
        }
        let aur: Vec<String> = self.aur_install.iter().chain(&self.aur_upgrade).cloned().collect();
        if !aur.is_empty() {
            ops.push(Op::InstallAur(aur));
        }
        ops
    }

    /// Заголовок задания: «Очередь: удаление 1 пакета, обновление 3 пакетов».
    pub fn title(&self) -> String {
        let mut parts = Vec::new();
        let n_inst = self.install.len() + self.aur_install.len();
        let n_up = self.upgrade.len() + self.aur_upgrade.len();
        if !self.remove.is_empty() {
            parts.push(t!("удаление {v}", v = crate::packages_word(self.remove.len())));
        }
        if n_inst > 0 {
            parts.push(t!("установка {v}", v = crate::packages_word(n_inst)));
        }
        if n_up > 0 {
            parts.push(t!("обновление {v}", v = crate::packages_word(n_up)));
        }
        t!("Очередь: {v}", v = parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use QAct::*;

    fn q(items: &[(&str, QAct)]) -> Vec<QItem> {
        let mut v = Vec::new();
        for (n, a) in items {
            merge(&mut v, QItem::new(*n, *a));
        }
        v
    }

    #[test]
    fn repeat_is_not_duplicated() {
        let mut v = q(&[("a", Install)]);
        assert_eq!(merge(&mut v, QItem::new("a", Install)), Merged::Same);
        assert_eq!(v, vec![QItem::new("a", Install)]);
    }

    #[test]
    fn install_then_remove_cancels() {
        let mut v = q(&[("a", Install), ("b", Install)]);
        assert_eq!(merge(&mut v, QItem::new("a", Remove)), Merged::Cancelled);
        assert_eq!(v, vec![QItem::new("b", Install)]);
        let mut v = q(&[("x", InstallAur)]);
        assert_eq!(merge(&mut v, QItem::new("x", Remove)), Merged::Cancelled);
        assert!(v.is_empty());
    }

    #[test]
    fn remove_then_install_keeps_installed() {
        let mut v = q(&[("a", Remove)]);
        assert_eq!(merge(&mut v, QItem::new("a", Install)), Merged::Cancelled);
        assert!(v.is_empty());
    }

    #[test]
    fn remove_excludes_from_upgrade_and_back() {
        let mut v = q(&[("a", Upgrade), ("b", Upgrade)]);
        assert_eq!(merge(&mut v, QItem::new("a", Remove)), Merged::Replaced);
        assert_eq!(Plan::of(&v).upgrade, vec!["b".to_string()]);
        assert_eq!(Plan::of(&v).remove, vec!["a".to_string()]);
        // Последняя побеждает: снова «обновить».
        assert_eq!(merge(&mut v, QItem::new("a", Upgrade)), Merged::Replaced);
        assert_eq!(Plan::of(&v).upgrade, vec!["a".to_string(), "b".to_string()]);
        assert!(Plan::of(&v).remove.is_empty());
    }

    #[test]
    fn install_and_upgrade_are_same() {
        let mut v = q(&[("a", Upgrade)]);
        assert_eq!(merge(&mut v, QItem::new("a", Install)), Merged::Same);
        assert_eq!(v, vec![QItem::new("a", Upgrade)]);
    }

    #[test]
    fn source_switch_last_wins() {
        let mut v = q(&[("a", Install)]);
        assert_eq!(merge(&mut v, QItem::new("a", InstallAur)), Merged::Replaced);
        assert_eq!(v, vec![QItem::new("a", InstallAur)]);
    }

    #[test]
    fn steps_merge_into_transactions() {
        // Исходный язык строк — русский (формы числа по его правилам).
        syngui::i18n::set_source_language("ru");
        let v = q(&[("r1", Remove), ("i1", Install), ("i2", Install), ("r2", Remove), ("x", InstallAur), ("y", UpgradeAur)]);
        let p = Plan::of(&v);
        assert_eq!(p.remove, vec!["r1".to_string(), "r2".to_string()]);
        assert_eq!(p.steps(&[], &[]), vec![Op::Install(vec!["i1".into(), "i2".into()]), Op::InstallAur(vec!["x".into(), "y".into()])]);
        assert_eq!(p.title(), "Очередь: удаление 2 пакета, установка 3 пакета, обновление 1 пакет");
    }

    #[test]
    fn upgrade_takes_installs_and_ignores_unselected() {
        let v = q(&[("u1", Upgrade), ("new", Install), ("u2", Remove)]);
        let p = Plan::of(&v);
        let updates = ["u1".to_string(), "u2".to_string(), "u3".to_string()];
        let ignore = ["held".to_string(), "new".to_string(), "u3".to_string()];
        assert_eq!(
            p.steps(&updates, &ignore),
            vec![Op::Upgrade { aur: false, ignore: vec!["u2".into(), "u3".into(), "held".into()], install: vec!["new".into()] }]
        );
    }

    #[test]
    fn empty_plan_has_no_steps() {
        assert!(Plan::of(&[]).is_empty());
        assert!(Plan::of(&[]).steps(&["a".into()], &[]).is_empty());
    }
}
