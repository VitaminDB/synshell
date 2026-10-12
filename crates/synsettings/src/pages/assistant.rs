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
    ("subagent", n_!("Субагенты"), n_!("Длинные подзадачи выполняет вложенный агент и возвращает только итог; запуск подтверждается, шаги внутри — нет")),
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
            switch_row(
                t!("Сжимать длинные разговоры"),
                t!("Когда контекст модели почти заполнен, старая часть разговора заменяется краткой сводкой"),
                op!["assistant", "autocompact"],
                a.autocompact,
            ),
            row_inline(t!("Окно ассистента"), t!("Чаты, вложения, история"), button(t!("Открыть"), || {
                let _ = std::process::Command::new("synassistant").spawn();
            })),
        ],
    );
    let mics: RwSignal<Option<Vec<String>>> = use_signal(None);
    std::thread::spawn(move || {
        let list = synassist_voice::list_microphones();
        run_on_main_thread(move || mics.set(Some(list)));
    });
    let current_mic = a.microphone.clone();
    let mic_pick = Reactive::new(move || -> Vec<W> {
        let mut v = vec![(String::new(), t!("Системный по умолчанию").to_string())];
        if let Some(list) = mics.get() {
            v.extend(list.into_iter().map(|d| (d.clone(), d)));
        }
        if !current_mic.is_empty() && !v.iter().any(|(id, _)| *id == current_mic) {
            v.push((current_mic.clone(), current_mic.clone()));
        }
        vec![boxed(choice_owned(op!["assistant", "microphone"], &current_mic, v, 280.0))]
    });
    let voice_note = note(t!("Голосовой разговор и диктовка используют модели распознавания речи и озвучки по умолчанию из «Нейросети → Модели по умолчанию». В окне ассистента — кнопка «Голосом» (Ctrl+Shift+M), диктовка — микрофон в строке ввода (Ctrl+M); в быстрой панели Super+A — те же кнопки."));
    let voice = group(
        &t!("Голос"),
        vec![
            switch_row(
                t!("Озвучивать ответы"),
                t!("Читать вслух ответы и в текстовом чате; в голосовом разговоре ответы озвучиваются всегда"),
                op!["assistant", "speak_answers"],
                a.speak_answers,
            ),
            choice_row(
                t!("Язык речи"),
                t!("Язык распознавания и озвучки"),
                op!["assistant", "voice_language"],
                &a.voice_language,
                &[("", t!("Автоматически")), ("ru", t!("Русский")), ("en", t!("Английский")), ("kk", t!("Казахский"))],
            ),
            text_row(
                t!("Голос"),
                t!("Описание голоса по-английски, например calm female voice (русский текст и цифры модель может прочитать вслух); пусто — голос модели по умолчанию. Голос держится одним на всю озвучку"),
                op!["assistant", "voice"],
                &a.voice,
                t!("по умолчанию"),
            ),
            slider_row(t!("Скорость речи"), t!("1,0 — обычная"), op!["assistant", "voice_speed"], a.voice_speed as f64, 0.5, 2.0, 0.1, 1),
            row_inline(t!("Микрофон"), t!("Устройство записи для голосового режима"), mic_pick),
            slider_row(
                t!("Уровень шума"),
                t!("Насколько порог тишины выше фонового шума. Поднимите, если в шумном месте ассистент не слышит, что вы замолчали; слишком высокий — не услышит тихую речь"),
                op!["assistant", "noise_level"],
                a.noise_level as f64,
                0.0,
                100.0,
                5.0,
                0,
            ),
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
        vec![general, voice, voice_note, group(&t!("Инструменты"), rows)],
    )
}
