//! Вкладки настроек — группировка как у форка (модель и стоимость, движение и ввод, композиция и цвет,
//! инструменты проверки) плюс «Помощник»: среда запуска, профили, DLL NGX, журнал, сброс.

use syngui::prelude::*;
use syngui::widgets::SegmentedButton;

use super::controls::{self as c, Latch::*};
use super::{boxed, icon_button, W};
use syn_dlss::{fld, pfld};
use crate::icons;
use crate::shm::{self, b2f, f2b};
use crate::state::{header, open_path, Dialog, St};

fn opts(list: &[String]) -> Vec<String> {
    list.to_vec()
}

fn styles() -> Vec<String> {
    vec![t!("Обычный"), t!("Естественный"), t!("Кинематографичный")]
}

/// «Игры syn-wine»: флаг, по которому наш Wine сам включает слой своим программам.
fn syn_wine_row() -> W {
    c::row(
        t!("Игры syn-wine"),
        t!("Слой DLSS 5 в программах нашего Wine без переменной VKLayer_DLSS5=1 — с их следующего запуска. Нужны кадры Vulkan: для Direct3D 8–11 — DXVK"),
        Toggle::with_state(crate::paths::syn_wine_flag().exists()).on_change(|on| {
            let flag = crate::paths::syn_wine_flag();
            let r = if on {
                std::fs::create_dir_all(crate::paths::config_dir()).and_then(|_| std::fs::write(&flag, b""))
            } else {
                std::fs::remove_file(&flag).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
            };
            if let Err(e) = r {
                tracing::warn!(?e, "флаг syn-wine");
            }
        }),
    )
}

/// Качество: разрешение модели и качество оптического потока.
const QUALITY: [(f32, u32); 3] = [(0.5, 0), (0.75, 1), (1.0, 2)];

/// Страница «Главное»: то, что нужно, чтобы включить и настроить на глаз.
pub fn simple(st: St) -> Vec<W> {
    let h = header();
    let scale = h.map(|h| b2f(h.get(fld!(working_scale_bits)))).unwrap_or(1.0);
    let q = QUALITY.iter().enumerate().min_by(|a, b| (a.1 .0 - scale).abs().total_cmp(&(b.1 .0 - scale).abs())).map(|(i, _)| i).unwrap_or(2);
    let quality = SegmentedButton::new(vec![t!("Быстро"), t!("Баланс"), t!("Максимум")]).selected(q).on_change(move |i| {
        let (sc, mq) = QUALITY[i.min(2)];
        st.write(fld!(working_scale_bits), f2b(sc), false);
        st.write(fld!(mvec_quality), mq, false);
    });
    let compare_on = h.map(|h| h.get(fld!(compare_mode)) != 0).unwrap_or(false);
    let compare = Toggle::with_state(compare_on).on_change(move |on| {
        st.write(fld!(compare_mode), if on { 2 } else { 0 }, false);
        st.rev.update(|r| *r += 1);
    });
    let mut look = vec![
        c::bool_row(st, t!("Включено"), t!("Выключено — игры и видео показывают свои кадры без изменений"), fld!(enabled), Live),
        c::choice_row(st, t!("Стиль"), t!("«Естественный» бережнее к лицам, «Кинематографичный» — с плёночным тоном"), fld!(style), &styles(), AtCreate),
        c::row(t!("Качество"), t!("Разрешение модели 50, 75 или 100% кадра и точность оценки движения. Ниже — быстрее"), quality),
        c::slider_row(st, t!("Сила эффекта"), t!("0 — кадр как есть, 1 — как задумано моделью"), fld!(intensity_bits), (0.0, 2.0, 0.05), 2, AtCreate),
        c::slider_row(st, t!("Резкость"), t!("Действует сразу"), fld!(sharpness_bits), (0.0, 1.0, 0.05), 2, Live),
        c::row(t!("Сравнение"), t!("Шторка в кадре: слева — без DLSS 5"), compare),
    ];
    if compare_on {
        look.push(c::slider_row(st, t!("Граница"), "", fld!(compare_split_bits), (0.0, 1.0, 0.01), 2, Live));
    }

    let steam = "VKLayer_DLSS5=1 %command%";
    let where_rows = vec![
        syn_wine_row(),
        c::row(
            t!("Игры Steam"),
            t!("Параметры запуска игры: {c}. Игре нужен Vulkan (для Direct3D — DXVK, в Proton он есть)", c = steam),
            icon_button(icons::COPY, t!("Копировать"), "tb-btn", move || {
                syngui::clipboard::copy(steam);
                st.toast(t!("Строка скопирована — вставьте её в «Параметры запуска» игры"));
            }),
        ),
        c::row(t!("Видеоплеер"), t!("Кнопка «Фильтры» в шапке плеера. У видео свой профиль — эти настройки на него не действуют"), Text::new("").class("hint")),
    ];

    let mut out = vec![c::group(t!("Картинка"), look), c::group(t!("Где работает"), where_rows)];

    // игра со слоем (не видеоплеер): свои настройки для неё
    let s = st.status.get_untracked();
    if !s.game.is_empty() && !s.video {
        let row = match st.game_profile.get_untracked() {
            Some(g) => c::row(
                t!("Свои настройки «{g}»", g = g),
                t!("Правки сейчас сохраняются только для этой игры; остальные игры — на общих"),
                Button::new(t!("Забыть")).class("btn ghost").on_click(move || st.forget_game_profile()),
            ),
            None => c::row(
                t!("Игра «{g}»", g = s.game),
                t!("Сейчас общие настройки. Сохраните — и в следующий раз эта игра откроется со своими"),
                Button::new(t!("Сохранить для игры")).icon(icons::SAVE).class("btn").on_click(move || st.save_game_profile()),
            ),
        };
        out.push(c::group(t!("Эта игра"), vec![row]));
    }

    let adv = Toggle::with_state(st.advanced.get_untracked()).on_change(move |on| {
        st.advanced.set(on);
        if on {
            st.tab.set(crate::state::Tab::Simple);
        }
        st.save();
    });
    out.push(c::group(t!("Ещё"), vec![c::row(t!("Все настройки"), t!("Вкладки с тонкой настройкой модели, движения, композиции, отладки и помощника"), adv)]));
    out
}

pub fn rendering(st: St) -> Vec<W> {
    vec![
        c::group(
            t!("Нейрорендеринг"),
            vec![
                c::bool_row(st, t!("Включено"), t!("Выключено — игра показывает свой кадр без изменений"), fld!(enabled), Live),
                c::choice_row(st, t!("Стиль"), t!("Профиль обработки самой модели"), fld!(style), &styles(), AtCreate),
                c::int_row(st, t!("Пресет"), t!("Пресет рендеринга модели"), fld!(preset), 0, 15, AtCreate),
                c::slider_row(st, t!("Интенсивность"), t!("Насколько сильно работает модель"), fld!(intensity_bits), (0.0, 4.0, 0.05), 2, AtCreate),
                c::slider_row(st, t!("Локальная структура"), "", fld!(local_structure_bits), (0.0, 4.0, 0.05), 2, AtCreate),
                c::slider_row(st, t!("Локальный тон"), "", fld!(local_tone_bits), (0.0, 4.0, 0.05), 2, AtCreate),
                c::slider_row(st, t!("Структура кожи"), t!("−1 — как локальная структура (так по умолчанию у модели), не нулевая сила"), fld!(skin_structure_bits), (-1.0, 4.0, 0.05), 2, AtCreate),
                c::bool_row(st, t!("Автомаска кожи"), t!("Модель сама находит кожу"), fld!(auto_mask), AtCreate),
                c::slider_row(st, t!("Резкость"), t!("Единственная сила, которую модель читает каждый кадр — действует сразу"), fld!(sharpness_bits), (0.0, 1.0, 0.05), 2, Live),
            ],
        ),
        c::group(
            t!("Стоимость"),
            vec![
                c::int_row(st, t!("Проходы"), t!("Сколько раз модель обрабатывает кадр; каждый проход — ещё один полный запуск"), fld!(passes), 1, shm::MAX_PASSES as u32, AtCreate),
                c::percent_row(st, t!("Разрешение модели, %"), t!("Доля разрешения кадра, на которой работает модель; сам кадр не уменьшается. Выше 100% — суперсэмплинг"), fld!(working_scale_bits), 25.0, 200.0, Live),
                c::choice_row(
                    st,
                    t!("Фильтр уменьшения"),
                    t!("Как ответ суперсэмплинга сводится к размеру кадра (только выше 100%)"),
                    fld!(scaling_downscaler),
                    &opts(&[t!("FSR1 (не поддерживается)"), "Bicubic".into(), "Catmull-Rom".into(), "Lanczos2".into(), "Lanczos3".into(), "Kaiser2".into(), "Kaiser3".into(), "Magic".into()]),
                    Live,
                ),
            ],
        ),
        passes(st),
    ]
}

/// Отдельные проходы: проход следует общим настройкам во всём, что у него не переопределено.
fn passes(st: St) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let sel = st.pass_sel.get();
        let Some(h) = header() else { return vec![] };
        let pc = &h.pass[sel];
        let mask = pc.override_mask.load(std::sync::atomic::Ordering::SeqCst);
        let mut pick = Dropdown::new().width(160.0).max_height(360.0);
        for i in 0..shm::MAX_PASSES {
            pick = pick.item(DropdownItem::new(i.to_string(), t!("Проход {n}", n = i + 1)));
        }
        let pick = pick.selected(sel.to_string()).on_change(move |v: &str| {
            if let Ok(i) = v.parse::<usize>() {
                st.pass_sel.set(i.min(shm::MAX_PASSES - 1));
            }
        });
        let mut rows: Vec<W> = vec![c::row(t!("Проход"), t!("Значения читаются при сборке функции прохода — изменение появится не сразу"), pick)];
        type PF = crate::shm::PassField;
        let float_ov = |label: String, bit: u32, f: PF, min: f32, max: f32| -> W {
            let on = mask & bit != 0;
            let v = b2f(f(pc).load(std::sync::atomic::Ordering::SeqCst)).clamp(min, max);
            let slider = Slider::new().range(min, max).step(0.05).value(v).show_value(2).width(200.0).disabled(!on).on_change(move |x| {
                st.write_pass(sel, bit, true, Some((f, f2b((x * 100.0).round() / 100.0))));
            });
            let toggle = Toggle::with_state(on).on_change(move |x| {
                st.write_pass(sel, bit, x, None);
                st.rev.update(|r| *r += 1);
            });
            c::row(label, "", Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(toggle).child(slider))
        };
        rows.push(float_ov(t!("Интенсивность"), shm::OV_INTENSITY, pfld!(intensity_bits), 0.0, 4.0));
        rows.push(float_ov(t!("Локальная структура"), shm::OV_LOCAL_STRUCTURE, pfld!(local_structure_bits), 0.0, 4.0));
        rows.push(float_ov(t!("Локальный тон"), shm::OV_LOCAL_TONE, pfld!(local_tone_bits), 0.0, 4.0));
        rows.push(float_ov(t!("Структура кожи"), shm::OV_SKIN_STRUCTURE, pfld!(skin_structure_bits), -1.0, 4.0));
        rows.push(float_ov(t!("Резкость"), shm::OV_SHARPNESS, pfld!(sharpness_bits), 0.0, 1.0));
        {
            let on = mask & shm::OV_STYLE != 0;
            let mut dd = Dropdown::new().width(200.0).disabled(!on);
            for (i, s) in styles().into_iter().enumerate() {
                dd = dd.item(DropdownItem::new(i.to_string(), s));
            }
            let dd = dd.selected(pc.style.load(std::sync::atomic::Ordering::SeqCst).min(2).to_string()).on_change(move |v: &str| {
                if let Ok(i) = v.parse::<u32>() {
                    st.write_pass(sel, shm::OV_STYLE, true, Some((pfld!(style), i)));
                }
            });
            let tg = Toggle::with_state(on).on_change(move |x| {
                st.write_pass(sel, shm::OV_STYLE, x, None);
                st.rev.update(|r| *r += 1);
            });
            rows.push(c::row(t!("Стиль"), "", Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tg).child(dd)));
        }
        {
            let on = mask & shm::OV_PRESET != 0;
            let sp = SpinBox::new().range(0.0, 15.0).step(1.0).value(pc.preset.load(std::sync::atomic::Ordering::SeqCst).min(15) as f64).width(120.0).disabled(!on).on_change(move |v| {
                st.write_pass(sel, shm::OV_PRESET, true, Some((pfld!(preset), v.round().max(0.0) as u32)));
            });
            let tg = Toggle::with_state(on).on_change(move |x| {
                st.write_pass(sel, shm::OV_PRESET, x, None);
                st.rev.update(|r| *r += 1);
            });
            rows.push(c::row(t!("Пресет"), "", Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tg).child(sp)));
        }
        {
            let on = mask & shm::OV_AUTO_MASK != 0;
            let am = Toggle::with_state(pc.auto_mask.load(std::sync::atomic::Ordering::SeqCst) != 0).disabled(!on).on_change(move |v| {
                st.write_pass(sel, shm::OV_AUTO_MASK, true, Some((pfld!(auto_mask), u32::from(v))));
            });
            let tg = Toggle::with_state(on).on_change(move |x| {
                st.write_pass(sel, shm::OV_AUTO_MASK, x, None);
                st.rev.update(|r| *r += 1);
            });
            rows.push(c::row(t!("Автомаска кожи"), "", Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(tg).child(am)));
        }
        vec![c::group(t!("Отдельные проходы"), rows)]
    });
    boxed(body)
}

pub fn quality(st: St) -> Vec<W> {
    vec![
        c::group(
            t!("Движение"),
            vec![
                c::bool_row(st, t!("Оценивать векторы движения"), t!("Слой не получает векторы от движка — они оцениваются оптическим потоком GPU по двум кадрам. Выключено — модель получает нулевое поле"), fld!(mvec_enabled), Live),
                c::choice_row(st, t!("Качество оценки"), t!("Какую долю времени кадра может занять оптический поток"), fld!(mvec_quality), &[t!("Быстро"), t!("Сбалансировано"), t!("Качество")], Live),
                c::choice_row(st, t!("Единицы векторов"), t!("Что числа поля значат для модели; оценка даёт пиксели"), fld!(mvec_scale_mode), &[t!("Нормированные"), t!("Пиксели"), "UV 0..1".into()], Live),
                c::choice_row(st, t!("Шаг сетки"), t!("Шаг сетки оптического потока в пикселях исходного кадра"), fld!(mvec_pixel_size), &["1 px".into(), "2 px".into(), "4 px".into(), "8 px".into()], Live),
            ],
        ),
        c::group(
            t!("Ввод и точность"),
            vec![
                c::choice_row(st, t!("HDR-вход"), t!("Авто — модель видит настоящий свет кадра, когда swapchain в HDR (float или 10 бит PQ). Принудительно — и для SDR, для сравнения"), fld!(hdr_mode), &[t!("Авто"), t!("Выключен"), t!("Принудительно float16")], Live),
                c::bool_row(st, t!("16-битные промежуточные SDR"), t!("Кадры между проходами в 16 битах; выключено — 8 бит, меньше видеопамяти. HDR всегда float16"), fld!(sdr16_multipass), AtCreate),
            ],
        ),
    ]
}

pub fn composition(st: St) -> Vec<W> {
    let comp = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let bypass = st.bypass.get();
        let toggle = Toggle::with_state(!bypass).on_change(move |on| {
            st.write(fld!(composition_bypass), u32::from(!on), false);
            st.bypass.set(!on);
        });
        let mut rows = vec![c::row(t!("Включена"), t!("Выключена — ответ модели показывается как кадр. Включена — ответ смешивается с кадром в пределах ниже"), toggle)];
        if !bypass {
            rows.extend([
                c::slider_row(st, t!("Сила деталей"), t!("Сколько правки модели доходит до кадра; 0 — кадр игры без изменений"), fld!(transfer_strength_bits), (0.0, 4.0, 0.05), 2, Live),
                c::slider_row(st, t!("Сила цвета"), t!("Сколько цвета модели приходит вместе со светом; 0 — оттенок игры"), fld!(colour_strength_bits), (0.0, 4.0, 0.05), 2, Live),
                c::int_row(st, t!("Предел цвета, %"), t!("Насколько модель может увести цвет пикселя от игрового; 200 — по умолчанию, ниже — если по краям цветная кайма"), fld!(colour_trust_percent), 0, 800, Live),
                c::int_slider_row(st, t!("Сглаживание освещения, %"), t!("Коэффициент освещения берётся с окрестности пикселя — позволяет поднять защиту светлых без пятен"), fld!(ratio_smooth_percent), 0, 100, 5, Live),
                c::slider_row(st, t!("Защита светлых"), t!("Насколько проход может сдвинуть свет окрестности; одиночный пиксель ограничен отдельно"), fld!(max_ratio_bits), (1.0, 30.0, 0.5), 1, Live),
                c::choice_row(st, t!("Как применяется ответ"), t!("Если модель работала не в разрешении кадра. «Кадр + правка» — без деления, края остаются родной резкости"), fld!(transfer), &[t!("Классически"), t!("Согласованный остаток"), t!("Кадр + правка")], Live),
            ]);
        }
        vec![c::group(t!("Композиция"), rows)]
    });
    vec![
        boxed(comp),
        c::group(
            t!("Цвет"),
            vec![
                c::choice_row(st, t!("В кадре"), t!("Готовая картинка (игра уже тонировала) или линейный свет; «Авто» решает по формату"), fld!(colour_mode), &[t!("Авто"), t!("Готовая картинка"), t!("Линейный свет")], Live),
                c::choice_row(st, t!("Белая точка"), t!("Имеет смысл только для линейного кадра"), fld!(white_point_source), &[t!("Из поля ниже"), t!("Измерять по кадру")], Live),
                c::float_row(st, t!("Белый «бумаги»"), t!("Что модель считает белым, если точка не измеряется"), fld!(white_point_bits), (0.01, 2000.0, 0.1), Live),
                c::float_row(st, t!("Масштаб белой точки"), t!("Умножает любую белую точку; выше — светлые ниже на кривой"), fld!(white_point_scale_bits), (0.01, 100.0, 0.05), Live),
                c::float_row(st, t!("Поправка измерения"), t!("Умножает только измеренную белую точку"), fld!(white_point_trim_bits), (0.01, 100.0, 0.05), Live),
            ],
        ),
    ]
}

/// Коды клавиш Linux (KEY_*), которые слой умеет ловить.
const KEYS: &[(&str, u32)] = &[
    ("F1", 59), ("F2", 60), ("F3", 61), ("F4", 62), ("F5", 63), ("F6", 64), ("F7", 65), ("F8", 66), ("F9", 67),
    ("F10", 68), ("F11", 87), ("F12", 88), ("Home", 102), ("End", 107), ("Insert", 110), ("Delete", 111),
    ("PageUp", 104), ("PageDown", 109), ("Pause", 119), ("ScrollLock", 70), ("`", 41),
];

pub fn inspect(st: St) -> Vec<W> {
    let key_now = header().map(|h| h.toggle_key.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0);
    let mut keys = Dropdown::new().width(240.0).max_height(360.0).item(DropdownItem::new("0", t!("Нет")));
    for (n, code) in KEYS {
        keys = keys.item(DropdownItem::new(code.to_string(), *n));
    }
    let keys = keys
        .selected(if KEYS.iter().any(|(_, c)| *c == key_now) { key_now.to_string() } else { "0".into() })
        .on_change(move |v: &str| {
            if let Ok(code) = v.parse::<u32>() {
                st.write(fld!(toggle_key), code, false);
            }
        });
    let cmd = format!("dlssnr-shmctl {} toggle enabled", crate::paths::shm_path().display());
    let cmd2 = cmd.clone();
    let shortcut = Column::new()
        .gap(8.0)
        .class("setting-row-wide")
        .child(Text::new(t!("Клавиша не работает (игра под Wayland или нет группы input) — назначьте сочетание в настройках рабочего стола на команду:")).max_lines(3).class("row-hint"))
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow code").child(Text::new(cmd).max_lines(2).class("code-text")))
                .child(icon_button(icons::COPY, t!("Копировать"), "tb-btn", move || {
                    syngui::clipboard::copy(&cmd2);
                    st.toast(t!("Команда скопирована"));
                })),
        );
    let frames = use_signal(8u32);
    let capture = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(SpinBox::new().range(1.0, 64.0).step(1.0).value(8.0).width(110.0).on_change(move |v| frames.set(v.round().clamp(1.0, 64.0) as u32)))
        .child(Button::new(t!("Снять")).icon(icons::CAMERA).class("btn").on_click(move || {
            if let Some(h) = header() {
                h.set(fld!(capture_request), frames.get_untracked(), false);
                st.toast(t!("Пары кадров до/после пишутся в каталог состояния"));
            }
        }));
    vec![
        c::group(
            t!("Проверка"),
            vec![
                c::bool_row(st, t!("Применять правку модели"), t!("Выключено — проход работает целиком, но показывается чистый кадр: честное сравнение при той же цене"), fld!(apply_model), Live),
                c::bool_row(st, t!("Заморозить кадр"), t!("Смена настройки пересчитывается на той же картинке — единственный чистый способ сравнить две настройки"), fld!(hold_frame), Live),
                c::choice_row(st, t!("Прокси"), t!("Какую картинку видит модель и смешивается её ответ с кадром или заменяет его; режимы замены мигают на ярком свете"), fld!(reversible_mode), &[t!("Мягкое колено"), "Neutwo".into(), t!("Neutwo, замена"), t!("Гибрид"), t!("Гибрид, замена")], Live),
                c::choice_row(
                    st,
                    t!("Отладочный вид"),
                    t!("«Что изменила» — усилено и центрировано на сером; предел цвета — зелёным, где цвет модели проходит целиком, красным — где сдержан"),
                    fld!(debug_view),
                    &[t!("Выключен"), t!("Что видела модель"), t!("Её сырой ответ"), t!("Что изменила"), t!("Где срабатывает предел цвета"), t!("Цвет до предела")],
                    Live,
                ),
                c::float_row(st, t!("Масштаб отладки"), t!("На что умножаются отладочные виды"), fld!(debug_scale_bits), (0.01, 100.0, 0.1), Live),
                c::choice_row(st, t!("Сравнение"), t!("Проход против самого себя; шторка режет один кадр без пересчёта"), fld!(compare_mode), &[t!("Выключено"), t!("Рядом"), t!("Шторка")], Live),
                c::slider_row(st, t!("Граница"), "", fld!(compare_split_bits), (0.0, 1.0, 0.01), 2, Live),
                c::slider_row(st, t!("Увеличение"), t!("Только «Рядом»: 1 — весь кадр с полями, 2 — заполнить половину"), fld!(compare_zoom_bits), (1.0, 2.0, 0.05), 2, Live),
                c::bool_row(st, t!("Поменять стороны"), t!("С какой стороны обработанный кадр"), fld!(compare_swap), Live),
            ],
        ),
        c::group(
            t!("Клавиша и снимки"),
            vec![
                c::row(t!("Клавиша переключения"), t!("Слой читает её сам: X11/XWayland, gamescope или группа input"), keys),
                boxed(shortcut),
                c::row(t!("Снять пары кадров"), t!("Совпадающие кадры до/после — в каталог состояния"), capture),
            ],
        ),
    ]
}

pub fn helper(st: St) -> Vec<W> {
    // Среда запуска помощника (не игры): syn-wine или Proton-подобное средство Steam.
    let (kind, path) = st.runner.get_untracked();
    let wine = crate::paths::which("wine").map(|p| p.display().to_string()).unwrap_or_else(|| "/usr/bin/wine".into());
    let mut runners: Vec<(String, String, String)> = vec![(format!("wine|{wine}"), t!("syn-wine (системный Wine)"), wine.clone())];
    for (name, exe) in crate::paths::discover_runners() {
        let e = exe.display().to_string();
        runners.push((format!("proton|{e}"), name, e));
    }
    let cur = if path.is_empty() {
        format!("wine|{wine}")
    } else if let Some(r) = runners.iter().find(|r| r.2 == path) {
        r.0.clone()
    } else {
        let k = format!("{}|{path}", if kind.is_empty() { "wine" } else { kind.as_str() });
        runners.push((k.clone(), t!("Свой: {p}", p = path), path.clone()));
        k
    };
    let mut dd = Dropdown::new().width(280.0).max_height(360.0);
    for (k, label, _) in &runners {
        dd = dd.item(DropdownItem::new(k.clone(), label.clone()));
    }
    let runner = dd.selected(cur).on_change(move |v: &str| {
        if let Some((k, p)) = v.split_once('|') {
            st.set_runner(k, p);
        }
    });

    let profiles = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let list = st.profiles.get();
        let cur = st.profile.get();
        let mut dd = Dropdown::new().width(220.0).max_height(360.0).item(DropdownItem::new("", t!("(по умолчанию)")));
        for p in &list {
            dd = dd.item(DropdownItem::new(p.clone(), p.clone()));
        }
        let dd = dd.selected(cur.clone()).on_change(move |v: &str| st.load_profile(v));
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(dd);
        row = row.child(icon_button(icons::SAVE, t!("Сохранить профиль…"), "tb-btn", move || st.dialog.set(Dialog::SaveProfile)));
        if !cur.is_empty() {
            let name = cur.clone();
            row = row.child(icon_button(icons::REFRESH, t!("Загрузить профиль заново"), "tb-btn", move || st.load_profile(&name)));
            let name = cur.clone();
            row = row.child(icon_button(icons::DELETE, t!("Удалить профиль"), "tb-btn", move || st.dialog.set(Dialog::DeleteProfile(name.clone()))));
        }
        vec![c::row(t!("Профиль"), t!("Наборы настроек в ~/.config/dlssnr/profiles; «по умолчанию» — сброс"), row)]
    });

    let bins = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let dir = st.binaries.get();
        let _ = st.status.get().model_missing;
        let model = std::path::Path::new(&dir).join("nvngx_dlssnr.dll").is_file();
        let core = std::path::Path::new(&dir).join("nvngx.dll").is_file();
        let state = match (model, core) {
            (true, true) => t!("nvngx_dlssnr.dll и nvngx.dll на месте"),
            (true, false) => t!("Модель на месте; nvngx.dll — из драйвера при запуске"),
            (false, _) => t!("Нет nvngx_dlssnr.dll — нейрорендеринг выключен"),
        };
        let dir2 = dir.clone();
        let buttons = Row::new()
            .gap(8.0)
            .child(Button::new(t!("Импортировать…")).icon(icons::IMPORT).class(if model { "btn" } else { "btn primary" }).on_click(move || st.dialog.set(Dialog::Import)))
            .child(icon_button(icons::FOLDER, t!("Открыть каталог"), "tb-btn", move || open_path(std::path::Path::new(&dir2))));
        vec![
            c::row(t!("DLL NVIDIA NGX"), format!("{dir}\n{state}"), buttons),
        ]
    });

    let log = crate::paths::default_log();
    vec![
        c::group(
            t!("Запуск"),
            vec![
                c::row(t!("Среда помощника"), t!("Помощник — Windows-программа (модель — Windows DLL), ему нужен Wine или Proton; от игры это не зависит"), runner),
                syn_wine_row(),
                c::int_row(st, t!("Пауза пересборки, мс"), t!("Сколько помощник ждёт после смены настройки модели перед пересборкой; поднимите, если модель перестаёт отвечать"), fld!(rebuild_settle_ms), 0, 5000, Live),
                c::row(
                    t!("Журнал помощника"),
                    log.display().to_string(),
                    icon_button(icons::LOG, t!("Открыть журнал"), "tb-btn", move || open_path(&log)),
                ),
            ],
        ),
        c::group(t!("Настройки"), vec![boxed(profiles), boxed(bins), c::row(t!("Сбросить всё"), t!("Все настройки — по умолчанию; помощник пересоберёт функции"), Button::new(t!("Сбросить…")).class("btn ghost").on_click(move || st.reset_open.set(true)))]),
    ]
}
