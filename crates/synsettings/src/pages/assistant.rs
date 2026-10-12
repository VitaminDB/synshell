use syngui::prelude::*;
use toml_edit::{Array, Value};

use crate::op;
use crate::state;
use crate::store;
use crate::ui::*;

const TOOLS: &[(&str, &str, &str)] = &[
    ("system", n_!("Управление системой"), n_!("Программы, Параметры, громкость, яркость, Wi‑Fi и Bluetooth, уведомления, напоминания, выключение")),
    ("windows", n_!("Окна"), n_!("Список окон, переключение, сворачивание, закрытие")),
    ("files", n_!("Файлы"), n_!("Просмотр, поиск, чтение и запись файлов")),
    ("bash", n_!("Терминал"), n_!("Команды bash от вашего имени")),
    ("web", n_!("Интернет"), n_!("Поиск и чтение страниц")),
    ("media", n_!("Медиа"), n_!("Озвучка текста; картинки, видео и музыка — когда служба их поддержит")),
];

fn set_disabled(list: Vec<String>) {
    let mut a = Array::new();
    for s in &list {
        a.push(s.as_str());
    }
    set(&op!["assistant", "disabled_tools"], Value::Array(a));
    state::bump();
}

fn default_model() -> String {
    synaptixd::Client::connect()
        .and_then(|mut c| c.resolve(synaptixd::ModelKind::Llm))
        .map(|m| m.title)
        .unwrap_or_else(|_| t!("служба нейросетей не отвечает").to_string())
}

pub fn assistant() -> W {
    let a = store::config().assistant.clone();
    let ctx = state::ctx();
    let general = group(
        &t!("Ассистент"),
        vec![
            switch_row(
                t!("Включить ассистента"),
                t!("Super+A — быстрый вопрос поверх окон; полное окно с чатами — «Ассистент» в меню программ"),
                op!["assistant", "enabled"],
                a.enabled,
            ),
            row_inline(
                t!("Модель"),
                t!("Языковая модель по умолчанию: {m}", m = default_model()),
                button(t!("Выбрать модель"), move || {
                    state::open_page(ctx, "models");
                    ctx.sub.set(Some(("models/defaults".into(), t!("Модели по умолчанию"))));
                }),
            ),
            switch_row(
                t!("Выполнять без подтверждения"),
                t!("Удаление и запись файлов, команды терминала и выключение — без вопроса. Включайте, только если доверяете модели"),
                op!["assistant", "auto_approve"],
                a.auto_approve,
            ),
            row_inline(t!("Окно ассистента"), t!("Чаты, вложения, история"), button(t!("Открыть"), || {
                let _ = std::process::Command::new("synassistant").spawn();
            })),
        ],
    );
    let rows: Vec<W> = TOOLS
        .iter()
        .map(|(id, title, hint)| {
            let on = !a.disabled_tools.iter().any(|d| d == id);
            let list = a.disabled_tools.clone();
            let id = id.to_string();
            row_inline(
                tl(title),
                tl(hint),
                Toggle::with_state(on).on_change(move |v| {
                    let mut l: Vec<String> = list.iter().filter(|d| **d != id).cloned().collect();
                    if !v {
                        l.push(id.clone());
                    }
                    set_disabled(l);
                }),
            )
        })
        .collect();
    page(
        t!("Ассистент"),
        t!("Помощник на нейросетях этого устройства: отвечает, управляет системой, работает с файлами, ищет в интернете."),
        vec![general, group(&t!("Инструменты"), rows)],
    )
}
