pub mod agent;
pub mod attach;
pub mod autoguide;
pub mod builtin;
pub mod chat;
pub mod compact;
pub mod kb;
pub mod memory;
pub mod pipeline;
pub mod prompt;
pub mod skills;
pub mod tools;

/// Каталоги переводов строк, которые ядро показывает пользователю (подписи
/// действий, ход генерации, подтверждения); их подключают программы:
/// `i18n::init(&[…, syn_assist::CATALOGS].concat())`. Строки для модели
/// (описания инструментов, системные подсказки, результаты) не переводятся.
pub const CATALOGS: &[&str] = &synshell_tr::catalogs!("../i18n");
