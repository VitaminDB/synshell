//! Шторки телефона — две, как в iOS и HyperOS: свайп сверху вниз в левой
//! половине экрана открывает уведомления, в правой — быстрые настройки
//! (`[gestures] edge_top` / `edge_top_right`, стороны меняются в «Параметры →
//! Жесты»). Свайп вниз по рабочему столу — та же шторка, что у края над
//! пальцем.
//!
//! Быстрые настройки: время, дата, параметры и питание; плитки; яркость
//! (проценты и ниты, если в `[[output]] max_nits` задан максимум панели) и
//! громкость; открытые приложения. Уведомления: лента на весь экран с
//! прокруткой, карточку смахивают вбок (`[notifications] swipe_dismiss`).
//! Свайп с другой стороны при открытой шторке переключает на другую.
//! Закрывается свайпом вверх, тапом по затемнению, жестом «назад».

use std::cell::Cell;
use std::sync::{Arc, Mutex};
use synshell_common::action::MobileMode;
use synshell_common::ipc::WindowOp;
use synshell_common::Action;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::{Motion, PanAxis, Presence, SwipeDirection, TransformOrigin};
use syngui::GestureDetector;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi, rx};

/// Какая шторка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Notifications,
    QuickSettings,
}

thread_local! {
    static SURFACE: Cell<Option<SurfaceId>> = const { Cell::new(None) };
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    static PANEL: Cell<Option<RwSignal<Panel>>> = const { Cell::new(None) };
    /// Открытый список вариантов плитки (удержание плитки).
    static MENU: Cell<Option<RwSignal<Option<TileMenu>>>> = const { Cell::new(None) };
}

pub fn is_open() -> bool {
    OPEN.with(|o| o.get()).is_some_and(|s| s.get_untracked())
}

/// Открытая шторка.
pub fn current() -> Option<Panel> {
    if !is_open() {
        return None;
    }
    PANEL.with(|p| p.get()).map(|p| p.get_untracked())
}

/// Открыть или закрыть быстрые настройки (`shell shade`); открыты
/// уведомления — переключить на настройки.
pub fn toggle() {
    match current() {
        Some(Panel::QuickSettings) => close(),
        _ => show(Panel::QuickSettings),
    }
}

/// Показать шторку; открыта другая — переключить на эту.
pub fn show(panel: Panel) {
    crate::notifications::hide_popups(ShellCtx::get());
    match PANEL.with(|p| p.get()) {
        Some(sig) if is_open() => {
            if sig.get_untracked() != panel {
                sig.set(panel);
            }
        }
        _ => open(panel),
    }
}

pub fn close() {
    let ctx = ShellCtx::get();
    match OPEN.with(|o| o.get()) {
        // Уход с анимацией; поверхность закроется по её концу.
        Some(s) if crate::anim::group_ms(&ctx, "shade", 100) > 0 => s.set(false),
        _ => close_now(),
    }
}

fn close_now() {
    if let Some(id) = SURFACE.with(|s| s.take()) {
        syngui_layer::close_surface(id);
    }
    OPEN.with(|o| o.set(None));
    PANEL.with(|p| p.set(None));
    MENU.with(|m| m.set(None));
}

fn open(panel: Panel) {
    if SURFACE.with(|s| s.get()).is_some() {
        // Ещё уезжает — поверхность та же, вернуть.
        if let (Some(o), Some(p)) = (OPEN.with(|o| o.get()), PANEL.with(|p| p.get())) {
            p.set(panel);
            o.set(true);
        }
        return;
    }
    let ctx = ShellCtx::get();
    ctx.close_popup();
    // Экранная клавиатура поверх ленты/шторки не нужна.
    crate::actions::spawn("synkeyboard hide");
    let open = use_signal(true);
    let which = use_signal(panel);
    OPEN.with(|o| o.set(Some(open)));
    PANEL.with(|p| p.set(Some(which)));
    let spec = SurfaceSpec {
        namespace: "syndesktop-shade".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        margin: [0; 4],
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::OnDemand,
        output: crate::manager::focused_output(&ctx),
        auto_size: false,
        clear_color: [0.0; 4],
    };
    let id = syngui_layer::create_surface(spec, move || Box::new(view(ShellCtx::get(), open, which)));
    SURFACE.with(|s| s.set(Some(id)));
}

/// Свайп вниз по рабочему столу в точке `x` (лог. px): то же действие, что у
/// верхнего края над пальцем.
pub fn swipe_down_at(x: f32) {
    let ctx = ShellCtx::get();
    let (w, _) = crate::manager::output_size(None);
    let g = &ctx.cfg().gestures;
    let action = if x < w as f32 / 2.0 { g.edge_top.clone() } else { g.edge_top_right.clone() };
    crate::actions::run(action);
}

fn view(ctx: ShellCtx, open: RwSignal<bool>, which: RwSignal<Panel>) -> impl Widget {
    let dur = crate::anim::group_ms(&ctx, "shade", 320);
    let scrim_dur = dur;
    let scrim = Presence::signal(open, || {
        Box::new(
            GestureDetector::new()
                .on_click(close)
                .child(DecoratedBox::new().class("shade-scrim")),
        )
    })
    .enter(Motion::fade())
    .exit(Motion::fade())
    .duration_ms(scrim_dur)
    .initial(scrim_dur > 0);
    let panel = Presence::signal(open, move || {
        // Смена шторки — пересборка содержимого; поверхность и затемнение те же.
        Box::new(rx(move || -> Box<dyn Widget> {
            let ctx = ShellCtx::get();
            let closer = || GestureDetector::new().pan_axis(PanAxis::Vertical).on_swipe(|dir, _| {
                if dir == SwipeDirection::Up {
                    close();
                }
            });
            // поворот экрана — перестроить под новую сторону
            let landscape = crate::manager::is_landscape(&ctx);
            match which.get() {
                // Горизонтально — во всю высоту двумя колонками с прокруткой: иначе плитки уходили за низ экрана.
                Panel::QuickSettings if landscape => Box::new(closer().child(quick_settings_wide(ctx))),
                Panel::QuickSettings => Box::new(
                    Column::new().main_axis_alignment(MainAxisAlignment::Start).child(closer().child(quick_settings(ctx))),
                ),
                // Во всю высоту: лента прокручивается внутри.
                Panel::Notifications => Box::new(closer().child(notifications(ctx, landscape))),
            }
        }))
    })
    .enter(Motion::fade().slide(0.0, -80.0))
    .exit(Motion::fade().slide(0.0, -60.0))
    .origin(TransformOrigin::Custom(0.5, 0.0))
    .easing(syngui::animation::Easing::EMPHASIZED_DECELERATE)
    .exit_easing(syngui::animation::Easing::EMPHASIZED_ACCELERATE)
    .duration_ms(dur)
    .exit_duration_ms(dur * 2 / 3)
    .initial(dur > 0)
    .on_exit_complete(close_now);
    let menu = use_signal(None::<TileMenu>);
    MENU.with(|m| m.set(Some(menu)));
    // смена шторки или её уход — список вариантов закрыт
    create_effect(move || {
        let _ = which.get();
        if !open.get() {
            menu.set(None);
        }
    });
    Stack::new().fit(StackFit::Expand).child(scrim).child(panel).child(tile_menu_layer(ctx, menu))
}

fn handle() -> impl Widget {
    Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("shade-handle"))
}

fn quick_settings(ctx: ShellCtx) -> impl Widget {
    Column::new()
        .gap(14.0)
        .child(header(ctx))
        .child(crate::link::shade_card(ctx))
        .child(tiles(ctx))
        .child(sliders(ctx))
        .child(open_apps(ctx))
        .child(notifications_link(ctx))
        .child(handle())
        .class("shade")
}

/// Быстрые настройки горизонтально: слева время, связь, ползунки, приложения;
/// справа плитки. Колонки прокручиваются каждая сама.
fn quick_settings_wide(ctx: ShellCtx) -> impl Widget {
    let left = Column::new()
        .gap(12.0)
        .child(header(ctx))
        .child(crate::link::shade_card(ctx))
        .child(sliders(ctx))
        .child(open_apps(ctx))
        .child(notifications_link(ctx));
    Column::new()
        .gap(8.0)
        .child(
            Row::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(ScrollView::new().vertical().child(left).class("shade-col grow"))
                .child(ScrollView::new().vertical().child(tiles(ctx)).class("shade-col grow"))
                .class("grow"),
        )
        .child(handle())
        .class("shade shade-wide")
}

/// В быстрых настройках — строка «N уведомлений»: тап переключает на ленту.
fn notifications_link(ctx: ShellCtx) -> impl Widget {
    rx(move || {
        let n = ctx.history.get().len();
        if n == 0 {
            return Box::new(DecoratedBox::new()) as Box<dyn Widget>;
        }
        Box::new(
            GestureDetector::new().on_click(|| show(Panel::Notifications)).child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon(mi::BELL).class("shade-slider-icon"))
                            .child(Text::new(tn!(n, "{n} уведомление", "{n} уведомления", "{n} уведомлений")).class("shade-tile-label grow"))
                            .child(icon(mi::CHEVRON_RIGHT).class("shade-slider-icon")),
                    )
                    .class("shade-slider shade-notif-link"),
            ),
        )
    })
}

/// Шторка уведомлений: время и дата, «Не беспокоить», «Очистить», лента.
/// Горизонтально — шапка колонкой слева, лента справа (карточки во всю ширину
/// экрана читались плохо).
fn notifications(ctx: ShellCtx, landscape: bool) -> Box<dyn Widget> {
    let time = rx(move || {
        let now = ctx.now.get();
        Box::new(
            Column::new()
                .gap(0.0)
                .child(Text::new(crate::clock::format(now, "%H:%M")).class("shade-time"))
                .child(Text::new(crate::clock::format(now, "%A, %d %B")).class("shade-date")),
        )
    });
    let list = ScrollView::new().vertical().child(crate::notifications::history_list(ctx)).class("shade-notif-list grow");
    if landscape {
        let side = Column::new()
            .gap(12.0)
            .child(time)
            .child(Text::new(t!("Уведомления")).class("shade-section"))
            .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(crate::notifications::dnd_chip(ctx)).child(crate::notifications::clear_button()))
            .class("shade-notif-side");
        return Box::new(
            Column::new()
                .gap(8.0)
                .child(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(side).child(list).class("grow"))
                .child(handle())
                .class("shade shade-notif-panel shade-wide"),
        );
    }
    Box::new(
        Column::new()
            .gap(12.0)
            .child(time)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Уведомления")).class("shade-section grow"))
                    .child(crate::notifications::dnd_chip(ctx))
                    .child(crate::notifications::clear_button()),
            )
            .child(list)
            .child(handle())
            .class("shade shade-notif-panel"),
    )
}

fn header(ctx: ShellCtx) -> impl Widget {
    let time = rx(move || {
        let now = ctx.now.get();
        Box::new(
            Column::new()
                .gap(0.0)
                .child(Text::new(crate::clock::format(now, "%H:%M")).class("shade-time"))
                .child(Text::new(crate::clock::format(now, "%A, %d %B")).class("shade-date")),
        )
    });
    let btn = |glyph: &'static str, f: fn()| {
        GestureDetector::new().on_click(f).child(DecoratedBox::new().child(icon(glyph).class("shade-head-icon")).class("shade-head-btn"))
    };
    // камера включена — значок «используется» (строки состояния на телефоне нет)
    let camera = rx(move || -> Box<dyn Widget> {
        if ctx.camera.get().is_some() {
            Box::new(DecoratedBox::new().child(icon(mi::CAMERA).class("shade-head-icon")).class("shade-head-btn shade-head-active"))
        } else {
            Box::new(DecoratedBox::new())
        }
    });
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(time).class("grow"))
        .child(camera)
        .child(btn(mi::SETTINGS, || {
            close();
            crate::actions::spawn("synsettings");
        }))
        .child(btn(mi::POWER, || {
            close();
            crate::commands::handle("power-menu");
        }))
}

// ─── Плитки и их варианты ────────────────────────────────────────────────────
//
// Плитка — [`Tile`]: короткое нажатие делает своё (переключает, открывает), удержание или
// правый щелчок открывают список вариантов плитки ([`TileMenu`]) поверх шторки: режимы,
// «Включить»/«Выключить», «Настройки…» (раздел «Параметров»). Варианты задаёт сама плитка
// ([`Tile::option`], [`Tile::switch`], [`Tile::settings`]); без своих — хотя бы «Настройки…».

type Act = Arc<dyn Fn() + Send + Sync>;

/// Пункт списка вариантов плитки.
#[derive(Clone)]
struct TileOption {
    glyph: &'static str,
    title: String,
    hint: String,
    /// Текущий вариант — с отметкой.
    checked: bool,
    /// Черта перед пунктом: отделяет действия («Очистить экран», «Настройки…») от вариантов.
    divider: bool,
    action: Act,
}

impl TileOption {
    fn new(glyph: &'static str, title: impl Into<String>, action: impl Fn() + Send + Sync + 'static) -> Self {
        Self { glyph, title: title.into(), hint: String::new(), checked: false, divider: false, action: Arc::new(action) }
    }

    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    fn divider(mut self) -> Self {
        self.divider = true;
        self
    }
}

/// Список вариантов плитки: заголовок (подпись плитки) и пункты.
#[derive(Clone)]
struct TileMenu {
    /// Номер показа: сигнал сравнивает списки по нему (пункты с замыканиями не сравнить).
    seq: u64,
    glyph: String,
    title: String,
    options: Vec<TileOption>,
}

impl PartialEq for TileMenu {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}

/// Плитка быстрой настройки: значок, подпись, состояние; `on` — включена.
struct Tile {
    glyph: String,
    label: String,
    state: String,
    on: bool,
    tap: Act,
    options: Vec<TileOption>,
}

impl Tile {
    fn new(glyph: &str, label: impl Into<String>, state: impl Into<String>, on: bool, tap: impl Fn() + Send + Sync + 'static) -> Self {
        Self { glyph: glyph.to_string(), label: label.into(), state: state.into(), on, tap: Arc::new(tap), options: Vec::new() }
    }

    fn option(mut self, o: TileOption) -> Self {
        self.options.push(o);
        self
    }

    /// «Включить» и «Выключить» (отметка — у текущего; `None` — состояние неизвестно).
    fn switch(self, on: Option<bool>, set: impl Fn(bool) + Send + Sync + 'static) -> Self {
        let set = Arc::new(set);
        let s2 = set.clone();
        self.option(TileOption::new(GLYPH_ON, t!("Включить"), move || set(true)).checked(on == Some(true)))
            .option(TileOption::new(GLYPH_OFF, t!("Выключить"), move || s2(false)).checked(on == Some(false)))
    }

    /// «Настройки…» — раздел `page` «Параметров» (`synsettings <page>`); шторка закрывается.
    fn settings(self, page: &'static str) -> Self {
        self.option(
            TileOption::new(mi::SETTINGS, t!("Настройки…"), move || {
                close();
                crate::actions::spawn(&format!("synsettings {page}"));
            })
            .divider(),
        )
    }

    fn build(self) -> impl Widget {
        let class = if self.on { "shade-tile shade-tile-on" } else { "shade-tile" };
        let tap = self.tap;
        let menu = TileMenu { seq: 0, glyph: self.glyph.clone(), title: self.label.clone(), options: self.options };
        GestureDetector::new()
            .on_click(move || tap())
            // без своего `on_long_press` удержание пальцем приходит сюда же
            .on_secondary_click(move |_| open_menu(menu.clone()))
            .child(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().child(icon(&self.glyph).class("shade-tile-icon")).class("shade-tile-badge"))
                            .child(
                                Column::new()
                                    .gap(0.0)
                                    .child(Text::new(self.label).max_lines(1).class("shade-tile-label"))
                                    .child(Text::new(self.state).max_lines(1).class("shade-tile-state"))
                                    .class("grow"),
                            ),
                    )
                    .class(class),
            )
    }
}

const GLYPH_ON: &str = "\u{E9F6}"; // toggle_on
const GLYPH_OFF: &str = "\u{E9F5}"; // toggle_off
const GLYPH_CHECK: &str = "\u{E5CA}"; // check
const GLYPH_TUNE: &str = "\u{E429}"; // tune

/// Показать список вариантов плитки поверх шторки.
fn open_menu(mut m: TileMenu) {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    m.seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if m.options.is_empty() {
        return;
    }
    if let Some(sig) = MENU.with(|s| s.get()) {
        sig.set(Some(m));
    }
}

fn close_menu() {
    if let Some(sig) = MENU.with(|s| s.get()) {
        sig.set(None);
    }
}

/// Слой списка вариантов над шторкой: затемнение (тап — закрыть) и карточка по центру.
/// На E Ink — без анимации и полупрозрачности: белая карточка в тёмной рамке, отметка текущего —
/// инверсией строки.
fn tile_menu_layer(ctx: ShellCtx, menu: RwSignal<Option<TileMenu>>) -> impl Widget {
    let eink = synshell_common::display::is_eink();
    rx(move || -> Box<dyn Widget> {
        let Some(m) = menu.get() else { return Box::new(DecoratedBox::new()) };
        let (w, h) = crate::manager::output_size(crate::manager::focused_output(&ctx).as_deref());
        let width = (w - 32.0).clamp(240.0, 420.0);
        let mut rows = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, o) in m.options.into_iter().enumerate() {
            if o.divider && i > 0 {
                rows = rows.child(DecoratedBox::new().class("tile-menu-sep"));
            }
            let act = o.action.clone();
            let mut text = Column::new().gap(1.0).child(Text::new(o.title).class("tile-menu-label"));
            if !o.hint.is_empty() {
                text = text.child(Text::new(o.hint).max_lines(2).class("tile-menu-hint"));
            }
            let mut row = Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(o.glyph).class("tile-menu-icon"))
                .child(text.class("grow"));
            if o.checked {
                row = row.child(icon(GLYPH_CHECK).class("tile-menu-check"));
            }
            rows = rows.child(
                GestureDetector::new()
                    .on_click(move || {
                        close_menu();
                        act();
                    })
                    .child(DecoratedBox::new().child(row).class(if o.checked { "tile-menu-row tile-menu-row-on" } else { "tile-menu-row" })),
            );
        }
        let head = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(icon(&m.glyph).class("tile-menu-head-icon"))
            .child(Text::new(m.title).max_lines(1).class("tile-menu-title grow"))
            .child(GestureDetector::new().on_click(close_menu).child(DecoratedBox::new().child(icon(mi::CLOSE).class("tile-menu-icon")).class("tile-menu-close")));
        let card = Column::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(head)
            .child(ScrollView::new().vertical().child(rows).style("max-height", StyleValue::px((h - 140.0).max(200.0))))
            .class(if eink { "tile-menu tile-menu-eink" } else { "tile-menu" })
            .style("width", StyleValue::px(width));
        // тап по карточке мимо пунктов не закрывает список
        let card = GestureDetector::new().on_click(|| {}).child(card);
        let card: Box<dyn Widget> = if eink {
            Box::new(card)
        } else {
            let dur = crate::anim::group_ms(&ctx, "menu", 160);
            Box::new(Presence::new(true, card).enter(Motion::fade().scale(0.94)).duration_ms(dur).initial(dur > 0))
        };
        Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(close_menu).child(DecoratedBox::new().class(if eink { "tile-menu-scrim-eink" } else { "tile-menu-scrim" })))
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(card),
                ),
        )
    })
}

fn mode_label(m: MobileMode) -> String {
    match m {
        MobileMode::Pages => t!("Страницы"),
        MobileMode::Free => t!("Свободный стол"),
    }
}

fn mode_hint(m: MobileMode) -> String {
    match m {
        MobileMode::Pages => t!("Каждое приложение во весь экран, листание вбок"),
        MobileMode::Free => t!("Свободные окна на большом столе"),
    }
}

fn rotation_option(r: synshell_common::action::Rotation, cur: synshell_common::action::Rotation) -> TileOption {
    use synshell_common::action::Rotation;
    let (title, hint) = match r {
        Rotation::Normal => (t!("Обычная"), t!("0°")),
        Rotation::R90 => (t!("Повернуть вправо"), t!("90° по часовой")),
        Rotation::R180 => (t!("Вверх ногами"), "180°".to_string()),
        Rotation::R270 => (t!("Повернуть влево"), t!("90° против часовой")),
    };
    TileOption::new(GLYPH_ROTATE, title, move || crate::rotation::rotate_manual(r)).hint(hint).checked(r == cur)
}

/// Bluetooth вкл/выкл (BlueZ, в фоне).
fn bluetooth_power(on: bool) {
    std::thread::spawn(move || match synsystem::bluetooth::Bluetooth::new() {
        Some(bt) => {
            if let Err(e) = bt.set_powered(on) {
                log::warn!("Bluetooth: {e}");
            }
        }
        None => log::warn!("Bluetooth: нет адаптера или службы bluetoothd"),
    });
}

fn tiles(ctx: ShellCtx) -> impl Widget {
    // не из построения вида: запись сигнала посреди реактивного построения вешает оболочку
    syngui::async_runtime::run_on_main_thread(move || torch_refresh(ctx));
    crate::netmenu::refresh_state();
    // Состояния вне сигналов — пересобрать плитки по счётчику.
    let rev = use_signal(0u32);
    rx(move || {
        let _ = rev.get();
        let net = ctx.network.get();
        let dnd = ctx.dnd.get();
        let cfg = ctx.config.get();
        let mode = ctx.mobile_mode();
        let wifi_state = if net.online { net.connection.clone() } else { t!("Нет подключения").into() };
        let torch = ctx.torch.get();
        let wifi_on = crate::netmenu::powered();
        let mut grid = Grid::new(2).gap(8.0);
        let open_network = || {
            // Окно сети: Wi-Fi, подключение, «Параметры сети…».
            close();
            ShellCtx::get().open_popup(crate::ctx::PopupKind::Network, crate::commands::centered());
        };
        grid = grid.child(
            Tile::new(crate::applets::network_glyph(&net), t!("Сеть"), wifi_state, net.online, open_network)
                .option(TileOption::new(mi::WIFI, t!("Сети Wi-Fi…"), open_network).hint(t!("Список сетей и подключение")))
                .switch(wifi_on, crate::netmenu::set_powered)
                .settings("wifi")
                .build(),
        );
        // Мобильная связь (есть модем): тап — радио модема вкл/выкл
        let set_radio = |on: bool| {
            std::thread::spawn(move || {
                if let Err(e) = synmodem::api::request(&synmodem::api::Request::SetRadio { on }) {
                    log::warn!("радио модема: {e:#}");
                }
            });
        };
        if let Some(m) = ctx.modem.get().filter(|m| m.present) {
            let (title, sub) = crate::modem::summary(&m);
            let on = m.radio;
            let glyph = if !on { "\u{E195}" } else if m.registration.registered() { "\u{E1C8}" } else { "\u{E1D0}" };
            let state = if sub.is_empty() { title.clone() } else if on { format!("{title} · {sub}") } else { sub };
            grid = grid.child(Tile::new(glyph, t!("Мобильная связь"), state, on, move || set_radio(!on)).switch(Some(on), set_radio).settings("mobile-network").build());
        }
        // Мобильный интернет (передача данных по сотовой сети), отдельно от режима полёта
        if let Some(m) = ctx.modem.get().filter(|m| m.present) {
            use synmodem::api::DataState;
            let on = m.data.enabled;
            let state = match m.data.state {
                DataState::Off => t!("Выключен").to_string(),
                DataState::Waiting if !m.radio => t!("Режим полёта").into(),
                DataState::Waiting => t!("Нет сети").into(),
                DataState::Connecting => t!("Подключение…").into(),
                DataState::Connected if m.roaming => t!("{v} · роуминг", v = if m.technology.is_empty() { "Подключён" } else { m.technology.as_str() }),
                DataState::Connected => if m.technology.is_empty() { t!("Подключён").into() } else { m.technology.clone() },
                DataState::Error => t!("Ошибка").into(),
            };
            let set_data = |on: bool| {
                std::thread::spawn(move || {
                    if let Err(e) = synmodem::api::request(&synmodem::api::Request::SetData { on }) {
                        log::warn!("передача данных: {e:#}");
                    }
                });
            };
            grid = grid.child(Tile::new("\u{E8D5}", t!("Моб. интернет"), state, on, move || set_data(!on)).switch(Some(on), set_data).settings("mobile-network").build());
        }
        // Местоположение для программ (агент GeoClue): тап — выключатель [location] enabled
        {
            let on = cfg.location.enabled;
            let state = if !on {
                t!("Выключено").to_string()
            } else if ctx.modem.get().is_some_and(|m| m.gnss) {
                t!("Используется").into()
            } else {
                t!("Включено").into()
            };
            let set = |on: bool| {
                if let Err(e) = synshell_common::config_edit::set_value(&["location", "enabled"], toml_edit::Value::from(on)) {
                    log::warn!("[location] enabled: {e:#}");
                }
            };
            grid = grid.child(Tile::new(crate::location::GLYPH, t!("Местоположение"), state, on, move || set(!on)).switch(Some(on), set).settings("location").build());
        }
        let set_dnd = |on: bool| ShellCtx::get().dnd.set(on);
        grid = grid
            .child(
                Tile::new("\u{E1A7}", "Bluetooth", t!("Параметры"), false, || {
                    close();
                    crate::actions::spawn("synsettings bluetooth");
                })
                .switch(None, bluetooth_power)
                .settings("bluetooth")
                .build(),
            )
            .child(
                Tile::new(if dnd { mi::BELL_OFF } else { mi::BELL }, t!("Не беспокоить"), if dnd { t!("Включено") } else { t!("Выключено") }, dnd, move || set_dnd(!dnd))
                    .switch(Some(dnd), set_dnd)
                    .settings("notifications")
                    .build(),
            )
            .child({
                let mut t = Tile::new(mi::WINDOW, t!("Режим окон"), mode_label(mode), true, move || crate::actions::run(Action::MobileModeCycle));
                for m in MobileMode::ALL {
                    t = t.option(TileOption::new(mi::WINDOW, mode_label(m), move || crate::actions::run(Action::MobileMode(m))).hint(mode_hint(m)).checked(m == mode));
                }
                t.settings("phone").build()
            })
            .child({
                let game = || {
                    close();
                    crate::actions::run(Action::FormFactor(synshell_common::action::FormFactorTarget::Game));
                };
                Tile::new(crate::input_mode::GAMEPAD, t!("Игровой режим"), t!("Лаунчер игр"), false, game)
                    .option(TileOption::new(crate::input_mode::GAMEPAD, t!("Открыть лаунчер игр"), game))
                    .settings("gameshell")
                    .build()
            })
            .child({
                // Автоповорот: выключен — ориентация зафиксирована; без акселерометра — неактивен.
                let sensor = crate::rotation::has_sensor();
                let auto = cfg.rotation.auto && sensor;
                let (glyph, state) = match (sensor, auto) {
                    (false, _) => ("\u{E1C0}", t!("Нет датчика")),
                    (true, true) => ("\u{E1C1}", t!("Включён")),
                    (true, false) => ("\u{E1C0}", t!("Фиксация")),
                };
                let t = Tile::new(glyph, t!("Автоповорот"), state, auto, move || {
                    if sensor {
                        crate::rotation::set_auto(!auto)
                    }
                });
                let t = if sensor { t.switch(Some(auto), crate::rotation::set_auto) } else { t };
                t.settings("phone").build()
            })
            .child({
                // Ручной поворот на четверть оборота по часовой (запоминается, фиксирует ориентацию).
                let _ = ctx.comp_outputs.get();
                let r = crate::rotation::current_rotation(&ctx);
                let state = if r == synshell_common::action::Rotation::Normal { t!("Обычная").to_string() } else { format!("{}°", r.as_str()) };
                let mut t = Tile::new(GLYPH_ROTATE, t!("Повернуть экран"), state, false, || crate::rotation::rotate_next(&ShellCtx::get()));
                for o in synshell_common::action::Rotation::ALL {
                    t = t.option(rotation_option(o, r));
                }
                t.settings("displays").build()
            });
        if let Some(t) = torch {
            let on = t.on;
            let state = if on { t!("Включён · {v}%", v = format!("{:.0}", t.level * 100.0)) } else { t!("Выключен").into() };
            let set = move |want: bool| {
                if ShellCtx::get().torch.get_untracked().is_some_and(|s| s.on) != want {
                    torch_toggle(ShellCtx::get());
                }
                rev.set(rev.get_untracked() + 1);
            };
            grid = grid.child(
                Tile::new(GLYPH_TORCH, t!("Фонарик"), state, on, move || {
                    torch_toggle(ShellCtx::get());
                    rev.set(rev.get_untracked() + 1);
                })
                .switch(Some(on), set)
                .build(),
            );
        }
        grid = eink_tiles(grid, rev);
        grid = if ctx.is_phone() {
            // Экранный ввод приложения в фокусе: клавиатура, контроллер или ничего.
            use synshell_common::config::InputMode;
            let _ = ctx.focused.get();
            let mode = crate::input_mode::current(&ctx);
            let state = match crate::input_mode::focused_app_name(&ctx) {
                Some(n) => format!("{} · {n}", mode.title()),
                None => mode.title().to_string(),
            };
            let mut t = Tile::new(crate::input_mode::glyph(mode), t!("Ввод"), state, mode == InputMode::Controller, || {
                close();
                ShellCtx::get().open_popup(crate::ctx::PopupKind::InputMode, crate::commands::centered());
            });
            for m in [InputMode::Keyboard, InputMode::Controller, InputMode::Off] {
                t = t.option(TileOption::new(crate::input_mode::glyph(m), m.title(), move || crate::input_mode::set_for_focused(&ShellCtx::get(), m)).checked(m == mode));
            }
            grid.child(t.option(TileOption::new(mi::KEYBOARD, t!("Показать клавиатуру"), || crate::actions::spawn("synkeyboard show"))).settings("keyboard").build())
        } else {
            grid.child(
                Tile::new(mi::KEYBOARD, t!("Клавиатура"), t!("Показать"), false, || {
                    close();
                    crate::actions::spawn("synkeyboard toggle");
                })
                .settings("keyboard")
                .build(),
            )
        };
        // снимок — когда шторка уже ушла с экрана
        let shot = |a: Action| {
            move || {
                close();
                let a = a.clone();
                syngui_layer::add_timer(std::time::Duration::from_millis(450), move || {
                    crate::actions::run(a.clone());
                    None
                });
            }
        };
        let recents = || {
            close();
            crate::recents::open();
        };
        let lock = || {
            close();
            crate::commands::handle("lock");
        };
        grid = grid
            .child(
                Tile::new("\u{E3B0}", t!("Снимок"), t!("Экрана"), false, shot(Action::Screenshot))
                    .option(TileOption::new("\u{E3B0}", t!("Весь экран"), shot(Action::Screenshot)))
                    .option(TileOption::new(mi::WINDOW, t!("Окно"), shot(Action::ScreenshotWindow)))
                    .option(TileOption::new("\u{E3C2}", t!("Область…"), shot(Action::ScreenshotInteractive)).hint(t!("Выделить часть экрана")))
                    .build(),
            )
            .child(Tile::new(mi::LAYERS, t!("Недавние"), t!("Приложения"), false, recents).option(TileOption::new(mi::LAYERS, t!("Открыть «Недавние»"), recents)).settings("phone").build())
            .child(Tile::new(mi::LOCK, t!("Блокировка"), t!("Экрана"), false, lock).option(TileOption::new(mi::LOCK, t!("Заблокировать"), lock)).settings("lock").build());
        Box::new(grid)
    })
}

// ─── Экран E Ink ─────────────────────────────────────────────────────────────

const GLYPH_ROTATE: &str = "\u{E41A}"; // rotate_right
const GLYPH_COLOR: &str = "\u{E40A}"; // palette
const GLYPH_MONO: &str = "\u{E3DB}"; // filter_b_and_w
const GLYPH_SPEED: &str = "\u{E9E4}"; // speed
const GLYPH_REFRESH: &str = "\u{E5D5}"; // refresh

/// Плитки экрана E Ink (только на E Ink): цвет/монохром (цветная панель), профиль по кругу
/// (удержание — список профилей, «Редактор профилей…» и очистка), полная очистка. Профили —
/// из общего файла (`eink_profiles`), активный — `[eink] profile`; выбор пишется сразу в драйвер
/// (в фоне) и в конфиг (synwm применит его же — без лишней записи — и запомнит для экрана входа).
fn eink_tiles(mut grid: Grid, rev: RwSignal<u32>) -> Grid {
    use synshell_common::display::{self, Eink};
    use synshell_common::eink_profiles::{self, Profile};
    let kind = display::display_kind();
    if !kind.is_eink() {
        return grid;
    }
    let Some(e) = Eink::find() else { return grid };
    // запись в sysfs — в фоне (драйвер может перерисовать экран), потом пересборка плиток
    let write = move |f: Box<dyn FnOnce(&Eink) -> std::io::Result<()> + Send>| {
        std::thread::spawn(move || {
            if let Some(e) = Eink::find() {
                if let Err(err) = f(&e) {
                    log::warn!("экран E Ink: {err}");
                }
            }
            syngui::async_runtime::run_on_main_thread(move || rev.set(rev.get_untracked() + 1));
        });
    };
    // очистка — когда шторка уже ушла с экрана
    let clear = move || {
        close();
        syngui_layer::add_timer(std::time::Duration::from_millis(450), move || {
            write(Box::new(|e| e.refresh()));
            None
        });
    };
    let editor = TileOption::new(GLYPH_TUNE, t!("Редактор профилей…"), || {
        close();
        crate::actions::spawn("syn-epdtest");
    })
    .hint(t!("Создать, назвать и настроить профили"))
    .divider();
    if kind.is_color_eink() {
        let on = e.color().unwrap_or(false);
        let set = move |c: bool| {
            write(Box::new(move |e| e.set_color(c)));
            if let Err(err) = synshell_common::config_edit::set_value(&["eink", "color"], toml_edit::Value::from(c)) {
                log::warn!("[eink] color: {err:#}");
            }
        };
        let (glyph, state) = if on { (GLYPH_COLOR, t!("Цветной")) } else { (GLYPH_MONO, t!("Монохром")) };
        grid = grid.child(
            Tile::new(glyph, t!("Цвет"), state, on, move || set(!on))
                .option(TileOption::new(GLYPH_COLOR, t!("Цветной"), move || set(true)).hint(t!("Цветной фильтр Kaleido")).checked(on))
                .option(TileOption::new(GLYPH_MONO, t!("Монохром"), move || set(false)).hint(t!("Чёрно-белый, чётче текст")).checked(!on))
                .settings("displays")
                .build(),
        );
    }
    // профили: встроенный «По умолчанию» и созданные в редакторе
    let profiles = eink_profiles::load();
    let active = profiles.resolve(Some(&eink_profiles::active_name(&synshell_common::Config::load().0.eink)));
    let select = Arc::new(move |p: Profile| {
        if let Err(err) = eink_profiles::set_active(&p.name) {
            log::warn!("[eink] profile: {err:#}");
        }
        write(Box::new(move |e| e.apply_profile(&p)));
    });
    let next = profiles.next_after(&active.name);
    let s = select.clone();
    let mut t = Tile::new(GLYPH_SPEED, t!("Режим E Ink"), active.title(), false, move || s(next.clone()));
    for p in profiles.all() {
        let s = select.clone();
        let on = p.name == active.name;
        let (title, hint) = (p.title(), p.summary());
        t = t.option(TileOption::new(GLYPH_SPEED, title, move || s(p.clone())).hint(hint).checked(on));
    }
    grid = grid.child(
        t.option(editor.clone())
            .option(TileOption::new(GLYPH_REFRESH, t!("Очистить экран"), clear).hint(t!("Убрать следы")))
            .settings("displays")
            .build(),
    );
    grid.child(
        Tile::new(GLYPH_REFRESH, t!("Очистить экран"), t!("Убрать следы"), false, clear)
            .option(TileOption::new(GLYPH_REFRESH, t!("Очистить сейчас"), clear))
            .option(editor)
            .build(),
    )
}

// ─── Открытые приложения ─────────────────────────────────────────────────────

/// Открытые окна лентой: тап — перейти, «×» — закрыть, «Все» — «Недавние». Удержание или
/// правый щелчок — на чипе «Закрыть» и «Убить» (SIGKILL). Нет окон — раздела нет.
fn open_apps(ctx: ShellCtx) -> impl Widget {
    let armed = use_signal(None::<u64>);
    rx(move || {
        let armed_id = armed.get();
        let list = crate::recents::visible_windows(&ctx);
        if list.is_empty() {
            return Box::new(DecoratedBox::new()) as Box<dyn Widget>;
        }
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for w in list {
            let entry = crate::xdg::app_for_window(&w.app_id);
            let name = entry.as_ref().map(|e| e.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| {
                if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() }
            });
            let icon_path = entry.as_ref().and_then(|e| crate::xdg::lookup_icon(&e.icon)).or_else(|| crate::xdg::window_icon(&w.app_id));
            let id = w.id;
            let mut chip = Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(crate::launchers::icon_widget(&icon_path, &None, "shade-app-icon", 24.0))
                .child(Text::new(name).max_lines(1).class("shade-app-name"));
            if armed_id == Some(id) {
                let act = |glyph: &'static str, label: String, class: &'static str, op: WindowOp| {
                    GestureDetector::new()
                        .on_click(move || {
                            crate::actions::window_op(id, op);
                            armed.set(None);
                        })
                        .child(
                            DecoratedBox::new()
                                .child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("shade-act-icon")).child(Text::new(label).class("shade-act-text")))
                                .class(class),
                        )
                };
                chip = chip
                    .child(act(mi::CLOSE, t!("Закрыть"), "shade-act", WindowOp::Close))
                    .child(act(crate::desk::KILL, t!("Убить"), "shade-act shade-act-kill", WindowOp::Kill));
            } else {
                chip = chip.child(
                    GestureDetector::new()
                        .on_click(move || crate::actions::window_op(id, WindowOp::Close))
                        .child(DecoratedBox::new().child(icon(mi::CLOSE).class("shade-app-close-icon")).class("shade-app-close")),
                );
            }
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        if armed.get_untracked().is_some() {
                            armed.set(None);
                            return;
                        }
                        crate::actions::window_op(id, WindowOp::Activate);
                        close();
                    })
                    .on_long_press(move |_| armed.set(Some(id)))
                    .on_secondary_click(move |_| armed.set(Some(id)))
                    .child(DecoratedBox::new().child(chip).class(if w.focused { "shade-app shade-app-focused" } else { "shade-app" })),
            );
        }
        let all = GestureDetector::new()
            .on_click(|| {
                close();
                crate::recents::open();
            })
            .child(DecoratedBox::new().child(Text::new(t!("Все")).class("shade-apps-all-text")).class("shade-apps-all"));
        Box::new(
            Column::new()
                .gap(6.0)
                .child(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(t!("Открытые приложения")).class("shade-section grow"))
                        .child(all),
                )
                .child(ScrollView::new().horizontal().child(row)),
        )
    })
}

// ─── Яркость и громкость ─────────────────────────────────────────────────────

/// Максимальная яркость панели, нит (`[[output]] max_nits` вывода с
/// фокусом или первого, где задано).
fn max_nits(ctx: &ShellCtx) -> Option<f32> {
    let cfg = ctx.cfg();
    let out = crate::manager::focused_output(ctx);
    cfg.outputs
        .iter()
        .find(|o| out.as_deref() == Some(o.name.as_str()) && o.max_nits.is_some())
        .or_else(|| cfg.outputs.iter().find(|o| o.max_nits.is_some()))
        .and_then(|o| o.max_nits)
}

fn sliders(ctx: ShellCtx) -> impl Widget {
    let sys = synsystem::Sys::host();
    let bl = synsystem::backlight::primary(&sys);
    let nits = max_nits(&ctx);
    // Общая с автояркостью: ползунок идёт за ней, а её поправку задаёт ползунок.
    let bright = ctx.brightness;
    if let Some(b) = bl.as_ref() {
        bright.set(Some(b.percent()));
    }
    // Запись в sysfs — в фоне, последнее значение побеждает.
    let pending: Arc<Mutex<Option<f32>>> = Arc::new(Mutex::new(None));
    let mut col = Column::new().gap(12.0);
    if bl.is_some() {
        // Вся строка — реактивная, как у громкости: ползунок идёт за автояркостью. Пересборка не рвёт
        // перетаскивание — элемент ползунка не трогает значение, пока его тянут.
        let row = rx(move || {
            // внутри rx: смена `[brightness] auto` (кнопка «А», Параметры) перерисовывает строку
            let auto = ctx.config.get().brightness.auto;
            let pct = bright.get().unwrap_or(50.0);
            let text = match nits {
                Some(max) => t!("{v} нит · {pct}%", v = format!("{:.0}", pct / 100.0 * max), pct = format!("{:.0}", pct)),
                None => format!("{pct:.0}%"),
            };
            let p2 = pending.clone();
            let slider = Slider::new().range(1.0, 100.0).step(1.0).value(pct).on_change(move |v| {
                bright.set(Some(v));
                let first = p2.lock().unwrap().replace(v).is_none();
                if first {
                    let p3 = p2.clone();
                    std::thread::spawn(move || {
                        let sys = synsystem::Sys::host();
                        while let Some(v) = p3.lock().unwrap().take() {
                            match synsystem::backlight::set_percent(&sys, v) {
                                Ok(_) => crate::backlight::remember_level(v),
                                Err(e) => log::warn!("яркость: {e}"),
                            }
                        }
                    });
                }
            });
            // Кнопка автояркости (как в Android): включена — акцентный кружок «А», выключена — значок яркости.
            let badge: Box<dyn Widget> = if auto {
                Box::new(DecoratedBox::new().child(Text::new(t!("А")).class("shade-auto-badge")).class("shade-auto shade-auto-on"))
            } else {
                Box::new(DecoratedBox::new().child(icon(mi::BRIGHTNESS).class("shade-slider-icon")).class("shade-auto"))
            };
            Box::new(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(GestureDetector::new().on_click(move || crate::autobright::set_auto(!auto)).child(badge))
                            .child(slider.class("grow"))
                            .child(Text::new(text).class("shade-slider-value")),
                    )
                    .class("shade-slider"),
            )
        });
        col = col.child(row);
    }
    if let Some(w) = synsystem::backlight::front_light(&sys).map(|fl| fl.state(&sys).1) {
        col = col.child(warmth_row(w));
    }
    let vol = rx(move || {
        let v = ctx.volume.get();
        let Some(v) = v else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        Box::new(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(
                            GestureDetector::new()
                                .on_click(|| crate::system::toggle_mute(ShellCtx::get(), false))
                                // в кружке той же высоты, что кнопка автояркости, — строки ползунков ровные
                                .child(
                                    DecoratedBox::new()
                                        .child(icon(if v.muted { mi::VOLUME_OFF } else { mi::VOLUME_UP }).class("shade-slider-icon"))
                                        .class("shade-auto"),
                                ),
                        )
                        .child(
                            Slider::new()
                                .range(0.0, ShellCtx::get().config.get().sound.max_volume() as f32)
                                .step(1.0)
                                .value(v.percent as f32)
                                .on_change(|x| crate::system::change_volume(ShellCtx::get(), 0, Some(x.round() as u32)))
                                .class("grow"),
                        )
                        .child(Text::new(format!("{}%", v.percent)).class("shade-slider-value")),
                )
                .class("shade-slider"),
        )
    });
    col.child(vol).child(torch_row(ctx)).style("padding", StyleValue::px(0.0))
}

/// Тепло передней подсветки из двух каналов (белый + тёплый, E Ink): холодно ↔ тепло.
fn warmth_row(w: f32) -> impl Widget {
    let val = use_signal(w);
    let text = rx(move || Box::new(Text::new(format!("{:.0}%", val.get())).class("shade-slider-value")) as Box<dyn Widget>);
    let slider = Slider::new().range(0.0, 100.0).step(1.0).value(w).on_change(move |v| {
        val.set(v);
        crate::backlight::set_warmth(v);
    });
    DecoratedBox::new()
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(icon(mi::WARMTH).class("shade-slider-icon")).class("shade-auto"))
                .child(slider.class("grow"))
                .child(text),
        )
        .class("shade-slider")
}

// ─── Фонарик ─────────────────────────────────────────────────────────────────
// Светодиоды вспышки (synsystem::torch, описание — /etc/syn-torch.conf платформы). Включённый фонарик —
// строка в ползунках шторки: кнопка выключения, яркость, у двухтоновой вспышки — кнопки «Т» и «Х».

/// Последняя яркость (доля наибольшего тока): с ней фонарик включается снова.
static TORCH_LEVEL: Mutex<f32> = Mutex::new(0.3);

fn torch_refresh(ctx: ShellCtx) {
    let st = synsystem::torch::Torch::find().map(|t| t.state());
    if let Some(s) = st.filter(|s| s.on) {
        *TORCH_LEVEL.lock().unwrap() = s.level;
    }
    ctx.torch.set(st);
}

/// Записать состояние (в фоне; последнее побеждает) и показать его сразу.
fn torch_set(ctx: ShellCtx, st: synsystem::torch::State) {
    static PENDING: Mutex<Option<synsystem::torch::State>> = Mutex::new(None);
    if st.on {
        *TORCH_LEVEL.lock().unwrap() = st.level;
    }
    ctx.torch.set(Some(st));
    let first = PENDING.lock().unwrap().replace(st).is_none();
    if first {
        std::thread::spawn(|| {
            let Some(t) = synsystem::torch::Torch::find() else { return };
            while let Some(st) = PENDING.lock().unwrap().take() {
                if let Err(e) = t.set(st) {
                    log::warn!("фонарик: {e}");
                }
            }
        });
    }
}

fn torch_toggle(ctx: ShellCtx) {
    let cur = ctx.torch.get_untracked().unwrap_or(synsystem::torch::State { on: false, level: 0.0, warm: false, cold: false });
    let level = *TORCH_LEVEL.lock().unwrap();  // отдельно: torch_set берёт тот же мьютекс
    torch_set(ctx, synsystem::torch::State { on: !cur.on, level, ..cur });
}

/// Строка включённого фонарика в ползунках шторки.
fn torch_row(ctx: ShellCtx) -> impl Widget {
    let two_tone = synsystem::torch::Torch::find().is_some_and(|t| t.two_tone());
    rx(move || {
        let Some(st) = ctx.torch.get().filter(|s| s.on) else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        let pct = (st.level * 100.0).round().clamp(1.0, 100.0);
        let badge = |text: &'static str, on: bool, f: Box<dyn Fn() + Send + Sync>| {
            GestureDetector::new().on_click(move || f()).child(
                DecoratedBox::new()
                    .child(Text::new(syngui::i18n::t(text)).class("shade-auto-badge"))
                    .class(if on { "shade-auto shade-auto-on" } else { "shade-auto shade-auto-off" }),
            )
        };
        let mut row = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(GestureDetector::new().on_click(move || torch_toggle(ShellCtx::get())).child(
                DecoratedBox::new().child(icon(GLYPH_TORCH).class("shade-auto-badge-icon")).class("shade-auto shade-auto-on"),
            ))
            .child(
                Slider::new()
                    .range(1.0, 100.0)
                    .step(1.0)
                    .value(pct)
                    .on_change(move |v| torch_set(ShellCtx::get(), synsystem::torch::State { level: v / 100.0, ..st }))
                    .class("grow"),
            )
            .child(Text::new(format!("{pct:.0}%")).class("shade-slider-value"));
        if two_tone {
            // оба выключены — горят оба; «Т»/«Х» — только тёплый/холодный, оба включены — смесь
            row = row
                .child(badge(n_!("Т"), st.warm, Box::new(move || torch_set(ShellCtx::get(), synsystem::torch::State { warm: !st.warm, ..st }))))
                .child(badge(n_!("Х"), st.cold, Box::new(move || torch_set(ShellCtx::get(), synsystem::torch::State { cold: !st.cold, ..st }))));
        }
        Box::new(DecoratedBox::new().child(row).class("shade-slider"))
    })
}

const GLYPH_TORCH: &str = "\u{E3E7}";
