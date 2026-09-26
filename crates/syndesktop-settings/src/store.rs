//! Хранилище настроек: `config.toml` как `toml_edit::DocumentMut`.
//!
//! Правки точечные — меняется только значение по пути, комментарии и
//! форматирование пользователя остаются. После каждой правки документ
//! разбирается в [`Config`] (для чтения значений и показа ошибок) и через
//! небольшую задержку сохраняется на диск: композитор и оболочка подхватят
//! файл сами, опросом mtime.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use syndesktop_common::config::{Config, DEFAULT_CONFIG_TOML};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

/// Сегмент пути в документе: ключ таблицы или индекс массива.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seg<'a> {
    K(&'a str),
    I(usize),
}

impl<'a> From<&'a str> for Seg<'a> {
    fn from(s: &'a str) -> Self {
        Seg::K(s)
    }
}

impl From<usize> for Seg<'_> {
    fn from(i: usize) -> Self {
        Seg::I(i)
    }
}

/// `path!["panel", 0, "size"]` → `[Seg::K("panel"), Seg::I(0), Seg::K("size")]`.
#[macro_export]
macro_rules! path {
    ($($s:expr),* $(,)?) => { [$($crate::store::Seg::from($s)),*] };
}

// ─── Чистые операции над документом (тестируются без диска) ────────────────

/// Установить значение по пути. Создаёт недостающие таблицы. Сохраняет
/// оформление (комментарий после значения) у заменяемого значения.
pub fn doc_set(doc: &mut DocumentMut, segs: &[Seg], value: Value) -> bool {
    let (last, parents) = match segs.split_last() {
        Some(x) => x,
        None => return false,
    };
    let root = doc.as_item_mut();
    with_container(root, parents, &mut |c| put(c, *last, value.clone()))
}

/// Удалить значение/таблицу/элемент массива по пути.
pub fn doc_remove(doc: &mut DocumentMut, segs: &[Seg]) -> bool {
    let (last, parents) = match segs.split_last() {
        Some(x) => x,
        None => return false,
    };
    let root = doc.as_item_mut();
    with_container(root, parents, &mut |c| match (c, *last) {
        (Container::Table(t), Seg::K(k)) => t.remove(k).is_some(),
        (Container::Inline(t), Seg::K(k)) => t.remove(k).is_some(),
        (Container::Aot(a), Seg::I(i)) if i < a.len() => {
            a.remove(i);
            true
        }
        (Container::Array(a), Seg::I(i)) if i < a.len() => {
            a.remove(i);
            true
        }
        _ => false,
    })
}

/// Прочитать значение по пути (копия).
pub fn doc_get(doc: &DocumentMut, segs: &[Seg]) -> Option<Item> {
    let mut cur: &Item = doc.as_item();
    let mut val: Option<&Value> = None;
    for (n, seg) in segs.iter().enumerate() {
        if let Some(v) = val {
            val = Some(match (v, seg) {
                (Value::InlineTable(t), Seg::K(k)) => t.get(k)?,
                (Value::Array(a), Seg::I(i)) => a.get(*i)?,
                _ => return None,
            });
            continue;
        }
        match (cur, seg) {
            (Item::Table(t), Seg::K(k)) => cur = t.get(k)?,
            (Item::ArrayOfTables(a), Seg::I(i)) => {
                let t = a.get(*i)?;
                // Таблица из массива — продолжаем как с обычной таблицей.
                return get_in_table(t, &segs[n + 1..]);
            }
            (Item::Value(v), s) => {
                val = Some(match (v, s) {
                    (Value::InlineTable(t), Seg::K(k)) => t.get(k)?,
                    (Value::Array(a), Seg::I(i)) => a.get(*i)?,
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    Some(match val {
        Some(v) => Item::Value(v.clone()),
        None => cur.clone(),
    })
}

fn get_in_table(t: &Table, rest: &[Seg]) -> Option<Item> {
    let mut d = DocumentMut::new();
    // Дешёвый способ переиспользовать doc_get: копия таблицы как корень.
    *d.as_table_mut() = t.clone();
    doc_get(&d, rest)
}

/// Добавить таблицу в массив таблиц (`[[panel]]`, `[[rule]]`, `[[output]]`).
/// Возвращает индекс новой записи.
pub fn doc_push_table(doc: &mut DocumentMut, key: &str, table: Table) -> usize {
    let root = doc.as_table_mut();
    if !matches!(root.get(key), Some(Item::ArrayOfTables(_))) {
        root.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let aot = root.get_mut(key).and_then(Item::as_array_of_tables_mut).expect("aot");
    aot.push(table);
    aot.len() - 1
}

/// Поменять местами элементы массива (или массива таблиц) по пути `segs`.
pub fn doc_swap(doc: &mut DocumentMut, segs: &[Seg], a: usize, b: usize) -> bool {
    let root = doc.as_item_mut();
    with_container(root, segs, &mut |c| match c {
        Container::Array(arr) => {
            if a >= arr.len() || b >= arr.len() || a == b {
                return false;
            }
            let (lo, hi) = (a.min(b), a.max(b));
            let hv = arr.remove(hi);
            let lv = arr.remove(lo);
            // Оформление (переносы строк/отступы) остаётся на позиции.
            let (ld, hd) = (lv.decor().clone(), hv.decor().clone());
            let mut hv = hv;
            let mut lv = lv;
            *hv.decor_mut() = ld;
            *lv.decor_mut() = hd;
            arr.insert_formatted(lo, hv);
            arr.insert_formatted(hi, lv);
            true
        }
        Container::Aot(aot) => {
            if a >= aot.len() || b >= aot.len() || a == b {
                return false;
            }
            // Порядок вывода задают позиции таблиц в документе — меняем
            // местами и их, иначе файл не изменится.
            let mut tables: Vec<Table> = aot.iter().cloned().collect();
            let positions: Vec<_> = tables.iter().map(|t| t.position()).collect();
            tables.swap(a, b);
            aot.clear();
            for (mut t, pos) in tables.into_iter().zip(positions) {
                if let Some(p) = pos {
                    t.set_position(p);
                }
                aot.push(t);
            }
            true
        }
        _ => false,
    })
}

/// Добавить значение в конец массива по пути (создаёт массив при нужде).
/// Многострочный массив остаётся многострочным: новому элементу достаётся
/// оформление последнего.
pub fn doc_array_push(doc: &mut DocumentMut, segs: &[Seg], value: Value) -> bool {
    if doc_get(doc, segs).is_none() {
        doc_set(doc, segs, Value::Array(Array::new()));
    }
    let root = doc.as_item_mut();
    with_container(root, segs, &mut |c| match c {
        Container::Array(arr) => {
            let mut v = value.clone();
            if let Some(last) = arr.iter().last() {
                *v.decor_mut() = last.decor().clone();
                arr.push_formatted(v);
            } else {
                arr.push(v);
            }
            true
        }
        _ => false,
    })
}

enum Container<'a> {
    Table(&'a mut Table),
    Inline(&'a mut InlineTable),
    Aot(&'a mut ArrayOfTables),
    Array(&'a mut Array),
}

fn put(c: Container, last: Seg, mut value: Value) -> bool {
    match (c, last) {
        (Container::Table(t), Seg::K(k)) => {
            match t.get_mut(k) {
                Some(Item::Value(old)) => {
                    *value.decor_mut() = old.decor().clone();
                    *old = value;
                }
                Some(item) => *item = Item::Value(value),
                None => {
                    t.insert(k, Item::Value(value));
                }
            }
            true
        }
        (Container::Inline(t), Seg::K(k)) => {
            match t.get_mut(k) {
                Some(old) => {
                    *value.decor_mut() = old.decor().clone();
                    *old = value;
                }
                None => {
                    t.insert(k, value);
                }
            }
            true
        }
        (Container::Array(a), Seg::I(i)) if i < a.len() => {
            let old = a.get_mut(i).unwrap();
            *value.decor_mut() = old.decor().clone();
            *old = value;
            true
        }
        _ => false,
    }
}

/// Найти (создав таблицы по пути) контейнер `segs` и вызвать `f`.
fn with_container(root: &mut Item, segs: &[Seg], f: &mut dyn FnMut(Container) -> bool) -> bool {
    let mut cur = root;
    let mut i = 0;
    while i < segs.len() {
        let seg = segs[i];
        let is_last = i + 1 == segs.len();
        match (cur, seg) {
            (Item::Table(t), Seg::K(k)) => {
                if !t.contains_key(k) {
                    let next = segs.get(i + 1);
                    // Следом индекс — массив сам не создаём (неизвестно,
                    // какой); следом ключ — таблица.
                    match next {
                        Some(Seg::I(_)) => return false,
                        _ => {
                            let mut nt = Table::new();
                            nt.set_implicit(!is_last);
                            t.insert(k, Item::Table(nt));
                        }
                    }
                }
                cur = t.get_mut(k).unwrap();
            }
            (Item::ArrayOfTables(a), Seg::I(n)) => {
                let Some(t) = a.get_mut(n) else { return false };
                return with_container_table(t, &segs[i + 1..], f);
            }
            (Item::Value(v), _) => return with_container_value(v, &segs[i..], f),
            _ => return false,
        }
        i += 1;
    }
    match cur {
        Item::Table(t) => f(Container::Table(t)),
        Item::ArrayOfTables(a) => f(Container::Aot(a)),
        Item::Value(Value::InlineTable(t)) => f(Container::Inline(t)),
        Item::Value(Value::Array(a)) => f(Container::Array(a)),
        _ => false,
    }
}

fn with_container_table(t: &mut Table, segs: &[Seg], f: &mut dyn FnMut(Container) -> bool) -> bool {
    if segs.is_empty() {
        return f(Container::Table(t));
    }
    match segs[0] {
        Seg::K(k) => {
            if !t.contains_key(k) {
                if matches!(segs.get(1), Some(Seg::I(_))) {
                    return false;
                }
                // Внутри [[panel]] вложенные таблицы пишем inline — так
                // запись остаётся одним блоком.
                t.insert(k, Item::Value(Value::InlineTable(InlineTable::new())));
            }
            let item = t.get_mut(k).unwrap();
            match item {
                Item::Table(inner) => with_container_table(inner, &segs[1..], f),
                Item::ArrayOfTables(a) => match segs.get(1) {
                    None => f(Container::Aot(a)),
                    Some(Seg::I(n)) => match a.get_mut(*n) {
                        Some(inner) => with_container_table(inner, &segs[2..], f),
                        None => false,
                    },
                    _ => false,
                },
                Item::Value(v) => with_container_value(v, &segs[1..], f),
                _ => false,
            }
        }
        Seg::I(_) => false,
    }
}

fn with_container_value(v: &mut Value, segs: &[Seg], f: &mut dyn FnMut(Container) -> bool) -> bool {
    if segs.is_empty() {
        return match v {
            Value::InlineTable(t) => f(Container::Inline(t)),
            Value::Array(a) => f(Container::Array(a)),
            _ => false,
        };
    }
    match (v, segs[0]) {
        (Value::InlineTable(t), Seg::K(k)) => {
            if !t.contains_key(k) {
                if matches!(segs.get(1), Some(Seg::I(_))) {
                    return false;
                }
                t.insert(k, Value::InlineTable(InlineTable::new()));
            }
            with_container_value(t.get_mut(k).unwrap(), &segs[1..], f)
        }
        (Value::Array(a), Seg::I(n)) => match a.get_mut(n) {
            Some(inner) => with_container_value(inner, &segs[1..], f),
            None => false,
        },
        _ => false,
    }
}

/// Сериализовать структуру serde в `toml_edit::Table` (для новых записей
/// `[[panel]]`, `[[rule]]`, `[[output]]` со значениями по умолчанию).
pub fn to_table<T: serde::Serialize>(v: &T) -> Table {
    let text = toml::to_string(v).unwrap_or_default();
    let doc: DocumentMut = text.parse().unwrap_or_default();
    // Вложенные таблицы и массивы таблиц — inline: запись внутри `[[panel]]`
    // не может нести свои заголовки `[[applets]]`.
    let mut out = Table::new();
    for (k, item) in doc.as_table().iter() {
        let item = match item.clone().into_value() {
            Ok(mut v) => {
                v.decor_mut().clear();
                if let Value::Array(a) = &mut v {
                    // Длинный список апплетов — по элементу в строке.
                    if a.len() > 3 {
                        for e in a.iter_mut() {
                            e.decor_mut().set_prefix("\n    ");
                        }
                        a.set_trailing("\n");
                        a.set_trailing_comma(true);
                    }
                }
                Item::Value(v)
            }
            Err(i) => i,
        };
        out.insert(k, item);
    }
    out
}

// ─── Глобальное хранилище ───────────────────────────────────────────────────

pub struct Store {
    pub path: PathBuf,
    pub doc: DocumentMut,
    pub config: Config,
    /// Ошибка разбора текущего документа (показывается баннером).
    pub error: Option<String>,
    /// mtime последней нашей записи — внешние правки отличаем от своих.
    pub written_mtime: Option<SystemTime>,
    /// Сохранение запрещено: документ на диске не разобрался как TOML,
    /// перезаписывать его нельзя, чтобы не потерять текст пользователя.
    pub read_only: bool,
}

static STORE: Mutex<Option<Store>> = Mutex::new(None);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);

fn lock() -> std::sync::MutexGuard<'static, Option<Store>> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Загрузить (или создать из встроенного образца) конфиг по пути.
pub fn init(path: &Path) {
    let store = load(path);
    *lock() = Some(store);
}

fn load(path: &Path) -> Store {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_CONFIG_TOML.to_string(),
        Err(e) => {
            return Store {
                path: path.to_path_buf(),
                doc: DocumentMut::new(),
                config: Config::default(),
                error: Some(format!("{}: {e}", path.display())),
                written_mtime: None,
                read_only: true,
            }
        }
    };
    let (doc, read_only, mut error) = match text.parse::<DocumentMut>() {
        Ok(d) => (d, false, None),
        Err(e) => (DocumentMut::new(), true, Some(format!("Файл не разобран как TOML: {e}"))),
    };
    let config = match Config::parse(&doc.to_string()) {
        Ok(c) => c,
        Err(e) => {
            error.get_or_insert(e);
            Config::default()
        }
    };
    Store {
        path: path.to_path_buf(),
        written_mtime: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        doc,
        config,
        error,
        read_only,
    }
}

/// Перечитать файл с диска (внешняя правка или «Сбросить»).
pub fn reload() {
    let mut g = lock();
    if let Some(s) = g.as_ref() {
        let path = s.path.clone();
        *g = Some(load(&path));
    }
}

static PENDING: AtomicU64 = AtomicU64::new(0);

fn save_pending() -> bool {
    PENDING.load(Ordering::SeqCst) > 0
}

pub fn with<R>(f: impl FnOnce(&Store) -> R) -> R {
    let g = lock();
    f(g.as_ref().expect("store::init не вызван"))
}

/// Текущий разобранный конфиг.
pub fn config() -> Config {
    with(|s| s.config.clone())
}

pub fn error() -> Option<String> {
    with(|s| s.error.clone())
}

pub fn path() -> PathBuf {
    with(|s| s.path.clone())
}


/// Изменить документ и запланировать сохранение.
pub fn edit(f: impl FnOnce(&mut DocumentMut) -> bool) -> bool {
    let changed = {
        let mut g = lock();
        let Some(s) = g.as_mut() else { return false };
        if s.read_only {
            return false;
        }
        let changed = f(&mut s.doc);
        if changed {
            match Config::parse(&s.doc.to_string()) {
                Ok(c) => {
                    s.config = c;
                    s.error = None;
                }
                Err(e) => s.error = Some(e),
            }
        }
        changed
    };
    if changed {
        schedule_save();
        crate::state::notify_changed();
    }
    changed
}

pub fn set(segs: &[Seg], value: impl Into<Value>) -> bool {
    let v = value.into();
    edit(|d| doc_set(d, segs, v))
}

pub fn remove(segs: &[Seg]) -> bool {
    edit(|d| doc_remove(d, segs))
}

/// Массив таблиц `key` отсутствует в файле, а в разобранном конфиге есть
/// значения по умолчанию (панель по умолчанию) — записать их в файл, чтобы
/// дальше править по индексам.
pub fn ensure_aot<T: serde::Serialize>(key: &str, defaults: &[T]) {
    let missing = with(|s| s.doc.get(key).is_none());
    if missing && !defaults.is_empty() {
        edit(|d| {
            for t in defaults {
                doc_push_table(d, key, to_table(t));
            }
            true
        });
    }
}

/// Сохранение через 300 мс после последней правки (ползунки шлют десятки
/// изменений в секунду — пишем один раз в конце).
fn schedule_save() {
    let gen = SAVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    PENDING.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        if SAVE_GEN.load(Ordering::SeqCst) == gen {
            if let Err(e) = save_now() {
                tracing::error!("не удалось сохранить конфиг: {e}");
            }
        }
        PENDING.fetch_sub(1, Ordering::SeqCst);
    });
}

/// Записать документ на диск атомарно (временный файл + rename).
pub fn save_now() -> std::io::Result<()> {
    let mut g = lock();
    let Some(s) = g.as_mut() else { return Ok(()) };
    if s.read_only {
        return Ok(());
    }
    if let Some(dir) = s.path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = s.path.with_extension("toml.tmp");
    std::fs::write(&tmp, s.doc.to_string())?;
    std::fs::rename(&tmp, &s.path)?;
    s.written_mtime = std::fs::metadata(&s.path).and_then(|m| m.modified()).ok();
    Ok(())
}

/// Проверка «файл поменяли снаружи» для таймера опроса.
pub fn external_change() -> bool {
    if save_pending() {
        return false;
    }
    let g = lock();
    let Some(s) = g.as_ref() else { return false };
    let now = std::fs::metadata(&s.path).and_then(|m| m.modified()).ok();
    now.is_some() && now != s.written_mtime
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"# Мой конфиг
[general]
terminal = "konsole"   # любимый терминал

[appearance]
# акцент
accent = "#3d8bfd"

[[panel]]
edge = "bottom"
applets = [
    { type = "launcher" },
    { type = "clock", format = "%H:%M" },
]
"##;

    fn doc() -> DocumentMut {
        SAMPLE.parse().unwrap()
    }

    #[test]
    fn set_keeps_comments_and_decor() {
        let mut d = doc();
        assert!(doc_set(&mut d, &path!["general", "terminal"], "foot".into()));
        let s = d.to_string();
        assert!(s.contains("# Мой конфиг"));
        assert!(s.contains(r#"terminal = "foot"   # любимый терминал"#), "{s}");
        assert!(s.contains("# акцент"));
        assert!(Config::parse(&s).is_ok());
    }

    #[test]
    fn set_creates_sections() {
        let mut d = doc();
        assert!(doc_set(&mut d, &path!["input", "keyboard", "layouts"], "us,de".into()));
        assert!(doc_set(&mut d, &path!["workspaces", "layouts", "3"], "tile".into()));
        let s = d.to_string();
        let c = Config::parse(&s).unwrap();
        assert_eq!(c.input.keyboard.layouts, "us,de");
        assert_eq!(c.workspaces.layouts.get("3").map(|l| l.as_str()), Some("tile"));
        assert!(s.contains("[input.keyboard]"), "{s}");
    }

    #[test]
    fn panel_array_of_tables() {
        let mut d = doc();
        assert!(doc_set(&mut d, &path!["panel", 0, "size"], 52i64.into()));
        let idx = doc_push_table(&mut d, "panel", to_table(&syndesktop_common::config::Panel::default()));
        assert_eq!(idx, 1);
        assert!(doc_set(&mut d, &path!["panel", 1, "edge"], "top".into()));
        let c = Config::parse(&d.to_string()).unwrap();
        assert_eq!(c.panels.len(), 2);
        assert_eq!(c.panels[0].size, 52);
        assert_eq!(c.panels[1].edge, syndesktop_common::config::Edge::Top);
        assert!(doc_remove(&mut d, &path!["panel", 1]));
        let c = Config::parse(&d.to_string()).unwrap();
        assert_eq!(c.panels.len(), 1);
    }

    #[test]
    fn applets_inline_array() {
        let mut d = doc();
        assert!(doc_set(&mut d, &path!["panel", 0, "applets", 1, "format"], "%H:%M:%S".into()));
        let mut t = InlineTable::new();
        t.insert("type", "spacer".into());
        assert!(doc_array_push(&mut d, &path!["panel", 0, "applets"], Value::InlineTable(t)));
        assert!(doc_swap(&mut d, &path!["panel", 0, "applets"], 0, 2));
        let s = d.to_string();
        let c = Config::parse(&s).unwrap();
        let kinds: Vec<_> = c.panels[0].applets.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(kinds, ["spacer", "clock", "launcher"]);
        assert_eq!(c.panels[0].applets[1].str("format"), Some("%H:%M:%S"));
        // Многострочность сохранилась.
        assert!(s.contains("applets = [\n"), "{s}");
        assert!(doc_remove(&mut d, &path!["panel", 0, "applets", 0]));
        let c = Config::parse(&d.to_string()).unwrap();
        assert_eq!(c.panels[0].applets.len(), 2);
    }

    #[test]
    fn rules_and_outputs() {
        let mut d = doc();
        let i = doc_push_table(&mut d, "rule", Table::new());
        assert!(doc_set(&mut d, &path!["rule", i, "app_id"], "^firefox$".into()));
        assert!(doc_set(&mut d, &path!["rule", i, "workspace"], 2i64.into()));
        let mut size = Array::new();
        size.push(800i64);
        size.push(600i64);
        assert!(doc_set(&mut d, &path!["rule", i, "size"], Value::Array(size)));
        let o = doc_push_table(&mut d, "output", Table::new());
        assert!(doc_set(&mut d, &path!["output", o, "name"], "eDP-1".into()));
        assert!(doc_set(&mut d, &path!["output", o, "scale"], 1.25.into()));
        let s = d.to_string();
        assert!(s.contains("[[rule]]") && s.contains("[[output]]"), "{s}");
        let c = Config::parse(&s).unwrap();
        assert_eq!(c.rules[0].app_id.as_deref(), Some("^firefox$"));
        assert_eq!(c.rules[0].size, Some([800, 600]));
        assert_eq!(c.outputs[0].scale, 1.25);
        // Удаление ключа из правила.
        assert!(doc_remove(&mut d, &path!["rule", 0, "workspace"]));
        assert_eq!(Config::parse(&d.to_string()).unwrap().rules[0].workspace, None);
        // Порядок правил можно менять.
        let j = doc_push_table(&mut d, "rule", Table::new());
        doc_set(&mut d, &path!["rule", j, "title"], "x".into());
        assert!(doc_swap(&mut d, &path!["rule"], 0, 1));
        assert_eq!(Config::parse(&d.to_string()).unwrap().rules[0].title.as_deref(), Some("x"));
    }

    #[test]
    fn keybindings_with_plus_keys() {
        let mut d = doc();
        assert!(doc_set(&mut d, &path!["keybindings", "Super+Return"], "spawn foot".into()));
        assert!(d.to_string().contains(r#""Super+Return" = "spawn foot""#), "{}", d);
        assert!(doc_remove(&mut d, &path!["keybindings", "Super+Return"]));
    }

    #[test]
    fn get_values() {
        let d = doc();
        assert_eq!(
            doc_get(&d, &path!["general", "terminal"]).and_then(|i| i.as_str().map(String::from)),
            Some("konsole".into())
        );
        assert_eq!(
            doc_get(&d, &path!["panel", 0, "applets", 1, "format"])
                .and_then(|i| i.as_str().map(String::from)),
            Some("%H:%M".into())
        );
        assert!(doc_get(&d, &path!["nope", "x"]).is_none());
    }

    #[test]
    fn default_file_roundtrips_untouched() {
        let d: DocumentMut = DEFAULT_CONFIG_TOML.parse().unwrap();
        assert_eq!(d.to_string(), DEFAULT_CONFIG_TOML);
    }
}
