//! syngui на layer-shell поверхностях Wayland — «winit для оболочки».
//!
//! winit не умеет `zwlr_layer_shell_v1`, а панели, обои, меню запуска и
//! уведомления рабочего стола — это именно layer-поверхности. Крейт
//! открывает соединение через smithay-client-toolkit, ведёт цикл calloop и
//! на каждой поверхности держит своё дерево syngui ([`syngui::embed::EmbedView`])
//! и свой [`syngui::gpu::Renderer`] поверх общего wgpu-устройства.
//!
//! Поверхности создаются и меняются из любого места главного потока —
//! в том числе из обработчиков кликов виджетов — через свободные функции
//! ([`create_surface`], [`reconfigure_surface`], [`close_surface`]):
//! команды копятся в очереди и применяются после текущего события.
//!
//! Отладка без экрана: `SYNGUI_LAYER_DUMP=<каталог>` сохраняет каждый
//! нарисованный кадр каждой поверхности в PNG, а
//! `SYNGUI_LAYER_HEADLESS=1600x900` запускает всё без композитора —
//! на одном выдуманном выводе, только в PNG.

mod gpu;
mod keys;
mod state;
mod vkbd;
mod surface;

use std::cell::RefCell;
use std::time::Duration;

pub use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
pub use syngui;

use syngui::input::{Key, Modifiers};
use syngui::signal::RwSignal;
use syngui::widget::Widget;

/// Идентификатор поверхности.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u64);

/// Параметры layer-поверхности.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceSpec {
    /// Пространство имён (`syndesktop-panel`…) — по нему композитор решает,
    /// как обращаться с поверхностью.
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Желаемый размер в логических единицах; 0 по оси — растянуть между
    /// противоположными якорями (или взять размер содержимого при `auto_size`).
    pub size: (u32, u32),
    /// Отступы: сверху, справа, снизу, слева.
    pub margin: [i32; 4],
    pub exclusive_zone: i32,
    pub keyboard: KeyboardInteractivity,
    /// Имя вывода (`eDP-1`); `None` — на усмотрение композитора.
    pub output: Option<String>,
    /// Подгонять размер по осям с нулевым `size` под содержимое.
    pub auto_size: bool,
    /// Цвет очистки кадра (по умолчанию прозрачный).
    pub clear_color: [f32; 4],
}

impl Default for SurfaceSpec {
    fn default() -> Self {
        Self {
            namespace: "syngui-layer".into(),
            layer: Layer::Top,
            anchor: Anchor::empty(),
            size: (0, 0),
            margin: [0; 4],
            exclusive_zone: 0,
            keyboard: KeyboardInteractivity::None,
            output: None,
            auto_size: false,
            clear_color: [0.0; 4],
        }
    }
}

/// Вывод (монитор), как его видит клиент.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    /// Логическое положение и размер.
    pub position: (i32, i32),
    pub size: (i32, i32),
    pub scale: i32,
}

/// Нажатие клавиши, как его видит перехватчик [`SurfaceHooks::on_key`].
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub key: Key,
    /// Имя keysym по xkb (`Escape`, `Return`, `a`).
    pub keysym: String,
    pub text: Option<String>,
    pub pressed: bool,
    pub modifiers: Modifiers,
}

/// Обработчики событий поверхности вне дерева виджетов.
#[derive(Default)]
pub struct SurfaceHooks {
    /// Клавиша до дерева; `true` — съедена.
    pub on_key: Option<Box<dyn FnMut(&KeyInfo) -> bool>>,
    /// Указатель вошёл (`true`) / ушёл (`false`) — автоскрытие панелей.
    pub on_pointer: Option<Box<dyn FnMut(bool)>>,
    /// Композитор закрыл поверхность (исчез вывод и т.п.).
    pub on_closed: Option<Box<dyn FnOnce()>>,
    /// Поверхность получила размер (логический) после configure.
    pub on_resize: Option<Box<dyn FnMut(u32, u32)>>,
}

type Factory = Box<dyn FnOnce() -> Box<dyn Widget>>;

pub(crate) enum Command {
    Create { id: SurfaceId, spec: SurfaceSpec, factory: Factory, hooks: SurfaceHooks },
    Reconfigure { id: SurfaceId, spec: SurfaceSpec },
    InputRegion { id: SurfaceId, rects: Option<Vec<[i32; 4]>> },
    Close { id: SurfaceId },
    /// Новая таблица стилей; второе поле — длительность перетекания цветов
    /// (мс), `None` — сменить скачком.
    Stylesheet(String, Option<u32>),
    Timer { id: u64, after: Duration, f: Box<dyn FnMut() -> Option<Duration>> },
    CancelTimer(u64),
    Redraw(Option<SurfaceId>),
    Lock(LockFactory),
    Unlock,
    Quit,
    /// Виртуальная клавиатура: раскладка XKB (текст keymap).
    VkKeymap(String),
    /// Виртуальная клавиатура: клавиша (evdev-код) нажата/отпущена.
    VkKey(u32, bool),
    /// Виртуальная клавиатура: группа раскладки (0 — первая в keymap).
    VkGroup(u32),
    VkModifiers(u32, u32, u32),
    /// Подписаться на активность полей ввода (zwp_input_method_v2).
    ImEnable,
}

/// Виджет экрана блокировки для вывода.
pub(crate) type LockFactory = std::rc::Rc<dyn Fn(&OutputInfo) -> Box<dyn Widget>>;

thread_local! {
    static COMMANDS: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    static OUTPUTS: RefCell<Option<RwSignal<Vec<OutputInfo>>>> = const { RefCell::new(None) };
    static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    static LOCKED: RefCell<Option<RwSignal<bool>>> = const { RefCell::new(None) };
}

fn push(cmd: Command) {
    COMMANDS.with(|c| c.borrow_mut().push(cmd));
    WAKE.with(|w| {
        if let Some(f) = w.borrow().as_ref() {
            f();
        }
    });
}

pub(crate) fn take_commands() -> Vec<Command> {
    COMMANDS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

pub(crate) fn set_wake(f: Box<dyn Fn()>) {
    WAKE.with(|w| *w.borrow_mut() = Some(f));
}

fn next_id() -> u64 {
    NEXT_ID.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    })
}

/// Создать поверхность. Виджет строится при первом configure.
pub fn create_surface(spec: SurfaceSpec, factory: impl FnOnce() -> Box<dyn Widget> + 'static) -> SurfaceId {
    create_surface_with(spec, SurfaceHooks::default(), factory)
}

/// Создать поверхность с обработчиками.
pub fn create_surface_with(
    spec: SurfaceSpec,
    hooks: SurfaceHooks,
    factory: impl FnOnce() -> Box<dyn Widget> + 'static,
) -> SurfaceId {
    let id = SurfaceId(next_id());
    push(Command::Create { id, spec, factory: Box::new(factory), hooks });
    id
}

/// Сменить параметры (якоря, размер, отступы, слой, клавиатуру).
/// Смена вывода пересоздаёт поверхность с тем же деревом.
pub fn reconfigure_surface(id: SurfaceId, spec: SurfaceSpec) {
    push(Command::Reconfigure { id, spec });
}

/// Область ввода поверхности (логические [x, y, w, h]): вне её указатель
/// проходит насквозь к окнам под поверхностью. `None` — вся поверхность.
/// Док с запасом места над значками под увеличение ловит клики только
/// своей полосой.
pub fn set_input_region(id: SurfaceId, rects: Option<Vec<[i32; 4]>>) {
    push(Command::InputRegion { id, rects });
}

pub fn close_surface(id: SurfaceId) {
    push(Command::Close { id });
}

/// Заменить таблицу стилей всех поверхностей (MSS-текст).
pub fn set_stylesheet(mss: impl Into<String>) {
    push(Command::Stylesheet(mss.into(), None));
}

/// Заменить таблицу стилей так, чтобы изменившиеся цвета перетекли за
/// `transition_ms` (плавная смена темы), а не сменились скачком.
pub fn set_stylesheet_with_transition(mss: impl Into<String>, transition_ms: u32) {
    push(Command::Stylesheet(mss.into(), Some(transition_ms)));
}

/// Перерисовать поверхность (или все) — для состояния вне сигналов.
pub fn request_redraw(id: Option<SurfaceId>) {
    push(Command::Redraw(id));
}

/// Таймер на главном потоке: `f` возвращает `Some(через)` для повтора.
pub fn add_timer(after: Duration, f: impl FnMut() -> Option<Duration> + 'static) -> u64 {
    let id = next_id();
    push(Command::Timer { id, after, f: Box::new(f) });
    id
}

pub fn cancel_timer(id: u64) {
    push(Command::CancelTimer(id));
}

/// Завершить цикл.
pub fn quit() {
    push(Command::Quit);
}

thread_local! {
    static IM_ACTIVE: RefCell<Option<RwSignal<bool>>> = const { RefCell::new(None) };
}

/// Виртуальная клавиатура (`zwp_virtual_keyboard_v1`): задать раскладку XKB
/// (текст keymap, см. [`xkb_keymap`]). Композитор выставит её всем клиентам.
pub fn virtual_keyboard_keymap(keymap: String) {
    push(Command::VkKeymap(keymap));
}

/// Виртуальная клавиатура: нажать/отпустить клавишу по evdev-коду (`KEY_A` = 30).
/// Модификаторы — тоже клавиши (`KEY_LEFTCTRL` = 29): композитор сам считает
/// состояние xkb, поэтому `Ctrl+C` — это нажатие 29, 46, отпускание 46, 29.
pub fn virtual_keyboard_key(evdev_code: u32, pressed: bool) {
    push(Command::VkKey(evdev_code, pressed));
}

/// Виртуальная клавиатура: переключить группу раскладки keymap (`us,ru` → 0/1).
/// Состояние модификаторов (маски xkb: Shift=1, Lock=2, Control=4, Mod1/Alt=8,
/// Mod4/Super=64). Композитор только по этому запросу сообщает клиентам
/// модификаторы — нажатия самих клавиш Shift/Ctrl их не меняют.
pub fn virtual_keyboard_modifiers(depressed: u32, latched: u32, locked: u32) {
    push(Command::VkModifiers(depressed, latched, locked));
}

pub fn virtual_keyboard_group(group: u32) {
    push(Command::VkGroup(group));
}

/// Активность поля ввода: `true`, когда окно с фокусом включило text-input
/// (композитор передаёт это через `zwp_input_method_v2`). Первый вызов
/// подписывается; без поддержки протокола сигнал остаётся `false`.
pub fn input_method_active() -> RwSignal<bool> {
    let sig = IM_ACTIVE.with(|o| *o.borrow_mut().get_or_insert_with(|| syngui::signal::use_signal(false)));
    push(Command::ImEnable);
    sig
}

pub(crate) fn set_input_method_active(active: bool) {
    if let Some(sig) = IM_ACTIVE.with(|o| *o.borrow()) {
        if sig.get_untracked() != active {
            sig.set(active);
        }
    }
}

/// Текст XKB-keymap для раскладок вида `"us,ru"` (варианты и опции — как в
/// setxkbmap). Нужны файлы xkeyboard-config (`/usr/share/X11/xkb`).
pub fn xkb_keymap(layouts: &str, variants: &str, options: &str) -> Option<String> {
    let ctx = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
    let opts = if options.is_empty() { None } else { Some(options.to_string()) };
    let keymap = xkbcommon::xkb::Keymap::new_from_names(&ctx, "", "", layouts, variants, opts, xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS)?;
    Some(keymap.get_as_string(xkbcommon::xkb::KEYMAP_FORMAT_TEXT_V1))
}

/// Выводы — сигнал: подписанный код пересоздаёт поверхности по мониторам.
pub fn outputs() -> RwSignal<Vec<OutputInfo>> {
    OUTPUTS.with(|o| *o.borrow_mut().get_or_insert_with(|| syngui::signal::use_signal(Vec::new())))
}

/// Заблокировать сеанс (ext-session-lock): композитор перестаёт показывать
/// окна, на каждом выводе — поверхность из `factory`. Снять — [`unlock_session`]
/// после проверки пароля. Без поддержки протокола [`session_locked`] так и
/// останется `false`.
pub fn lock_session(factory: impl Fn(&OutputInfo) -> Box<dyn Widget> + 'static) {
    push(Command::Lock(std::rc::Rc::new(factory)));
}

pub fn unlock_session() {
    push(Command::Unlock);
}

/// Сеанс заблокирован (композитор подтвердил блокировку).
pub fn session_locked() -> RwSignal<bool> {
    LOCKED.with(|o| *o.borrow_mut().get_or_insert_with(|| syngui::signal::use_signal(false)))
}

/// Параметры запуска.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Встроенный шрифт иконок (Material Icons подключается всегда).
    pub font_family: Option<String>,
}

/// Запустить цикл. `init` вызывается, когда всё готово (выводы уже в
/// [`outputs`]); в нём создают первые поверхности.
pub fn run(options: RunOptions, stylesheet: &str, init: impl FnOnce() + 'static) -> anyhow::Result<()> {
    state::run(options, stylesheet, Box::new(init))
}

pub(crate) fn next_id_pub() -> u64 {
    next_id()
}
