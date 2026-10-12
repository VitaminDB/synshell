use syngui::prelude::*;
use toml_edit::{Array, Value};

use synassist_voice::{MicTest, MicTestEvent, VadState};

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

thread_local! {
    /// Идущая проверка микрофона (одна на окно).
    static MIC_TEST: std::cell::RefCell<Option<MicTest>> = const { std::cell::RefCell::new(None) };
    static MIC_TEST_WATCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn stop_mic_test() {
    MIC_TEST.with(|t| t.borrow_mut().take());
}

/// Проверка микрофона (как в synspeak): тот же детектор речи, что в голосовом
/// режиме, на живом звуке — уровень, фон, пороги и где фраза закончится.
fn mic_check() -> W {
    stop_mic_test();
    let running = use_signal(false);
    let vad: RwSignal<Option<VadState>> = use_signal(None);
    let phrase: RwSignal<Option<f32>> = use_signal(None);
    let error: RwSignal<Option<String>> = use_signal(None);
    // Ушли со страницы — микрофон больше не держим.
    if !MIC_TEST_WATCH.with(|w| w.replace(true)) {
        let ctx = state::ctx();
        create_detached_effect(move || {
            if ctx.page.get() != "assistant" {
                stop_mic_test();
            }
        });
    }
    let toggle = Reactive::new(move || -> Vec<W> {
        let on = running.get();
        let label = if on { t!("Остановить") } else { t!("Проверить") };
        vec![boxed(button(label, move || {
            if running.get_untracked() {
                stop_mic_test();
                running.set(false);
                vad.set(None);
                return;
            }
            let a = store::config().assistant.clone();
            let mic = Some(a.microphone).filter(|m| !m.is_empty());
            phrase.set(None);
            error.set(None);
            let test = MicTest::start(mic, a.noise_level, move |e| match e {
                MicTestEvent::Level(s) => vad.set(Some(s)),
                MicTestEvent::Phrase(sec) => phrase.set(Some(sec)),
                MicTestEvent::Error(e) => {
                    error.set(Some(e));
                    running.set(false);
                }
            });
            MIC_TEST.with(|t| *t.borrow_mut() = Some(test));
            running.set(true);
        }))]
    });
    let meter = Reactive::new(move || -> Vec<W> {
        if let Some(e) = error.get() {
            return vec![boxed(Text::new(t!("Микрофон не открылся: {e}", e = e)).class("mic-test-state pause"))];
        }
        if !running.get() {
            return Vec::new();
        }
        let Some(s) = vad.get() else {
            return vec![boxed(Text::new(t!("Включаю микрофон…")).class("mic-test-text"))];
        };
        vec![boxed(mic_meter(s, phrase.get()))]
    });
    boxed(
        Column::new()
            .gap(0.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(row_inline(
                t!("Проверка микрофона"),
                t!("Говорите и делайте паузы: полоса — громкость микрофона, красная отметка — порог тишины. Тише отметки — тишина, через секунду тишины фраза заканчивается"),
                toggle,
            ))
            .child(meter),
    )
}

fn mic_meter(s: VadState, phrase: Option<f32>) -> impl Widget {
    let (text, class) = if s.in_phrase && s.speaking {
        (t!("Слышу речь"), "mic-test-state on")
    } else if s.in_phrase {
        (t!("Пауза — здесь фраза закончится"), "mic-test-state pause")
    } else if s.speaking {
        (t!("Громко — похоже на начало речи"), "mic-test-state on")
    } else {
        (t!("Тишина"), "mic-test-state")
    };
    // Шкала — до 20 % или с запасом над порогом речи, если «Уровень шума» высокий.
    let (w, full) = (480.0, (s.speech * 1.25).clamp(0.2, 1.0));
    let mut col = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .class("mic-test")
        .child(
            Column::new()
                .width(w)
                .gap(1.0)
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .child(ProgressBar::with_value((s.level / full).clamp(0.0, 1.0)).class(if s.speaking { "mic-meter on" } else { "mic-meter" }))
                .child(
                    Row::new()
                        .width(w)
                        .child(Column::new().width((s.silence / full).clamp(0.0, 1.0) * w - 1.0))
                        .child(DecoratedBox::new().class("mic-meter-mark")),
                ),
        )
        .child(
            Text::new(t!(
                "уровень {l} % · фон {f} % · тишина ниже {s} % · речь выше {p} %",
                l = format!("{:.1}", s.level * 100.0),
                f = format!("{:.1}", s.floor * 100.0),
                s = format!("{:.1}", s.silence * 100.0),
                p = format!("{:.1}", s.speech * 100.0)
            ))
            .class("mic-test-text"),
        )
        .child(Text::new(text).class(class));
    if let Some(sec) = phrase {
        col = col.child(Text::new(t!("Последняя фраза закончилась: {s} с", s = format!("{sec:.1}"))).class("mic-test-text"));
    }
    col
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
            row(
                t!("Уровень шума, %"),
                t!("Звук тише этого уровня — тишина (0 — только автоматически по фону). Включите проверку микрофона ниже и поставьте чуть выше музыки или шума, но ниже своей речи"),
                Slider::new()
                    .range(0.0, 50.0)
                    .step(0.5)
                    .value(a.noise_level)
                    .show_value(1)
                    .width(240.0)
                    .on_change(|v| {
                        let v = round_to(v as f64, 2);
                        set(&op!["assistant", "noise_level"], v);
                        MIC_TEST.with(|t| t.borrow().as_ref().map(|t| t.set_noise_level(v as f32)));
                    }),
            ),
            mic_check(),
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
