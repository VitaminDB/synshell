//! Виджет клавиатуры и её состояние. Поверхность создаётся при показе и
//! закрывается при скрытии (как домашний экран synmobile-shell).
//!
//! Два положения: **внизу** — полоса во всю ширину, высота из числа рядов, exclusive zone той же
//! высоты отодвигает окна; **плавающая** — поверхность Overlay во весь экран (поверх полноэкранной
//! игры и контроллера), область ввода — только сама клавиатура (окно поменьше с ручкой сверху:
//! таскается пальцем, кнопки «вниз» и «спрятать»), окна она не трогает. Клавиша ⧉ в нижнем ряду
//! переключает одно в другое; `[osk] placement` — что выбирать самой.
//!
//! «Захват» (`show … for <сокет>`): клавиши идут не композитору через виртуальную клавиатуру,
//! а программе по её сокету строками `text <символы>` и `key <имя>` — поле контроллера над игрой
//! получает текст, не отбирая у игры фокус клавиатуры (игра во весь экран от потери фокуса
//! сворачивается).

use synshell_common::config::OskPlacement;
use synshell_common::haptics::{self, Feedback};
use std::cell::Cell;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use syngui::core::Point;
use syngui::prelude::*;
use syngui::StyleValue;
use syngui::widget::WidgetExt;
use syngui::GestureDetector;
use syngui::containers::{MultiTouch, Positioned, TouchPhase, TouchPoint};
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
/// Плавающая: ручка сверху (за неё таскать; справа — кнопки «вниз» и «спрятать»), отступ от краёв
/// экрана, доля ширины экрана и её пределы.
const GRIP_H: u32 = 30;
const GRIP_BTN_W: f32 = 44.0;
const FLOAT_MARGIN: f32 = 12.0;
const FLOAT_SHARE: f32 = 0.62;
const FLOAT_MIN_W: f32 = 360.0;
const FLOAT_MAX_W: f32 = 760.0;
/// Material Icons: вниз (vertical_align_bottom), спрятать (keyboard_hide).
const ICON_DOCK: &str = "\u{E258}";
const ICON_HIDE: &str = "\u{E31A}";

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
    /// Выезжать самой на текстовом поле (режим ввода приложения не «Выключен», `[osk] apps`).
    pub auto: RwSignal<bool>,
    /// Плавающая (иначе внизу).
    pub floating: RwSignal<bool>,
    /// Левый верхний угол плавающей (единицы интерфейса); `(-1, -1)` — ещё не ставилась (внизу по центру).
    pub pos: RwSignal<Point>,
    /// Захват: имя сокета в `$XDG_RUNTIME_DIR`, куда идут клавиши вместо виртуальной клавиатуры.
    pub capture: RwSignal<Option<String>>,
    /// `[osk] placement`.
    pub placement: RwSignal<OskPlacement>,
    /// Что выбрал пользователь клавишей ⧉ (при `placement = "auto"` — до перезапуска).
    pub chosen: RwSignal<Option<bool>>,
}

thread_local! {
    /// Поверхность и её вид (плавающая?) — смена вида пересоздаёт поверхность (другой слой).
    static SURFACE: Cell<Option<(SurfaceId, bool)>> = const { Cell::new(None) };
    static REPEAT: Cell<Option<u64>> = const { Cell::new(None) };
    /// Перетаскивание за ручку: палец и его сдвиг от угла клавиатуры.
    static DRAG: Cell<Option<(u64, f32, f32)>> = const { Cell::new(None) };
}

impl Keyboard {
    pub fn new(placement: OskPlacement) -> Self {
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
            auto: use_signal(true),
            floating: use_signal(false),
            pos: use_signal(Point::new(-1.0, -1.0)),
            capture: use_signal(None),
            placement: use_signal(placement),
            chosen: use_signal(None),
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
        if self.floating.get_untracked() {
            // Во весь экран поверх всего (как syngamepad); где клавиатура — область ввода.
            return SurfaceSpec {
                namespace: "synkeyboard".into(),
                layer: Layer::Overlay,
                anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                size: (0, 0),
                exclusive_zone: -1,
                keyboard: KeyboardInteractivity::None,
                ..Default::default()
            };
        }
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

    /// Ширина плавающей по ширине экрана.
    fn float_width(&self) -> f32 {
        let vw = syngui::viewport::viewport_size().get_untracked().width;
        (vw * FLOAT_SHARE).clamp(FLOAT_MIN_W, FLOAT_MAX_W).min((vw - 2.0 * FLOAT_MARGIN).max(100.0))
    }

    /// Прямоугольник плавающей клавиатуры с ручкой (единицы интерфейса).
    fn float_rect(&self) -> [f32; 4] {
        let p = self.pos.get_untracked();
        [p.x, p.y, self.float_width(), (GRIP_H + self.height()) as f32]
    }

    /// Поставить плавающую на экран: впервые — внизу по центру, иначе — не дальше краёв
    /// (экран повернули, ряды Fn раскрыли).
    fn place(&self) {
        let vp = syngui::viewport::viewport_size().get_untracked();
        let [x, y, w, h] = self.float_rect();
        let (max_x, max_y) = ((vp.width - w - FLOAT_MARGIN).max(FLOAT_MARGIN), (vp.height - h - FLOAT_MARGIN).max(FLOAT_MARGIN));
        let (nx, ny) = if x < 0.0 && y < 0.0 {
            (((vp.width - w) / 2.0).max(FLOAT_MARGIN), max_y)
        } else {
            (x.clamp(FLOAT_MARGIN, max_x), y.clamp(FLOAT_MARGIN, max_y))
        };
        if (nx - x).abs() > 0.5 || (ny - y).abs() > 0.5 {
            self.pos.set(Point::new(nx, ny));
        }
    }

    /// Где поверхность принимает касания: внизу — без запаса [`HEADROOM`] сверху; плавающая — она сама.
    fn input_rect(&self) -> [i32; 4] {
        if self.floating.get_untracked() {
            let [x, y, w, h] = self.float_rect();
            return [x.floor() as i32, y.floor() as i32, w.ceil() as i32, h.ceil() as i32];
        }
        [0, HEADROOM as i32, 1 << 16, self.height() as i32]
    }

    /// Показать: `float` — как просили (`None` — по `[osk] placement`), `capture` — кому идут клавиши.
    pub fn show(&self, float: Option<bool>, capture: Option<String>) {
        let floating = float.unwrap_or_else(|| self.auto_floating(capture.is_some()));
        self.capture.set(capture);
        self.floating.set(floating);
        self.visible.set(true);
    }

    pub fn hide(&self) {
        self.visible.set(false);
        self.capture.set(None);
    }

    /// Плавающая ли, когда не сказали явно.
    fn auto_floating(&self, for_capture: bool) -> bool {
        match self.placement.get_untracked() {
            OskPlacement::Floating => true,
            OskPlacement::Docked => false,
            // Поле контроллера над полноэкранной игрой: внизу клавиатура легла бы под игру
            // (Top-слои при полноэкранном окне касаний не получают) и отодвинула бы окна.
            OskPlacement::Auto => for_capture || self.chosen.get_untracked().unwrap_or_else(free_desktop),
        }
    }

    pub fn set_floating(&self, floating: bool) {
        self.chosen.set(Some(floating));
        if self.floating.get_untracked() != floating {
            self.floating.set(floating);
        }
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

    /// Клавиша с клавиатуры: при захвате — программе по сокету (`text` — её символ, как подписан,
    /// иначе имя клавиши), без него — композитору.
    fn press(&self, code: u32, with_shift: bool, latin: bool, text: Option<&str>) {
        if let Some(sock) = self.capture.get_untracked() {
            let line = match text {
                Some(t) if !t.is_empty() => format!("text {t}"),
                _ => match layout::key_name(code) {
                    Some(name) => format!("key {name}"),
                    None => return,
                },
            };
            capture_send(&sock, line);
            return;
        }
        self.tap(code, with_shift, latin);
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
                self.press(code, shift, latin, None);
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
            Action::Hide => self.hide(),
            Action::Float => self.set_floating(!self.floating.get_untracked()),
            Action::Spacer => {}
        }
    }
}

/// Свободный ли стол у композитора (`[mobile] mode = "free"`): там клавиатура плавающая.
fn free_desktop() -> bool {
    use synshell_common::action::MobileMode;
    use synshell_common::ipc::{Client, Request, Response};
    match Client::connect().and_then(|mut c| c.request(&Request::Mobile)) {
        Ok(Response::Mobile { mobile }) => mobile.mode == MobileMode::Free,
        _ => false,
    }
}

/// Строка программе-получателю клавиш: сокет `$XDG_RUNTIME_DIR/<имя>` (только имя файла), в своём
/// потоке — главный не ждёт ответа.
fn capture_send(sock: &str, line: String) {
    if sock.is_empty() || sock.contains('/') {
        return;
    }
    let path = synshell_common::paths::runtime_dir().join(sock);
    std::thread::spawn(move || match UnixStream::connect(&path) {
        Ok(mut s) => {
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            if writeln!(s, "{line}").is_ok() {
                let mut reply = String::new();
                let _ = BufReader::new(s).read_line(&mut reply);
            }
        }
        Err(e) => log::warn!("захват клавиш: {}: {e}", path.display()),
    });
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
            name => key = Some(layout::key_by_name(name).ok_or_else(|| t!("неизвестная клавиша «{part}»", part = part))?),
        }
    }
    let code = key.ok_or_else(|| t!("нет клавиши в сочетании"))?;
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
        let active = im.get() && kb.auto.get();
        if kb.visible.get_untracked() != active {
            if active {
                kb.show(None, None);
            } else {
                kb.hide();
            }
        }
    });

    // Поверхность: создать при показе, закрыть при скрытии, перенастроить
    // высоту при показе/скрытии функциональных рядов; плавающая ↔ внизу — другой слой, пересоздать.
    create_effect(move || {
        let visible = kb.visible.get();
        let floating = kb.floating.get();
        let _ = kb.fn_rows.get();
        let current = SURFACE.with(|s| s.get());
        match (visible, current) {
            (true, Some((id, f))) if f == floating => {
                syngui_layer::reconfigure_surface(id, kb.spec());
                syngui_layer::set_input_region(id, Some(vec![kb.input_rect()]));
            }
            (true, old) => {
                if let Some((id, _)) = old {
                    stop_repeat();
                    kb.popup.set(None);
                    syngui_layer::close_surface(id);
                }
                let id = syngui_layer::create_surface(kb.spec(), move || view(kb));
                syngui_layer::set_input_region(id, Some(vec![kb.input_rect()]));
                SURFACE.with(|s| s.set(Some((id, floating))));
            }
            (false, Some((id, _))) => {
                SURFACE.with(|s| s.set(None));
                stop_repeat();
                kb.popup.set(None);
                DRAG.with(|d| d.set(None));
                syngui_layer::close_surface(id);
            }
            (false, None) => {}
        }
    });

    // Плавающую подвинули или её поверхность разметилась/экран сменился — поставить (впервые — внизу по
    // центру, иначе в границах экрана) и область ввода за ней. Размер экрана — сигнал viewport: пока
    // плавающая поверхность не разметилась, в нём размер прежней (полосы внизу) — ставить только по её поверхности.
    create_effect(move || {
        let _ = kb.pos.get();
        let _ = syngui::viewport::viewport_size().get();
        if !kb.visible.get_untracked() || !kb.floating.get_untracked() {
            return;
        }
        if let Some((id, true)) = SURFACE.with(|s| s.get()) {
            kb.place();
            syngui_layer::set_input_region(id, Some(vec![kb.input_rect()]));
        }
    });
}

fn stop_repeat() {
    if let Some(t) = REPEAT.with(|r| r.take()) {
        syngui_layer::cancel_timer(t);
    }
}

fn view(kb: Keyboard) -> Box<dyn Widget> {
    if kb.floating.get_untracked() {
        return Box::new(floating_view(kb));
    }
    Box::new(
        Stack::new()
            .child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(DecoratedBox::new().style("height", StyleValue::px(HEADROOM as f32)))
                    .child(keys_view(kb)),
            )
            .child(popup_view(kb)),
    )
}

/// Плавающая: на весь экран — ручка и клавиши в своём месте, всплывающая буква над ними. Касания
/// ручки берёт [`MultiTouch`] (координаты поверхности не плывут, когда клавиатура едет под пальцем),
/// остальное идёт клавишам и кнопкам.
fn floating_view(kb: Keyboard) -> impl Widget {
    MultiTouch::new()
        .on_touch(move |t| grip_touch(kb, t))
        .child(
            Stack::new()
                .child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
                    let _ = kb.fn_rows.get();
                    let _ = syngui::viewport::viewport_size().get();
                    let [_, _, w, h] = kb.float_rect();
                    // Высота — явно: иначе `.keyboard` (flex-grow) растянулся бы до низа экрана.
                    let body = Column::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(grip_view(kb))
                        .child(keys_view(kb))
                        .class("keyboard-float")
                        .style("width", StyleValue::px(w))
                        .style("height", StyleValue::px(h));
                    vec![Box::new(Positioned::new(body).offset_signal(kb.pos))]
                }))
                .child(popup_view(kb)),
        )
}

/// Ручка плавающей: полоска по центру (тащить), справа — «вниз» и «спрятать».
fn grip_view(kb: Keyboard) -> impl Widget {
    let button = |glyph: &str, f: Box<dyn Fn() + Send + Sync>| {
        GestureDetector::new()
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(glyph.to_string()).class("keyboard-grip-icon"))
                    .class("keyboard-grip-btn"),
            )
            .on_click(move || f())
            .style("width", StyleValue::px(GRIP_BTN_W))
    };
    Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .height(GRIP_H as f32)
        .child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("keyboard-grip-pill"))
                .style("flex-grow", 1.0_f32),
        )
        .child(button(ICON_DOCK, Box::new(move || kb.set_floating(false))))
        .child(button(ICON_HIDE, Box::new(move || kb.hide())))
        .class("keyboard-grip")
}

/// Палец на полоске ручки — тащить клавиатуру; кнопки справа и клавиши — не наше.
fn grip_touch(kb: Keyboard, t: TouchPoint) -> bool {
    match t.phase {
        TouchPhase::Down => {
            if DRAG.with(|d| d.get()).is_some() {
                return false;
            }
            let [x, y, w, _] = kb.float_rect();
            let on_grip = t.position.x >= x && t.position.x < x + w - 2.0 * GRIP_BTN_W && t.position.y >= y && t.position.y < y + GRIP_H as f32;
            if !on_grip {
                return false;
            }
            haptics::play(Feedback::Key);
            DRAG.with(|d| d.set(Some((t.id, t.position.x - x, t.position.y - y))));
            true
        }
        TouchPhase::Move => {
            let Some((id, dx, dy)) = DRAG.with(|d| d.get()) else { return false };
            if id != t.id {
                return false;
            }
            let vp = syngui::viewport::viewport_size().get_untracked();
            let [_, _, w, h] = kb.float_rect();
            let nx = (t.position.x - dx).clamp(FLOAT_MARGIN, (vp.width - w - FLOAT_MARGIN).max(FLOAT_MARGIN));
            let ny = (t.position.y - dy).clamp(FLOAT_MARGIN, (vp.height - h - FLOAT_MARGIN).max(FLOAT_MARGIN));
            kb.pos.set(Point::new(nx, ny));
            true
        }
        TouchPhase::Up | TouchPhase::Cancel => {
            if DRAG.with(|d| d.get()).is_some_and(|(id, ..)| id == t.id) {
                DRAG.with(|d| d.set(None));
                return true;
            }
            false
        }
    }
}

fn popup_view(kb: Keyboard) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
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
    })
}

fn keys_view(kb: Keyboard) -> impl Widget {
    DecoratedBox::new()
        .child(Reactive::new(move || {
            let page = kb.page.get();
            let lang = &LANGS[kb.lang.get()];
            let shift = kb.shift.get();
            let (ctrl, alt, sup) = (kb.ctrl.get(), kb.alt.get(), kb.sup.get());
            let fn_rows = kb.fn_rows.get();
            let floating = kb.floating.get();

            let mut main = Column::new().gap(GAP as f32);
            for row in layout::rows(page, lang) {
                main = main.child(row_widget(kb, row, shift, ctrl, alt, sup, floating, KEY_H as f32));
            }
            let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            if fn_rows {
                // Дополнительные ряды — своим блоком другого фона, с просветом
                // до основной клавиатуры.
                let mut extra = Column::new().gap(GAP as f32);
                for row in layout::fn_rows() {
                    extra = extra.child(row_widget(kb, row, shift, ctrl, alt, sup, floating, FN_H as f32));
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

#[allow(clippy::too_many_arguments)]
fn row_widget(kb: Keyboard, keys: Vec<Key>, shift: Shift, ctrl: bool, alt: bool, sup: bool, floating: bool, height: f32) -> impl Widget {
    let mut row = Row::new().gap(GAP as f32).height(height).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for key in keys {
        row = row.child(key_widget(kb, key, shift, ctrl, alt, sup, floating));
    }
    row
}

fn key_widget(kb: Keyboard, key: Key, shift: Shift, ctrl: bool, alt: bool, sup: bool, floating: bool) -> Box<dyn Widget> {
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
        Action::Float => floating,
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
    // Значок (Material Icons, частная область Юникода) — виджетом значка, не текстом.
    let icon = !label.is_empty() && label.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c));
    let text: Box<dyn Widget> = if icon { Box::new(Icon::new(label).class("key-icon")) } else { Box::new(Text::new(label).class("key-label")) };
    let body = DecoratedBox::new()
        .child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(text)
                .class("key-fill"),
        )
        .class(class);

    // Что печатает клавиша при захвате: символьная — свою подпись (с Shift — заглавную).
    let typed = (key.class == "key" && matches!(action, Action::Key { .. })).then(|| label_shown.clone());
    let mut gd = GestureDetector::new().child(body);
    match action {
        Action::Key { code, shift: with_shift, repeat: true, latin } => {
            // Автоповтор: первый тап сразу при касании, затем по таймеру до
            // отпускания.
            gd = gd
                .on_press(move |_| {
                    haptics::play(Feedback::Key);
                    stop_repeat();
                    kb.press(code, with_shift, latin, None);
                    let t = syngui_layer::add_timer(Duration::from_millis(400), move || {
                        kb.press(code, with_shift, latin, None);
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
        Action::Key { code, shift: with_shift, latin, .. } => {
            gd = gd.on_release(move |inside| {
                kb.popup.set(None);
                if inside {
                    kb.press(code, with_shift, latin, typed.as_deref());
                    kb.release_oneshot();
                }
            })
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
