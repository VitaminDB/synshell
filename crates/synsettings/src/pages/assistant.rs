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

fn set_list(key: &str, list: &[String]) {
    let mut a = Array::new();
    for s in list {
        a.push(s.as_str());
    }
    set(&op!["assistant", key], Value::Array(a));
}

/// Режим инструмента: deny — отключён, ask — спрашивать (опасные действия),
/// allow — выполнять сразу.
fn set_mode(id: &str, mode: &str) {
    let a = store::config().assistant.clone();
    let mut disabled: Vec<String> = a.disabled_tools.into_iter().filter(|d| d != id).collect();
    let mut approved: Vec<String> = a.approved_tools.into_iter().filter(|d| d != id).collect();
    match mode {
        "deny" => disabled.push(id.to_string()),
        "allow" => approved.push(id.to_string()),
        _ => {}
    }
    set_list("disabled_tools", &disabled);
    set_list("approved_tools", &approved);
    state::bump();
}

/// Базы знаний: папки, состояние индекса, обновление в фоне.
fn knowledge(list: Vec<String>) -> W {
    // Ход индексации по папке (текст для подсказки), пусто — не идёт.
    let progress: RwSignal<std::collections::HashMap<String, String>> = use_signal(Default::default());
    let rows = Reactive::new(move || -> Vec<W> {
        let prog = progress.get();
        let mut rows: Vec<W> = Vec::new();
        for (i, k) in list.iter().enumerate() {
            let root = synassist::builtin::expand(k);
            let st = synassist::kb::status(&root);
            let hint = match prog.get(k) {
                Some(p) => p.clone(),
                None if !root.is_dir() => t!("Папки нет: {p}", p = root.display()).to_string(),
                None if st.indexed => t!("{p} · файлов: {f}, фрагментов: {c}", p = root.display(), f = st.files, c = st.chunks).to_string(),
                None => t!("{p} · ещё не проиндексирована — индекс построится при первом поиске или кнопкой", p = root.display()).to_string(),
            };
            let busy = prog.contains_key(k);
            let key = k.clone();
            let refresh = button(if st.indexed { t!("Обновить") } else { t!("Проиндексировать") }, move || {
                if progress.get_untracked().contains_key(&key) {
                    return;
                }
                let key = key.clone();
                let mut m = progress.get_untracked();
                m.insert(key.clone(), t!("Индексация…").to_string());
                progress.set(m);
                std::thread::spawn(move || {
                    let root = synassist::builtin::expand(&key);
                    let k2 = key.clone();
                    let r = synassist::kb::refresh(&root, &mut |done, total| {
                        let k3 = k2.clone();
                        let text = t!("Индексация: {d} из {t} файлов", d = done, t = total).to_string();
                        run_on_main_thread(move || {
                            let mut m = progress.get_untracked();
                            m.insert(k3, text);
                            progress.set(m);
                        });
                    });
                    run_on_main_thread(move || {
                        let mut m = progress.get_untracked();
                        m.remove(&key);
                        progress.set(m);
                        if let Err(e) = r {
                            state::toast(t!("База знаний: {e}", e = e).to_string());
                        }
                    });
                });
            });
            let all = list.clone();
            let remove = button(t!("Убрать"), move || {
                let rest: Vec<String> = all.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| v.clone()).collect();
                set_list("knowledge", &rest);
                state::bump();
            });
            let mut actions = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if !busy {
                actions = actions.child(refresh);
            }
            actions = actions.child(remove);
            rows.push(row_inline(synassist::kb::name_of(&root), hint, actions));
        }
        if rows.is_empty() {
            rows.push(row_inline(t!("Баз пока нет"), t!("Добавьте папку с документами (txt, md, pdf, код) — ассистент будет искать в ней по смыслу и ссылаться на файлы"), Text::new("")));
        }
        rows
    });
    let current = store::config().assistant.knowledge.clone();
    let add = row_inline(t!("Добавить папку"), t!("Документы индексирует модель эмбеддингов по умолчанию (Нейросети → Модели по умолчанию → embed)"), button(t!("Выбрать…"), move || {
        let current = current.clone();
        std::thread::spawn(move || {
            let home = synassist::builtin::home();
            if let Ok(Some(dir)) = synsystem::portal_files::pick(&t!("Папка базы знаний"), true, &home, &[]) {
                let mut list = current.clone();
                let s = dir.display().to_string();
                if !list.contains(&s) {
                    list.push(s);
                }
                run_on_main_thread(move || {
                    set_list("knowledge", &list);
                    state::bump();
                });
            }
        });
    }));
    group(&t!("Базы знаний"), vec![boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(rows)), add])
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

/// Громкость системного микрофона: ползунок шлёт значения в один поток,
/// который применяет только последнее (при перетаскивании их десятки).
fn set_mic_volume(percent: u32) {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Mutex, OnceLock};
    static TX: OnceLock<Mutex<Sender<u32>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<u32>();
        std::thread::spawn(move || {
            while let Ok(mut v) = rx.recv() {
                while let Ok(n) = rx.try_recv() {
                    v = n;
                }
                synsystem::volume::set_mic(v);
            }
        });
        Mutex::new(tx)
    });
    let _ = tx.lock().map(|t| t.send(percent));
}

fn mic_volume() -> W {
    let vol: RwSignal<Option<u32>> = use_signal(None);
    std::thread::spawn(move || {
        let v = synsystem::volume::read().map(|v| v.mic_percent);
        run_on_main_thread(move || vol.set(Some(v.unwrap_or(0))));
    });
    let control = Reactive::new(move || -> Vec<W> {
        let Some(v) = vol.get() else {
            return vec![boxed(Text::new("…").class("row-hint"))];
        };
        vec![boxed(
            Slider::new()
                .range(0.0, 100.0)
                .step(1.0)
                .value(v.min(100) as f32)
                .show_value(0)
                .width(240.0)
                .on_change(|v| set_mic_volume(v.round() as u32)),
        )]
    });
    row(t!("Громкость микрофона, %"), t!("Системный микрофон по умолчанию; то же, что в окне звука на панели"), control)
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
            mic_volume(),
            mic_check(),
        ],
    );
    let tools_note = note(t!("Безопасные действия (чтение, поиск, просмотр) выполняются всегда без вопроса. «Спрашивать» — подтверждать удаление, запись, команды и выключение; «Сразу» — без подтверждения."));
    let mut rows: Vec<W> = Vec::new();
    rows.extend(TOOLS.iter().map(|(id, title, hint)| {
        let mode = if a.disabled_tools.iter().any(|d| d == id) {
            "deny"
        } else if a.approved_tools.iter().any(|d| d == id) {
            "allow"
        } else {
            "ask"
        };
        let id = id.to_string();
        let dd = Dropdown::new()
            .width(170.0)
            .item(DropdownItem::new("ask", t!("Спрашивать")))
            .item(DropdownItem::new("allow", t!("Сразу")))
            .item(DropdownItem::new("deny", t!("Отключён")))
            .selected(mode)
            .on_change(move |v: &str| set_mode(&id, v));
        row_inline(tl(title), tl(hint), dd)
    }));
    let skills_dir = synassist::skills::resolve(&a.skills_dir);
    let found = synassist::skills::list(&skills_dir);
    let mut skill_rows: Vec<W> = vec![
        switch_row(
            t!("Использовать навыки"),
            t!("Навык — папка с файлом SKILL.md: в начале между строками --- поля name и description (когда применять), дальше — инструкции. Ассистент видит описания и подгружает навык, когда задача подходит"),
            op!["assistant", "skills"],
            a.skills,
        ),
        text_row(
            t!("Каталог навыков"),
            t!("Пусто — {d}", d = synassist::skills::resolve("").display()),
            op!["assistant", "skills_dir"],
            &a.skills_dir,
            t!("по умолчанию"),
        ),
        row_inline(t!("Папка навыков"), skills_dir.display().to_string(), button(t!("Открыть"), move || {
            let _ = std::fs::create_dir_all(&skills_dir);
            let _ = std::process::Command::new("xdg-open").arg(&skills_dir).spawn();
        })),
    ];
    if found.is_empty() {
        skill_rows.push(row_inline(t!("Навыков пока нет"), t!("Положите в каталог папку с SKILL.md — навык появится здесь и у ассистента"), Text::new("")));
    }
    for k in found {
        skill_rows.push(row_inline(k.name.clone(), k.description.clone(), Text::new(k.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).class("muted")));
    }
    let memory_dir = synassist::memory::resolve(&a.memory_dir);
    let memory = group(
        &t!("Память"),
        vec![
            switch_row(
                t!("Помнить между разговорами"),
                t!("Ассистент сохраняет важное о вас и ваших делах — по файлу на факт, индекс MEMORY.md читает в начале каждого разговора. Попросите «запомни…» или «забудь…»"),
                op!["assistant", "memory"],
                a.memory,
            ),
            text_row(
                t!("Каталог памяти"),
                t!("Файлы .md можно править руками; пусто — {d}", d = synassist::memory::resolve("").display()),
                op!["assistant", "memory_dir"],
                &a.memory_dir,
                t!("по умолчанию"),
            ),
            row_inline(t!("Файлы памяти"), memory_dir.display().to_string(), button(t!("Открыть"), move || {
                let _ = std::fs::create_dir_all(&memory_dir);
                let _ = std::process::Command::new("xdg-open").arg(&memory_dir).spawn();
            })),
        ],
    );
    page(
        t!("Ассистент"),
        t!("Помощник на нейросетях этого устройства: отвечает, управляет системой, работает с файлами, ищет в интернете."),
        vec![general, memory, knowledge(a.knowledge.clone()), group(&t!("Инструменты"), rows), tools_note, group(&t!("Навыки"), skill_rows), voice, voice_note],
    )
}
