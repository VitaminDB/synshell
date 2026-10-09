//! Точечные правки `config.toml` из оболочки (режим редактирования панелей
//! и дока) с сохранением комментариев и порядка — через `toml_edit`.
//! «Параметры системы» ведут свою копию документа (`synsettings::store`).

use crate::config::{Applet, Config, DeskWidget, Panel};
use crate::paths;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

/// Прочитать документ, применить правку и записать атомарно. `f` возвращает
/// `false` — ничего не менять.
pub fn edit(f: impl FnOnce(&mut DocumentMut) -> bool) -> anyhow::Result<bool> {
    let path = paths::config_file();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: DocumentMut = text.parse()?;
    if !f(&mut doc) {
        return Ok(false);
    }
    // Документ должен остаться корректным конфигом — иначе не пишем.
    let out = doc.to_string();
    Config::parse(&out).map_err(|e| anyhow::anyhow!(e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &out)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// Записать значение по пути таблиц (`["files", "view"]`), создавая
/// недостающие таблицы. Одинаковое значение не переписывает файл.
pub fn set_value(path: &[&str], value: Value) -> anyhow::Result<bool> {
    let Some((key, tables)) = path.split_last() else { return Ok(false) };
    edit(|doc| {
        let mut t: &mut Table = doc.as_table_mut();
        for name in tables {
            if !t.contains_key(name) {
                t.insert(name, Item::Table(Table::new()));
            }
            match t.get_mut(name).and_then(Item::as_table_mut) {
                Some(next) => t = next,
                None => return false,
            }
        }
        if t.get(key).and_then(Item::as_value).map(|v| v.to_string().trim() == value.to_string().trim()).unwrap_or(false) {
            return false;
        }
        t.insert(key, Item::Value(value));
        true
    })
}

/// Структура serde → таблица `toml_edit` (вложенное — inline).
pub fn to_table<T: serde::Serialize>(v: &T) -> Table {
    let text = toml::to_string(v).unwrap_or_default();
    let doc: DocumentMut = text.parse().unwrap_or_default();
    let mut out = Table::new();
    for (k, item) in doc.as_table().iter() {
        let item = match item.clone().into_value() {
            Ok(mut v) => {
                v.decor_mut().clear();
                if let Value::Array(a) = &mut v {
                    pretty_array(a);
                }
                Item::Value(v)
            }
            Err(i) => i,
        };
        out.insert(k, item);
    }
    out
}

/// Длинный массив — по элементу в строке.
fn pretty_array(a: &mut toml_edit::Array) {
    if a.len() > 3 {
        for e in a.iter_mut() {
            e.decor_mut().set_prefix("\n    ");
        }
        a.set_trailing("\n");
        a.set_trailing_comma(true);
    }
}

/// Апплет → inline-таблица `{ type = "…", … }`.
pub fn applet_value(a: &Applet) -> Value {
    let mut t = InlineTable::new();
    t.insert("type", a.kind.as_str().into());
    for (k, v) in &a.options {
        if let Some(v) = toml_value(v) {
            t.insert(k, v);
        }
    }
    Value::InlineTable(t)
}

fn toml_value(v: &toml::Value) -> Option<Value> {
    Some(match v {
        toml::Value::String(s) => s.as_str().into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(f) => (*f).into(),
        toml::Value::Boolean(b) => (*b).into(),
        toml::Value::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for x in a {
                arr.push_formatted(toml_value(x)?);
            }
            Value::Array(arr)
        }
        toml::Value::Table(t) => {
            let mut it = InlineTable::new();
            for (k, x) in t {
                it.insert(k, toml_value(x)?);
            }
            Value::InlineTable(it)
        }
        toml::Value::Datetime(d) => d.to_string().into(),
    })
}

/// Панелей в файле нет (действуют встроенные) — записать их, чтобы дальше
/// править по индексам.
fn ensure_panels(doc: &mut DocumentMut, cfg_panels: &[Panel]) {
    if doc.get("panel").is_some() {
        return;
    }
    let mut aot = toml_edit::ArrayOfTables::new();
    for p in cfg_panels {
        aot.push(to_table(p));
    }
    doc.insert("panel", Item::ArrayOfTables(aot));
}

/// Заменить апплеты панели `index` (номер `[[panel]]` в файле).
pub fn set_panel_applets(index: usize, current: &[Panel], applets: &[Applet]) -> anyhow::Result<bool> {
    edit(|doc| {
        ensure_panels(doc, current);
        let Some(aot) = doc.get_mut("panel").and_then(|i| i.as_array_of_tables_mut()) else { return false };
        let Some(t) = aot.get_mut(index) else { return false };
        let mut arr = toml_edit::Array::new();
        for a in applets {
            arr.push_formatted(applet_value(a));
        }
        pretty_array(&mut arr);
        if applets.len() <= 3 {
            arr.set_trailing("");
        }
        t.insert("applets", Item::Value(Value::Array(arr)));
        true
    })
}

/// Задать ключ панели (`mode = "dock"`, `edge = "left"` …).
pub fn set_panel_key(index: usize, current: &[Panel], key: &str, value: impl Into<Value>) -> anyhow::Result<bool> {
    let value = value.into();
    edit(|doc| {
        ensure_panels(doc, current);
        let Some(aot) = doc.get_mut("panel").and_then(|i| i.as_array_of_tables_mut()) else { return false };
        let Some(t) = aot.get_mut(index) else { return false };
        t.insert(key, Item::Value(value));
        true
    })
}

/// Добавить панель (док) в конец.
pub fn push_panel(current: &[Panel], panel: &Panel) -> anyhow::Result<bool> {
    edit(|doc| {
        ensure_panels(doc, current);
        let Some(aot) = doc.get_mut("panel").and_then(|i| i.as_array_of_tables_mut()) else { return false };
        aot.push(to_table(panel));
        true
    })
}

/// Записать виджеты рабочего стола целиком (`[[widget]]`; пустой список —
/// `widget = []`, иначе вернулась бы встроенная раскладка).
pub fn set_widgets(widgets: &[DeskWidget]) -> anyhow::Result<bool> {
    edit(|doc| {
        doc.remove("widget");
        if widgets.is_empty() {
            doc.insert("widget", Item::Value(Value::Array(toml_edit::Array::new())));
        } else {
            let mut aot = toml_edit::ArrayOfTables::new();
            for w in widgets {
                aot.push(to_table(w));
            }
            doc.insert("widget", Item::ArrayOfTables(aot));
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applet_roundtrip() {
        let mut a = Applet::new("group");
        a.options.insert("name".into(), toml::Value::String("Разработка".into()));
        a.options.insert(
            "items".into(),
            toml::Value::Array(vec![toml::Value::String("code".into()), toml::Value::String("org.kde.konsole".into())]),
        );
        let v = applet_value(&a);
        let text = format!("x = {v}");
        let parsed: toml::Table = toml::from_str(&text).unwrap();
        let back: Applet = parsed["x"].clone().try_into().unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn panel_serializes_with_dock_table() {
        let p = Panel::dock_default();
        let t = to_table(&p);
        let mut doc = DocumentMut::new();
        let mut aot = toml_edit::ArrayOfTables::new();
        aot.push(t);
        doc.insert("panel", Item::ArrayOfTables(aot));
        let cfg = Config::parse(&doc.to_string()).unwrap();
        assert_eq!(cfg.panels[0], p);
    }

    #[test]
    fn widgets_roundtrip() {
        let w = DeskWidget::new("cpu", 1, 2, 4, 3).with("view", "line").with("history", 120i64);
        let mut doc = DocumentMut::new();
        let mut aot = toml_edit::ArrayOfTables::new();
        aot.push(to_table(&w));
        doc.insert("widget", Item::ArrayOfTables(aot));
        let cfg = Config::parse(&doc.to_string()).unwrap();
        assert_eq!(cfg.widgets, Some(vec![w]));
        let empty = Config::parse("widget = []").unwrap();
        assert_eq!(empty.widgets, Some(vec![]));
        assert_eq!(Config::parse("").unwrap().widgets, None);
    }
}
