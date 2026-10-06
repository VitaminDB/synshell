//! Виджет клавиатуры и её состояние. Поверхность создаётся при показе и
//! закрывается при скрытии (как домашний экран synmobile-shell); её высота —
//! из числа рядов, exclusive zone той же высоты отодвигает окна.

use synshell_common::haptics::{self, Feedback};
use std::cell::Cell;
use std::time::Duration;

use syngui::prelude::*;
use syngui::StyleValue;
use syngui::widget::WidgetExt;
use syngui::GestureDetector;
use syngui::containers::Positioned;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

pub use crate::layout::Action;
use crate::layout::{self, Key, Modifier, Page, LANGS};

/// Высота обычного ряда, функционального ряда, отступы (логические px).
const KEY_H: u32 = 46;
const FN_H: u32 = 34;
const GAP: u32 = 5;
const PAD: u32 = 6;
/// Функциональные ряды — отдельным блоком: внутренний отступ и промежуток
/// до основной клавиатуры.
const FN_PAD: u32 = 4;
const FN_SEP: u32 = 8;
/// Прозрачный запас над клавиатурой под всплывающую букву нажатой клавиши
/// (как в Gboard): поверхность выше на него, но окна он не отодвигает и
/// касания пропускает насквозь (input region — только клавиатура).
const HEADROOM: u32 = 64;
/// Всплывающая буква: высота, наезд на клавишу снизу, наименьшая ширина.
const POP_H: f32 = 62.0;
const POP_OVERLAP: f32 = 6.0;
const POP_MIN_W: f32 = 46.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Off,
    /// На одну клавишу.
    Once,
    /// Caps Lock.
    Lock,
}

/// Состояние клавиатуры (копируется — внутри сигналы).
#[derive(Clone, Copy)]
pub struct Keyboard {
    pub visible: RwSignal<bool>,
    pub page: RwSignal<Page>,
    pub lang: RwSignal<usize>,
    pub shift: RwSignal<Shift>,
    pub ctrl: RwSignal<bool>,
    pub alt: RwSignal<bool>,
    pub sup: RwSignal<bool>,
    /// Показаны ряды Esc/Tab/стрелки и F1–F12.
    pub fn_rows: RwSignal<bool>,
    /// Буква над нажатой клавишей: подпись и прямоугольник клавиши
    /// (x, y, ширина, высота) в координатах поверхности.
    pub popup: RwSignal<Option<(String, [f32; 4])>>,
}

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static REPEAT: Cell<Option<u64>> = const { Cell::new(None) };
}

impl Keyboard {
    pub fn new() -> Self {
        Self {
            visible: use_signal(false),
            page: use_signal(Page::Letters),
            lang: use_signal(0),
            shift: use_signal(Shift::Off),
            ctrl: use_signal(false),
            alt: use_signal(false),
            sup: use_signal(false),
            fn_rows: use_signal(false),
            popup: use_signal(None),
        }
    }

    fn height(&self) -> u32 {
        let mut h = PAD * 2 + KEY_H * 4 + GAP * 3;
        if self.fn_rows.get_untracked() {
            let n = layout::fn_rows().len() as u32;
            h += FN_H * n + GAP * (n - 1) + FN_PAD * 2 + FN_SEP;
        }
        h
    }

    fn spec(&self) -> SurfaceSpec {
        let h = self.height();
        SurfaceSpec {
            namespace: "synkeyboard".into(),
            layer: Layer::Top,
            anchor: Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            size: (0, h + HEADROOM),
            exclusive_zone: h as i32,
            keyboard: KeyboardInteractivity::None,
            ..Default::default()
        }
    }

    /// Где поверхность принимает касания: без запаса [`HEADROOM`] сверху.
    fn input_rect(&self) -> [i32; 4] {
        [0, HEADROOM as i32, 1 << 16, self.height() as i32]
    }

    /// Нажать и отпустить клавишу с учётом залипших модификаторов.
    fn tap(&self, code: u32, with_shift: bool, latin: bool) {
        let group = LANGS[self.lang.get_untracked()].group;
        let switch = latin && group != 0;
        if switch {
            syngui_layer::virtual_keyboard_group(0);
        }
        let mut mods: Vec<Modifier> = Vec::new();
        if with_shift || self.shift.get_untracked() != Shift::Off {
            mods.push(Modifier::Shift);
        }
        if self.ctrl.get_untracked() {
            mods.push(Modifier::Ctrl);
        }
        if self.alt.get_untracked() {
            mods.push(Modifier::Alt);
        }
        if self.sup.get_untracked() {
            mods.push(Modifier::Super);
        }
        tap_with(code, &mods);
        if switch {
            syngui_layer::virtual_keyboard_group(group);
        }
    }

    /// Снять одноразовые модификаторы после клавиши.
    fn release_oneshot(&self) {
        if self.shift.get_untracked() == Shift::Once {
            self.shift.set(Shift::Off);
        }
        if self.ctrl.get_untracked() {
            self.ctrl.set(false);
        }
        if self.alt.get_untracked() {
            self.alt.set(false);
        }
        if self.sup.get_untracked() {
            self.sup.set(false);
        }
    }

    pub fn act(&self, action: Action) {
        match action {
            Action::Key { code, shift, latin, .. } => {
                self.tap(code, shift, latin);
                self.release_oneshot();
            }
            Action::Modifier(Modifier::Shift) => self.shift.set(match self.shift.get_untracked() {
                Shift::Off => Shift::Once,
                _ => Shift::Off,
            }),
            Action::Modifier(Modifier::Ctrl) => self.ctrl.set(!self.ctrl.get_untracked()),
            Action::Modifier(Modifier::Alt) => self.alt.set(!self.alt.get_untracked()),
            Action::Modifier(Modifier::Super) => self.sup.set(!self.sup.get_untracked()),
            Action::Page(p) => self.page.set(p),
            Action::Layout => {
                let next = (self.lang.get_untracked() + 1) % LANGS.len();
                self.lang.set(next);
                syngui_layer::virtual_keyboard_group(LANGS[next].group);
            }
            Action::Fn => self.fn_rows.set(!self.fn_rows.get_untracked()),
            Action::Hide => self.visible.set(false),
            Action::Spacer => {}
        }
    }
}

/// Напечатать текст (символы US-раскладки — в группе 0; буквы — в текущей группе xkb).
pub fn type_text(kb: Keyboard, text: &str) {
    for ch in text.chars() {
        match layout::us_char(ch) {
            Some((code, shift)) => kb.tap(code, shift, !ch.is_ascii_alphabetic()),
            None => log::warn!("нет клавиши для символа {ch:?}"),
        }
    }
}

/// Нажать сочетание вида `ctrl+shift+c`, `f5`, `alt+tab`, `enter`.
pub fn press_combo(kb: Keyboard, combo: &str) -> std::result::Result<(), String> {
    let mut mods: Vec<Modifier> = Vec::new();
    let mut key: Option<u32> = None;
    for part in combo.split('+') {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "ctrl" | "control" => mods.push(Modifier::Ctrl),
            "shift" => mods.push(Modifier::Shift),
            "alt" => mods.push(Modifier::Alt),
            "super" | "meta" | "win" => mods.push(Modifier::Super),
            name => key = Some(layout::key_by_name(name).ok_or_else(|| format!("неизвестная клавиша «{part}»"))?),
        }
    }
    let code = key.ok_or("нет клавиши в сочетании")?;
    tap_with(code, &mods);
    let _ = kb;
    Ok(())
}

/// Клавиша с модификаторами: композитор сообщает клиентам модификаторы только
/// по запросу `modifiers`, поэтому маска идёт до нажатия и снимается после;
/// сами клавиши Shift/Ctrl тоже нажимаются — для программ, следящих за ними.
fn tap_with(code: u32, mods: &[Modifier]) {
    let mask = mods.iter().fold(0, |m, x| m | x.mask());
    if mask != 0 {
        syngui_layer::virtual_keyboard_modifiers(mask, 0, 0);
    }
    for m in mods {
        syngui_layer::virtual_keyboard_key(m.code(), true);
    }
    syngui_layer::virtual_keyboard_key(code, true);
    syngui_layer::virtual_keyboard_key(code, false);
    for m in mods.iter().rev() {
        syngui_layer::virtual_keyboard_key(m.code(), false);
    }
    if mask != 0 {
        syngui_layer::virtual_keyboard_modifiers(0, 0, 0);
    }
}

pub fn install(kb: Keyboard) {
    // Раскладка: все языки в одной keymap, переключение группой.
    let layouts: Vec<&str> = LANGS.iter().map(|l| l.xkb).collect();
    match syngui_layer::xkb_keymap(&layouts.join(","), "", "") {
        Some(keymap) => syngui_layer::virtual_keyboard_keymap(keymap),
        None => log::error!("не удалось собрать xkb keymap для {layouts:?}"),
    }

    // Автопоказ по input-method: приложение открыло/закрыло поле ввода.
    let im = syngui_layer::input_method_active();
    create_effect(move || {
        let active = im.get();
        if kb.visible.get_untracked() != active {
            kb.visible.set(active);
        }
    });

    // Поверхность: создать при показе, закрыть при скрытии, перенастроить
    // высоту при показе/скрытии функциональных рядов.
    create_effect(move || {
        let visible = kb.visible.get();
        let _ = kb.fn_rows.get();
        let current = SURFACE.with(|s| s.get());
        match (visible, current) {
            (true, None) => {
                let id = syngui_layer::create_surface(kb.spec(), move || Box::new(view(kb)));
                syngui_layer::set_input_region(id, Some(vec![kb.input_rect()]));
                SURFACE.with(|s| s.set(Some(id)));
            }
            (true, Some(id)) => {
                syngui_layer::reconfigure_surface(id, kb.spec());
                syngui_layer::set_input_region(id, Some(vec![kb.input_rect()]));
            }
            (false, Some(id)) => {
                SURFACE.with(|s| s.set(None));
                stop_repeat();
                kb.popup.set(None);
                syngui_layer::close_surface(id);
            }
            (false, None) => {}
        }
    });
}

fn stop_repeat() {
    if let Some(t) = REPEAT.with(|r| r.take()) {
        syngui_layer::cancel_timer(t);
    }
}

fn view(kb: Keyboard) -> impl Widget {
    Stack::new()
        .child(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().style("height", StyleValue::px(HEADROOM as f32)))
                .child(keys_view(kb)),
        )
        .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            let Some((label, [x, y, w, _h])) = kb.popup.get() else { return Vec::new() };
            let pw = (w * 1.2).max(POP_MIN_W);
            let width = syngui::viewport::viewport_size().get_untracked().width;
            let left = (x + w / 2.0 - pw / 2.0).clamp(2.0, (width - pw - 2.0).max(2.0));
            let top = (y - POP_H + POP_OVERLAP).max(0.0);
            let bubble = DecoratedBox::new()
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(label).class("key-pop-label"))
                        .class("key-fill"),
                )
                .class("key-pop")
                .style("width", StyleValue::px(pw))
                .style("height", StyleValue::px(POP_H));
            vec![Box::new(Positioned::new(bubble).at(left, top))]
        }))
}

fn keys_view(kb: Keyboard) -> impl Widget {
    DecoratedBox::new()
        .child(Reactive::new(move || {
            let page = kb.page.get();
            let lang = &LANGS[kb.lang.get()];
            let shift = kb.shift.get();
            let (ctrl, alt, sup) = (kb.ctrl.get(), kb.alt.get(), kb.sup.get());
            let fn_rows = kb.fn_rows.get();

            let mut main = Column::new().gap(GAP as f32);
            for row in layout::rows(page, lang) {
                main = main.child(row_widget(kb, row, shift, ctrl, alt, sup, KEY_H as f32));
            }
            let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            if fn_rows {
                // Дополнительные ряды — своим блоком другого фона, с просветом
                // до основной клавиатуры.
                let mut extra = Column::new().gap(GAP as f32);
                for row in layout::fn_rows() {
                    extra = extra.child(row_widget(kb, row, shift, ctrl, alt, sup, FN_H as f32));
                }
                col = col
                    .child(DecoratedBox::new().child(extra).class("keyboard-fn").style("padding", StyleValue::px(FN_PAD as f32)))
                    .child(DecoratedBox::new().style("height", StyleValue::px(FN_SEP as f32)));
            }
            col = col.child(main);
            vec![Box::new(col) as Box<dyn Widget>]
        }))
        .class("keyboard")
}

fn row_widget(kb: Keyboard, keys: Vec<Key>, shift: Shift, ctrl: bool, alt: bool, sup: bool, height: f32) -> impl Widget {
    let mut row = Row::new().gap(GAP as f32).height(height).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for key in keys {
        row = row.child(key_widget(kb, key, shift, ctrl, alt, sup));
    }
    row
}

fn key_widget(kb: Keyboard, key: Key, shift: Shift, ctrl: bool, alt: bool, sup: bool) -> Box<dyn Widget> {
    let upper = shift != Shift::Off;
    let label = match (&key.shifted, upper) {
        (Some(s), true) => s.clone(),
        _ => key.label.clone(),
    };
    // Активные модификаторы подсвечиваются.
    let active = match key.action {
        Action::Modifier(Modifier::Shift) => shift != Shift::Off,
        Action::Modifier(Modifier::Ctrl) => ctrl,
        Action::Modifier(Modifier::Alt) => alt,
        Action::Modifier(Modifier::Super) => sup,
        Action::Fn => kb.fn_rows.get_untracked(),
        _ => false,
    };
    let mut class = key.class.to_string();
    if active {
        class.push_str(" key-active");
    }
    if shift == Shift::Lock && matches!(key.action, Action::Modifier(Modifier::Shift)) {
        class.push_str(" key-lock");
    }
    let width = key.width;
    let action = key.action;
    if action == Action::Spacer {
        return Box::new(DecoratedBox::new().class("key-spacer").style("flex-grow", width));
    }
    let label_shown = label.clone();
    let text = Text::new(label).class("key-label");
    let body = DecoratedBox::new()
        .child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(text)
                .class("key-fill"),
        )
        .class(class);

    let mut gd = GestureDetector::new().child(body);
    match action {
        Action::Key { code, shift: with_shift, repeat: true, latin } => {
            // Автоповтор: первый тап сразу при касании, затем по таймеру до
            // отпускания.
            gd = gd
                .on_press(move |_| {
                    haptics::play(Feedback::Key);
                    stop_repeat();
                    kb.tap(code, with_shift, latin);
                    let t = syngui_layer::add_timer(Duration::from_millis(400), move || {
                        kb.tap(code, with_shift, latin);
                        Some(Duration::from_millis(60))
                    });
                    REPEAT.with(|r| r.set(Some(t)));
                })
                .on_release(move |_| {
                    stop_repeat();
                    kb.release_oneshot();
                });
        }
        Action::Modifier(Modifier::Shift) => {
            gd = gd
                .on_click(move || kb.act(action))
                .on_double_click(move || kb.shift.set(Shift::Lock));
        }
        // Клавиша срабатывает на отпускании над ней — и после долгого
        // удержания (тап после удержания не синтезируется).
        _ => {
            gd = gd.on_release(move |inside| {
                kb.popup.set(None);
                if inside {
                    kb.act(action);
                }
            })
        }
    }
    // Отклик — сразу при касании клавиши (у автоповтора — в его on_press).
    // Буква над пальцем — у символьных клавиш (не у служебных и Fn).
    if !matches!(action, Action::Key { repeat: true, .. }) {
        let popup = (key.class == "key" && matches!(action, Action::Key { .. })).then_some(label_shown);
        gd = gd.on_press_with_bounds(move |_, r| {
            haptics::play(Feedback::Key);
            if let Some(l) = &popup {
                kb.popup.set(Some((l.clone(), [r.origin.x, r.origin.y, r.size.width, r.size.height])));
            }
        });
    }
    Box::new(gd.style("flex-grow", width))
}
