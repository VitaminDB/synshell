//! Строки редактора: каталоги (`node.*`, `nodes.*` и др. — ключи из
//! Synthos) и подписи нод, разделов палитры. Программа подключает каталоги
//! при запуске: `syngui::i18n::register_catalogs(&syn_node_editor::i18n::CATALOGS)`
//! (подписи нод synaptix-flow — `synaptix_flow::CATALOGS`).
//!
//! Подписи видов программы: ключ `node.<вид>.title` в её каталогах, иначе
//! `NodeKindMeta::title`; разделы — `node.category.<ключ>`, иначе
//! `CategoryInfo::label`.

use syngui::prelude::*;

use crate::registry::{NodeCategory, NodeKindMeta};
use crate::types::NodeKind;

pub const CATALOGS: [&str; 14] = [
    include_str!("../i18n/en.lang"),
    include_str!("../i18n/ru.lang"),
    include_str!("../i18n/de.lang"),
    include_str!("../i18n/fr.lang"),
    include_str!("../i18n/es.lang"),
    include_str!("../i18n/it.lang"),
    include_str!("../i18n/pt-BR.lang"),
    include_str!("../i18n/pl.lang"),
    include_str!("../i18n/uk.lang"),
    include_str!("../i18n/kk.lang"),
    include_str!("../i18n/tr.lang"),
    include_str!("../i18n/zh-CN.lang"),
    include_str!("../i18n/ja.lang"),
    include_str!("../i18n/ko.lang"),
];

/// serde-имя вида (snake_case у встроенных, имя у видов программы) —
/// стабильный ключ каталога.
pub fn node_kind_key(kind: NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Заголовок ноды: `node.<kind>.title`, иначе литерал из реестра.
pub fn node_title(meta: &NodeKindMeta) -> String {
    syngui::i18n::try_tr(&format!("node.{}.title", node_kind_key(meta.kind)))
        .unwrap_or_else(|| meta.title.to_string())
}

pub fn node_category_label(category: NodeCategory) -> String {
    match category {
        NodeCategory::Custom(k) => syngui::i18n::try_tr(&format!("node.category.{k}"))
            .unwrap_or_else(|| category.label().to_string()),
        _ => tr!(&format!("node.category.{}", category.key())),
    }
}

pub fn node_subcategory_label(key: &str) -> String {
    tr!(&format!("node.subcategory.{key}"))
}
