//! Окно DLSS 5: своя рамка (заголовок со знаком, перетаскивание, кнопки окна, изменение размера),
//! карточка состояния помощника и игры, вкладки настроек, диалоги импорта и профилей.

mod controls;
mod pages;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;

use crate::icons;
use crate::shm;
use crate::state::{Dialog, Layer, St, Status, Tab};

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

pub fn icon_button(glyph: &str, tooltip: impl Into<String>, class: &str, on_click: impl FnMut() + Send + 'static) -> W {
    boxed(Tooltip::new(Button::new("").icon(glyph).class(&format!("btn round flat {class}")).on_click(on_click), tooltip).delay_ms(500))
}

fn brand_mark() -> W {
    boxed(DecoratedBox::new().class("brand").child(icon(icons::BRAND).class("ic brand-ic")))
}

fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let title = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.status.get();
        let (class, text) = layer_badge(&s);
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(DecoratedBox::new().class(class));
        row = row.child(Text::new(text).max_lines(1).elide(Elide::End).class("tb-title"));
        vec![boxed(row)]
    });
    boxed(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().class("tb-left").child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(brand_mark()).child(Text::new("DLSS 5").class("brand-title"))))
                .child(DecoratedBox::new().class("grow drag-wrap").child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space").child(title))))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Игра со слоем: кадры идут, слой подключён без кадров (пауза, свёрнуто), игры нет.
fn layer_badge(s: &Status) -> (&'static str, String) {
    let game = if s.game.is_empty() { String::new() } else { format!(" — {}", s.game) };
    match s.layer {
        Layer::Active => ("dot ok", format!("{}{game}", t!("Игра обрабатывается"))),
        Layer::Idle => ("dot idle", format!("{}{game}", t!("Игра на паузе"))),
        Layer::None => ("dot off", t!("Нет игры со слоем")),
    }
}

fn helper_text(s: &Status) -> (String, &'static str) {
    if !s.attached {
        return (t!("Общая память недоступна"), "dot bad");
    }
    if let Some(v) = s.mismatch {
        return (t!("Общая память версии {v}, окно — версии {w}: слой или помощник устарели — обновите их вместе", v = v, w = shm::VERSION), "dot bad");
    }
    if !s.running {
        return (t!("Помощник остановлен"), "dot off");
    }
    match s.helper_state {
        shm::HELPER_STARTING => (t!("Помощник запускается"), "dot idle"),
        shm::HELPER_NO_VULKAN => (t!("Нет устройства NVIDIA с расширениями NVX"), "dot bad"),
        shm::HELPER_NO_BINARIES => (t!("Нет DLL NGX"), "dot bad"),
        shm::HELPER_MODEL_FAILED => (t!("Модель не запустилась"), "dot bad"),
        shm::HELPER_RUNNING if s.model_up => (t!("Модель работает"), "dot ok"),
        shm::HELPER_RUNNING => (t!("Помощник готов, ждёт кадры"), "dot ok"),
        // процесс уже есть, а заголовок ещё не обновлён
        shm::HELPER_STOPPED => (t!("Помощник запускается"), "dot idle"),
        _ => (t!("Помощник: неизвестное состояние"), "dot idle"),
    }
}

fn status_card(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let s = st.status.get();
        let busy = st.busy.get();
        let (text, dot) = helper_text(&s);
        let mut info = Column::new().gap(3.0).class("grow").child(
            Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(dot)).child(Text::new(text).max_lines(2).class("status-title")),
        );
        let mut details = Vec::new();
        if !s.reason.is_empty() {
            details.push(s.reason.clone());
        }
        if s.running && s.helper_frames > 0 {
            details.push(t!("кадров {n} · модель {e} мс · туда {u} мс · обратно {r} мс", n = s.helper_frames, e = format!("{:.1}", s.eval_ms), u = format!("{:.1}", s.upload_ms), r = format!("{:.1}", s.readback_ms)));
        }
        if s.running && (s.vram_mb > 0 || s.features > 0) {
            details.push(t!("видеопамять {v} МБ · функций NGX {f}", v = s.vram_mb, f = s.features));
        }
        if s.layer != Layer::None {
            let mut l = t!("слой {w}×{h} · {ms} мс", w = s.layer_size.0, h = s.layer_size.1, ms = format!("{:.1}", s.layer_ms));
            l.push_str(if s.dmabuf { " · dma-buf" } else { "" });
            if s.hdr {
                l.push_str(" · HDR");
            }
            details.push(l);
            if !s.layer_reason.is_empty() {
                details.push(s.layer_reason.clone());
            }
        }
        for d in details {
            info = info.child(Text::new(d).max_lines(2).elide(Elide::End).class("hint"));
        }
        let action: W = if let Some(b) = busy {
            boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(CircularProgress::new().indeterminate().size(18.0).stroke_width(2.5).class("spin")).child(Text::new(b).class("hint")))
        } else if s.running {
            boxed(
                Row::new()
                    .gap(8.0)
                    .child(icon_button(icons::RESTART, t!("Перезапустить помощник"), "tb-btn", move || st.restart_helper()))
                    .child(Button::new(t!("Остановить")).icon(icons::STOP).class("btn ghost").on_click(move || st.helper("stop"))),
            )
        } else {
            boxed(Button::new(t!("Запустить помощник")).icon(icons::PLAY).class("btn primary").on_click(move || st.helper("start")))
        };
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).child(info).child(action));
        if s.model_missing {
            col = col.child(
                DecoratedBox::new().class("banner").child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(icons::WARNING).class("ic warn-ic"))
                        .child(
                            Column::new()
                                .gap(2.0)
                                .class("grow")
                                .child(Text::new(t!("Нет модели nvngx_dlssnr.dll")).class("banner-title"))
                                .child(Text::new(t!("Без неё нейрорендеринг выключен и игры показывают свои кадры. Это проприетарный файл NVIDIA, в пакет он не входит — импортируйте свой.")).max_lines(3).class("hint")),
                        )
                        .child(Button::new(t!("Импортировать…")).icon(icons::IMPORT).class("btn primary").on_click(move || st.dialog.set(Dialog::Import))),
                ),
            );
        }
        vec![boxed(DecoratedBox::new().class("status-card").child(col))]
    }))
}

fn tabs(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = st.tab.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).class("tabs");
        for t in Tab::ALL {
            row = row.child(GestureDetector::new().on_click(move || st.tab.set(t)).child(DecoratedBox::new().class(if t == cur { "tab on" } else { "tab" }).child(Text::new(t.label()).class("tab-text"))));
        }
        vec![boxed(row)]
    }))
}

fn content(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let tab = st.tab.get();
        let _ = st.rev.get();
        let groups = match tab {
            Tab::Rendering => pages::rendering(st),
            Tab::Quality => pages::quality(st),
            Tab::Composition => pages::composition(st),
            Tab::Inspect => pages::inspect(st),
            Tab::Helper => pages::helper(st),
        };
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("page");
        for g in groups {
            col = col.child(g);
        }
        vec![boxed(ScrollView::new().vertical().child(col))]
    }))
}

fn toast_layer(st: St) -> W {
    let toast = Reactive::new(move || -> Vec<W> {
        let Some((_, text)) = st.toast.get() else { return vec![] };
        vec![boxed(DecoratedBox::new().class("toast").child(Text::new(text).max_lines(3).class("toast-text")))]
    });
    boxed(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).class("toast-wrap").child(toast))
}

fn modal(st: St, card: impl Widget + 'static) -> W {
    let scrim = GestureDetector::new().on_click(move || st.dialog.set(Dialog::None)).child(DecoratedBox::new().class("scrim"));
    let centered = Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).class("dialog-wrap").child(DecoratedBox::new().class("dialog-card").child(card));
    boxed(Stack::new().fit(StackFit::Expand).child(scrim).child(centered))
}

fn dialogs(st: St) -> W {
    let d = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        match st.dialog.get() {
            Dialog::None => vec![],
            Dialog::Import => {
                let start = st.binaries_dir();
                let home = std::env::var("HOME").unwrap_or_default();
                // picked — выбранное в диалоге (поле перестраивается), typed — набранное (без перестройки)
                let picked = use_signal(String::new());
                let typed = use_signal(String::new());
                let field = Reactive::new(move || -> Vec<W> {
                    let p = picked.get();
                    vec![boxed(
                        TextField::new()
                            .text(p)
                            .placeholder(t!("Каталог с nvngx_dlssnr.dll"))
                            .autofocus(true)
                            .on_change(move |t| typed.set(t.to_string()))
                            .on_submit(move |t| st.import(t))
                            .on_escape(move || st.dialog.set(Dialog::None))
                            .class("field"),
                    )]
                });
                let browse = Button::new(t!("Обзор…")).icon(icons::FOLDER).class("btn").on_click(move || {
                    let start = if start.is_dir() { start.clone() } else { std::path::PathBuf::from(&home) };
                    std::thread::spawn(move || {
                        if let Some(p) = crate::state::pick_dir(&start) {
                            run_on_main_thread(move || {
                                typed.set(p.clone());
                                picked.set(p);
                            });
                        }
                    });
                });
                let card = Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(t!("Импорт DLL NVIDIA NGX")).class("card-title"))
                    .child(Text::new(t!("Нужен каталог с nvngx_dlssnr.dll; вместе с ней копируются nvngx.dll, nvapi64.dll, sl.*.dll и лицензии, если есть. Файлы попадают в {p}.", p = crate::paths::default_binaries().display())).max_lines(5).class("hint"))
                    .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(field)).child(browse))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(t!("Отмена")).class("btn flat").on_click(move || st.dialog.set(Dialog::None)))
                            .child(Button::new(t!("Импортировать")).class("btn primary").on_click(move || st.import(&typed.get_untracked()))),
                    );
                vec![modal(st, card)]
            }
            Dialog::SaveProfile => {
                let cur = st.profile.get_untracked();
                let name = use_signal(cur.clone());
                let card = Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(t!("Сохранить профиль")).class("card-title"))
                    .child(
                        TextField::new()
                            .text(cur)
                            .placeholder(t!("Название"))
                            .autofocus(true)
                            .on_change(move |t| name.set(t.to_string()))
                            .on_submit(move |t| st.save_profile(t))
                            .on_escape(move || st.dialog.set(Dialog::None))
                            .class("field"),
                    )
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(t!("Отмена")).class("btn flat").on_click(move || st.dialog.set(Dialog::None)))
                            .child(Button::new(t!("Сохранить")).class("btn primary").on_click(move || st.save_profile(&name.get_untracked()))),
                    );
                vec![modal(st, card)]
            }
            Dialog::DeleteProfile(n) => {
                let n2 = n.clone();
                let card = Column::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(t!("Удалить профиль «{n}»?", n = n)).class("card-title"))
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(t!("Отмена")).class("btn flat").on_click(move || st.dialog.set(Dialog::None)))
                            .child(Button::new(t!("Удалить")).class("btn primary").on_click(move || st.delete_profile(&n2))),
                    );
                vec![modal(st, card)]
            }
        }
    });
    let reset = ConfirmDialog::new(t!("Сбросить все настройки?"), t!("Все настройки вернутся к значениям по умолчанию; помощник пересоберёт функции модели."), st.reset_open, move |ok| {
        if ok {
            st.reset_all();
        }
    })
    .icon(icons::RESTART)
    .width(380.0);
    boxed(Stack::new().fit(StackFit::Expand).child(d).child(reset))
}

fn keys(st: St, inner: impl Widget + 'static) -> W {
    boxed(
        EventHook::new()
            .on_key_down(move |k, m| {
                if k == Key::Escape && st.dialog.get_untracked() != Dialog::None {
                    st.dialog.set(Dialog::None);
                    return KeyReply::Handled;
                }
                if m.ctrl && !m.alt {
                    let tab = match k {
                        Key::Num1 => Tab::Rendering,
                        Key::Num2 => Tab::Quality,
                        Key::Num3 => Tab::Composition,
                        Key::Num4 => Tab::Inspect,
                        Key::Num5 => Tab::Helper,
                        _ => return KeyReply::Ignore,
                    };
                    st.tab.set(tab);
                    return KeyReply::Handled;
                }
                KeyReply::Ignore
            })
            .child(inner),
    )
}

pub fn root(st: St) -> W {
    let main = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("window")
        .child(titlebar(st))
        .child(DecoratedBox::new().class("head").child(Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(status_card(st)).child(tabs(st))))
        .child(DecoratedBox::new().class("grow body").child(content(st)));
    let stack = Stack::new().fit(StackFit::Expand).child(main).child(toast_layer(st)).child(dialogs(st));
    let frame = DecoratedBox::new().class("window-frame").clip(true).child(keys(st, stack));
    boxed(WindowResizeRegion::new().over_content(true).inset(5.0).child(frame))
}
