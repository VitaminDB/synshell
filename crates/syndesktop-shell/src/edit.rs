//! Режим редактирования панелей и дока: значки можно перетаскивать,
//! удалять и настраивать, добавлять приложения, разделы (группы значков со
//! всплывающим окном), папки и апплеты. Правки сразу пишутся в
//! `config.toml` (`syndesktop_common::config_edit`) — оболочка
//! перечитывает его и пересобирает панели.
//!
//! Здесь же меню панели и значков (правый клик).

use std::any::Any;
use syndesktop_common::config::Applet;
use syndesktop_common::ipc::WindowOp;
use syngui::core::{Point, Rect, Size};
use syngui::input::{Event, EventResult, MouseButton};
use syngui::layout::Constraints;
use syngui::mss::{ComputedStyle, MssFields};
use syngui::prelude::*;
use syngui::render::DisplayList;
use syngui::widget::context::EventContext;
use syngui::widget::{DirtyFlags, Element, ElementId, ElementTree, LayoutHint, UpdateContext};
use syngui::widgets::{DropArea, Draggable};

use crate::ctx::{PopupKind, ShellCtx};
use crate::launchers::{self, Launchable};
use crate::panel::PanelCtx;
use crate::popup::menu_item;
use crate::ui::{boxed, icon, mi, InputArea};

const DRAG_TYPE: &str = "syndesktop-panel-item";

// ─── Правки конфига ─────────────────────────────────────────────────────────

fn applets_of(panel: usize) -> Vec<Applet> {
    ShellCtx::get().cfg().panels.get(panel).map(|p| p.applets.clone()).unwrap_or_default()
}

fn save(panel: usize, applets: Vec<Applet>) {
    let cfg = ShellCtx::get().cfg();
    match syndesktop_common::config_edit::set_panel_applets(panel, &cfg.panels, &applets) {
        Ok(_) => crate::reload_after_write(),
        Err(e) => {
            log::error!("не удалось сохранить панель: {e:#}");
            crate::osd::show(ShellCtx::get(), mi::INFO, None, "Не удалось сохранить config.toml".into());
        }
    }
}

pub fn remove(panel: usize, index: usize) {
    let mut a = applets_of(panel);
    if index < a.len() {
        a.remove(index);
        save(panel, a);
    }
}

/// Переставить апплет `from` на место `to` (номер до удаления).
pub fn move_item(panel: usize, from: usize, to: usize) {
    let mut a = applets_of(panel);
    if from >= a.len() || from == to {
        return;
    }
    let item = a.remove(from);
    let to = if to > from { to - 1 } else { to }.min(a.len());
    a.insert(to, item);
    save(panel, a);
}

pub fn replace(panel: usize, index: usize, applet: Applet) {
    let mut a = applets_of(panel);
    if index < a.len() {
        a[index] = applet;
        save(panel, a);
    }
}

/// Куда вставить новый значок: после последнего значка запуска/раздела/
/// папки, иначе перед панелью задач, иначе в конец.
fn insert_position(list: &[Applet], applet: &Applet) -> usize {
    let launcher_like = |k: &str| matches!(k, "app" | "group" | "folder");
    if launcher_like(&applet.kind) {
        if let Some(i) = list.iter().rposition(|a| launcher_like(&a.kind) && (applet.kind != "app" || a.kind == "app")) {
            return i + 1;
        }
        if let Some(i) = list.iter().position(|a| a.kind == "taskbar") {
            return i;
        }
    }
    list.len()
}

pub fn add(panel: usize, applet: Applet) {
    let mut a = applets_of(panel);
    let at = insert_position(&a, &applet);
    a.insert(at, applet);
    save(panel, a);
}

/// Закрепить приложение (из меню работающего окна).
pub fn pin_app(panel: usize, app_id: &str) {
    let mut a = applets_of(panel);
    if a.iter().any(|x| x.kind == "app" && x.str("app") == Some(app_id)) {
        return;
    }
    let applet = app_applet(app_id);
    let at = a.iter().position(|x| x.kind == "taskbar").unwrap_or_else(|| insert_position(&a, &applet));
    a.insert(at, applet);
    save(panel, a);
}

pub fn app_applet(app_id: &str) -> Applet {
    let mut a = Applet::new("app");
    a.options.insert("app".into(), toml::Value::String(app_id.into()));
    a
}

fn set_mode(panel: usize, mode: &str) {
    let cfg = ShellCtx::get().cfg();
    let r = syndesktop_common::config_edit::set_panel_key(panel, &cfg.panels, "mode", mode);
    if let Err(e) = r {
        log::error!("не удалось сменить вид панели: {e:#}");
    }
    crate::reload_after_write();
}

pub fn start_editing(panel: usize) {
    let ctx = ShellCtx::get();
    ctx.close_popup();
    ctx.editing.set(Some(panel));
}

pub fn stop_editing() {
    let ctx = ShellCtx::get();
    ctx.editing.set(None);
    if ctx.popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::AddItem(_) | PopupKind::EditItem { .. })) {
        ctx.close_popup();
    }
}

// ─── Значок в режиме редактирования ─────────────────────────────────────────

/// Обёртка апплета панели: вне режима редактирования — как есть; в режиме —
/// перетаскивание (порядок), кнопка удаления, клик — настройка значка.
pub fn item_frame(pc: &PanelCtx, index: usize, w: Box<dyn Widget>, editing: bool) -> Box<dyn Widget> {
    if !editing {
        return w;
    }
    let panel = pc.index;
    let pc2 = pc.clone();
    let vertical = pc.vertical;
    let body = Draggable::new(DRAG_TYPE, format!("{panel}:{index}"))
        .threshold(6.0)
        .on_click(move || {
            let r = pc2.bounds_of(index).unwrap_or(Rect::zero());
            ShellCtx::get().open_popup(PopupKind::EditItem { panel, index }, pc2.anchor(r));
        })
        .child(Shield::new(DecoratedBox::new().child(w).class("edit-item")));
    let remove = InputArea::new(boxed("edit-remove", icon(mi::CLOSE)))
        .pointer()
        .on_click(move |b, _, _| {
            if b == MouseButton::Left {
                remove(panel, index);
            }
        });
    Box::new(
        DropArea::new()
            .accept_types(vec![DRAG_TYPE.to_string()])
            .on_drop_positioned(move |info| {
                let Some((p, from)) = info.data.payload.split_once(':') else { return };
                let (Ok(p), Ok(from)) = (p.parse::<usize>(), from.parse::<usize>()) else { return };
                if p != panel {
                    return;
                }
                let after = if vertical {
                    info.local_position.y > info.size.height / 2.0
                } else {
                    info.local_position.x > info.size.width / 2.0
                };
                move_item(panel, from, if after { index + 1 } else { index });
            })
            .child(Stack::new().child(body).child(remove).class("edit-frame")),
    )
}

/// Кнопки режима редактирования в конце панели: «добавить» и «готово».
pub fn edit_controls(pc: &PanelCtx) -> Box<dyn Widget> {
    let panel = pc.index;
    let pc2 = pc.clone();
    let slot = std::sync::Arc::new(syngui::core::sync::Mutex::new(Rect::zero()));
    let slot2 = slot.clone();
    let dir = if pc.vertical { FlexDirection::Column } else { FlexDirection::Row };
    Box::new(
        Flex::new()
            .direction(dir)
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("edit-controls")
            .child(
                syngui::widgets::EventHook::new().report_bounds(slot).child(
                    InputArea::new(boxed("edit-btn edit-add", icon("\u{E145}"))).pointer().on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            let r = *slot2.lock().unwrap_or_else(|e| e.into_inner());
                            ShellCtx::get().open_popup(PopupKind::AddItem(panel), pc2.anchor(r));
                        }
                    }),
                ),
            )
            .child(InputArea::new(boxed("edit-btn edit-done", icon("\u{E876}"))).pointer().on_click(|b, _, _| {
                if b == MouseButton::Left {
                    stop_editing();
                }
            })),
    )
}

/// Перехват нажатий: в режиме редактирования клик по значку не запускает
/// приложение, а достаётся обёртке (перетаскивание, настройка). Наведение
/// проходит — значки откликаются как обычно.
struct Shield {
    child: Box<dyn Widget>,
}

impl Shield {
    fn new(child: impl Widget + 'static) -> Self {
        Self { child: Box::new(child) }
    }
}

impl Widget for Shield {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ShieldElement { id: ElementId::new(), bounds: Rect::zero(), dirty: DirtyFlags::LAYOUT | DirtyFlags::RENDER, mss: MssFields::new() })
    }
    fn can_update(&self, other: &dyn Any) -> bool {
        other.is::<Self>()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn mount(&self, tree: &mut ElementTree, parent_id: ElementId) {
        let el = self.child.create_element();
        let id = tree.insert_with_type_id(el, Some(parent_id), self.child.as_any().type_id());
        self.child.mount(tree, id);
    }
    fn child_widgets(&self) -> Vec<&dyn Widget> {
        vec![self.child.as_ref()]
    }
}

struct ShieldElement {
    id: ElementId,
    bounds: Rect,
    dirty: DirtyFlags,
    mss: MssFields,
}

impl Element for ShieldElement {
    fn update(&mut self, _widget: &dyn Widget, _ctx: &mut UpdateContext) {}
    fn layout(&mut self, c: Constraints) -> Size {
        let size = Size::new(
            if c.max_width.is_finite() { c.max_width } else { 0.0 },
            if c.max_height.is_finite() { c.max_height } else { 0.0 },
        );
        self.bounds.size = size;
        size
    }
    fn layout_hint(&self) -> LayoutHint {
        LayoutHint::Padding { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 }
    }
    fn build_display_list(&self, _list: &mut DisplayList, _clip: Rect) {}
    fn handle_event(&mut self, _event: &Event, _ctx: &mut EventContext) -> EventResult {
        EventResult::Ignored
    }
    fn intercepts_event(&self, event: &Event) -> bool {
        matches!(event, Event::MouseDown { .. } | Event::MouseUp { .. } | Event::DoubleClick { .. } | Event::MouseWheel { .. })
    }
    fn children(&self) -> &[ElementId] {
        &[]
    }
    fn bounds(&self) -> Rect {
        self.bounds
    }
    fn set_position(&mut self, pos: Point) {
        self.bounds.origin = pos;
    }
    fn set_content_size(&mut self, size: Size) {
        self.bounds.size = size;
    }
    fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }
    fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
    fn is_dirty(&self, flags: DirtyFlags) -> bool {
        self.dirty.contains(flags)
    }
    fn id(&self) -> ElementId {
        self.id
    }
    fn set_id(&mut self, id: ElementId) {
        self.id = id;
    }
    fn mount(&mut self, _tree: &mut ElementTree) {}
    fn reset_mss_styles(&mut self) {
        self.mss.reset();
    }
    fn mss(&self) -> Option<&MssFields> {
        Some(&self.mss)
    }
    fn apply_computed_style(&mut self, style: &ComputedStyle) {
        self.mss.apply(style);
    }
    fn element_type_name(&self) -> &str {
        "EditShield"
    }
}

// ─── Меню ───────────────────────────────────────────────────────────────────

fn title(text: impl Into<String>) -> impl Widget {
    Text::new(text.into()).max_lines(1).class("popup-title")
}

fn sep() -> impl Widget {
    DecoratedBox::new().class("menu-sep")
}

/// Меню панели (правый клик по пустому месту).
pub fn panel_menu(ctx: ShellCtx, panel: usize) -> impl Widget {
    let cfg = ctx.cfg();
    let dock = cfg.panels.get(panel).is_some_and(|p| p.is_dock());
    let editing = ctx.editing.get_untracked() == Some(panel);
    let what = if dock { "док" } else { "панель" };
    Column::new()
        .gap(2.0)
        .child(title(if dock { "Док" } else { "Панель" }))
        .child(menu_item("\u{E3C9}", if editing { format!("Закончить правку ({what})") } else { format!("Изменить {what}") }, move || {
            if editing {
                stop_editing();
            } else {
                start_editing(panel);
            }
        }))
        .child(menu_item("\u{E145}", "Добавить значок, раздел, папку…", move || {
            let ctx = ShellCtx::get();
            ctx.editing.set(Some(panel));
            // Меню закроется, окно «Добавить» откроется следующим кадром у того же места.
            let anchor = ctx.popup.get_untracked().map(|p| p.anchor);
            syngui_layer::add_timer(std::time::Duration::from_millis(30), move || {
                if let Some(a) = anchor.clone() {
                    ShellCtx::get().popup.set(Some(crate::ctx::Popup { kind: PopupKind::AddItem(panel), anchor: a }));
                }
                None
            });
        }))
        .child(sep())
        .child(menu_item(if dock { "\u{E8F2}" } else { "\u{E30C}" }, if dock { "Сделать обычной панелью" } else { "Сделать доком" }, move || {
            set_mode(panel, if dock { "panel" } else { "dock" });
        }))
        .child(menu_item(mi::SETTINGS, "Параметры панелей…", || crate::actions::spawn("syndesktop-settings panels")))
}

/// Меню значка панели/дока.
pub fn item_menu(ctx: ShellCtx, panel: usize, index: usize) -> impl Widget {
    let cfg = ctx.cfg();
    let applet = cfg.panels.get(panel).and_then(|p| p.applets.get(index)).cloned().unwrap_or_else(|| Applet::new("?"));
    let mut col = Column::new().gap(2.0);
    match applet.kind.as_str() {
        "app" => {
            let l = Launchable::from_applet(&applet);
            col = col.child(title(l.name.clone()));
            col = window_items(ctx, col, &l.app_id);
            let l2 = l.clone();
            col = col.child(menu_item("\u{E89E}", "Новое окно", move || launchers::launch(ShellCtx::get(), &l2)));
            let app_id = l.app_id.clone();
            if !launchers::windows_of(&app_id, &ctx.windows.get_untracked()).is_empty() {
                col = col.child(menu_item(mi::CLOSE, "Закрыть все окна", move || close_all(&app_id)));
            }
            col = col.child(sep()).child(menu_item(mi::PUSH_PIN, "Открепить", move || remove(panel, index)));
        }
        "group" | "folder" => {
            col = col.child(title(launchers::stack_title(&applet)));
            let anchor = ctx.popup.get_untracked().map(|p| p.anchor);
            col = col.child(menu_item("\u{E89E}", "Открыть", move || {
                let a = anchor.clone();
                syngui_layer::add_timer(std::time::Duration::from_millis(30), move || {
                    if let Some(a) = a.clone() {
                        ShellCtx::get().popup.set(Some(crate::ctx::Popup { kind: PopupKind::Stack { panel, index, hover: false }, anchor: a }));
                    }
                    None
                });
            }));
            if applet.kind == "folder" {
                let spec = applet.str_or("path", "~").to_string();
                col = col.child(menu_item("\u{E2C8}", "Открыть в файловом менеджере", move || {
                    launchers::open_path(&launchers::resolve_path(&spec));
                }));
                if launchers::is_trash(applet.str_or("path", "")) {
                    col = col.child(menu_item("\u{E872}", "Очистить корзину", launchers::empty_trash));
                }
            }
            col = col.child(sep()).child(menu_item(mi::CLOSE, "Убрать", move || remove(panel, index)));
        }
        kind => {
            col = col.child(title(crate::edit::applet_label(kind)));
            col = col.child(menu_item(mi::CLOSE, "Убрать с панели", move || remove(panel, index)));
        }
    }
    if matches!(applet.kind.as_str(), "app" | "group" | "folder") {
        let anchor = ctx.popup.get_untracked().map(|p| p.anchor);
        col = col.child(menu_item("\u{E3C9}", "Настроить…", move || {
            let a = anchor.clone();
            ShellCtx::get().editing.set(Some(panel));
            syngui_layer::add_timer(std::time::Duration::from_millis(30), move || {
                if let Some(a) = a.clone() {
                    ShellCtx::get().popup.set(Some(crate::ctx::Popup { kind: PopupKind::EditItem { panel, index }, anchor: a }));
                }
                None
            });
        }));
    }
    let dock = cfg.panels.get(panel).is_some_and(|p| p.is_dock());
    col.child(menu_item("\u{E8B8}", if dock { "Изменить док" } else { "Изменить панель" }, move || start_editing(panel)))
}

/// Меню работающего приложения без значка (апплет `taskbar` на доке).
pub fn app_menu(ctx: ShellCtx, panel: usize, app_id: &str) -> impl Widget {
    let entry = crate::xdg::app_by_id(app_id);
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| app_id.to_string());
    let mut col = Column::new().gap(2.0).child(title(name));
    col = window_items(ctx, col, app_id);
    if let Some(e) = entry {
        let l = Launchable::from_entry(&e);
        col = col.child(menu_item("\u{E89E}", "Новое окно", move || launchers::launch(ShellCtx::get(), &l)));
        let id = app_id.to_string();
        col = col.child(menu_item(mi::PUSH_PIN, "Закрепить", move || pin_app(panel, &id)));
    }
    let id = app_id.to_string();
    col.child(sep()).child(menu_item(mi::CLOSE, "Закрыть все окна", move || close_all(&id)))
}

fn window_items(ctx: ShellCtx, mut col: Column, app_id: &str) -> Column {
    let windows = ctx.windows.get_untracked();
    let mine = launchers::windows_of(app_id, &windows);
    for w in &mine {
        let id = w.id;
        let t = if w.title.is_empty() { w.app_id.clone() } else { w.title.clone() };
        let glyph = if w.minimized { mi::MINIMIZE } else { mi::WINDOW };
        col = col.child(menu_item(glyph, t, move || crate::actions::window_op(id, WindowOp::Activate)));
    }
    if !mine.is_empty() {
        col = col.child(sep());
    }
    col
}

fn close_all(app_id: &str) {
    let ctx = ShellCtx::get();
    for w in launchers::windows_of(app_id, &ctx.windows.get_untracked()) {
        crate::actions::window_op(w.id, WindowOp::Close);
    }
}

// ─── «Добавить» ─────────────────────────────────────────────────────────────

/// Апплеты, которые можно добавить из режима редактирования.
const APPLET_KINDS: &[(&str, &str, &str)] = &[
    ("launcher", "Меню запуска", "\u{E5C3}"),
    ("taskbar", "Открытые окна", "\u{F088}"),
    ("workspaces", "Рабочие столы", "\u{E53B}"),
    ("separator", "Разделитель", "\u{E15B}"),
    ("spacer", "Растяжка", "\u{E8D4}"),
    ("clock", "Часы", "\u{E8B5}"),
    ("tray", "Системный лоток", "\u{E5CF}"),
    ("keyboard", "Раскладка", "\u{E312}"),
    ("volume", "Громкость", "\u{E050}"),
    ("network", "Сеть", "\u{E63E}"),
    ("battery", "Батарея", "\u{E1A4}"),
    ("notifications", "Уведомления", "\u{E7F4}"),
    ("show-desktop", "Показать рабочий стол", "\u{E30C}"),
    ("window-title", "Заголовок окна", "\u{F088}"),
    ("window-buttons", "Кнопки окна", "\u{E5CD}"),
    ("appmenu", "Глобальное меню", "\u{E5D2}"),
    ("layout", "Раскладка окон", "\u{E871}"),
    ("cpu", "Процессор", "\u{E30D}"),
    ("memory", "Память", "\u{E322}"),
    ("power", "Питание", "\u{E8AC}"),
];

pub fn applet_label(kind: &str) -> String {
    APPLET_KINDS.iter().find(|k| k.0 == kind).map(|k| k.1.to_string()).unwrap_or_else(|| kind.to_string())
}

/// Значки для разделов (глифы Material Icons).
const GROUP_GLYPHS: &[&str] = &[
    "\u{E86F}", // code
    "\u{E30A}", // computer
    "\u{E3AE}", // brush
    "\u{E405}", // music
    "\u{E04A}", // movie
    "\u{E338}", // games
    "\u{E80B}", // public
    "\u{E0BE}", // mail
    "\u{E8F9}", // work
    "\u{EA5F}", // calculate
    "\u{E869}", // build
    "\u{E838}", // star
];

/// Окно «Добавить на панель/док».
pub fn add_view(ctx: ShellCtx, panel: usize) -> impl Widget {
    const TABS: [&str; 4] = ["Приложение", "Раздел", "Папка", "Апплет"];
    let tab = use_signal(0usize);
    let mut bar = Row::new().gap(4.0).class("add-tabs");
    for (i, t) in TABS.iter().enumerate() {
        bar = bar.child(move || {
            let cls = if tab.get() == i { "add-tab add-tab-active" } else { "add-tab" };
            InputArea::new(boxed(cls, Text::new(*t).class("add-tab-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    tab.set(i);
                }
            })
        });
    }
    let dock = ctx.cfg().panels.get(panel).is_some_and(|p| p.is_dock());
    Column::new()
        .gap(10.0)
        .child(Text::new(if dock { "Добавить на док" } else { "Добавить на панель" }).class("popup-title"))
        .child(bar)
        .child(crate::ui::rx(move || -> Box<dyn Widget> {
            match tab.get() {
                0 => Box::new(add_app(panel)),
                1 => Box::new(group_form(panel, None)),
                2 => Box::new(folder_form(panel, None)),
                _ => Box::new(add_applet(panel)),
            }
        }))
}

/// Список приложений с поиском: клик — значок на панель.
fn app_picker(on_pick: impl Fn(String) + Clone + Send + Sync + 'static, selected: Option<RwSignal<Vec<String>>>) -> impl Widget {
    let q = use_signal(String::new());
    Column::new()
        .gap(6.0)
        .child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(icon(mi::SEARCH).class("search-icon"))
                        .child(TextField::new().placeholder("Поиск приложений").autofocus(true).on_change(move |t| q.set(t.to_string())).class("search-field grow")),
                )
                .class("search-box"),
        )
        .child(DecoratedBox::new().style("height", syngui::mss::StyleValue::px(290.0)).child(
            ScrollView::new()
                .vertical()
                .child(move || {
                    let query = q.get().to_lowercase();
                    let sel = selected.map(|s| s.get()).unwrap_or_default();
                    let apps = crate::xdg::apps();
                    let mut list: Vec<&crate::xdg::DesktopEntry> = apps
                        .iter()
                        .filter(|e| !e.no_display)
                        .filter(|e| {
                            query.is_empty()
                                || e.name.to_lowercase().contains(&query)
                                || e.generic_name.to_lowercase().contains(&query)
                                || e.id.to_lowercase().contains(&query)
                        })
                        .collect();
                    list.sort_by_key(|e| e.name.to_lowercase());
                    let mut col = Column::new().gap(1.0);
                    for e in list.into_iter().take(200) {
                        let id = e.id.clone();
                        let picked = sel.contains(&id);
                        let pick = on_pick.clone();
                        let mut row = Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(launchers::icon_widget(&crate::xdg::lookup_icon(&e.icon), &None, "add-app-icon", 28.0))
                            .child(
                                Column::new()
                                    .gap(0.0)
                                    .class("grow")
                                    .child(Text::new(e.name.clone()).max_lines(1).class("menu-label"))
                                    .child(Text::new(if e.generic_name.is_empty() { e.id.clone() } else { e.generic_name.clone() }).max_lines(1).class("add-app-hint")),
                            );
                        if selected.is_some() {
                            row = row.child(icon(if picked { "\u{E834}" } else { "\u{E835}" }).class(if picked { "add-check add-check-on" } else { "add-check" }));
                        }
                        col = col.child(
                            InputArea::new(DecoratedBox::new().child(row).class(if picked { "menu-item add-app add-app-picked" } else { "menu-item add-app" }))
                                .pointer()
                                .on_click(move |b, _, _| {
                                    if b == MouseButton::Left {
                                        pick(id.clone());
                                    }
                                }),
                        );
                    }
                    col
                })
                .class("add-scroll"),
        ))
}

fn add_app(panel: usize) -> impl Widget {
    let added = use_signal(Vec::<String>::new());
    Column::new()
        .gap(6.0)
        .child(move || {
            let a = added.get();
            Text::new(if a.is_empty() {
                "Щёлкните приложение — его значок появится на панели.".to_string()
            } else {
                format!("Добавлено: {}", a.len())
            })
            .class("add-hint")
        })
        .child(app_picker(
            move |id| {
                let mut a = added.get_untracked();
                a.push(id.clone());
                added.set(a);
                add(panel, app_applet(&id));
            },
            None,
        ))
}

fn add_applet(panel: usize) -> impl Widget {
    let mut flex = Flex::new().wrap().gap(6.0).class("add-applets");
    for (kind, label, glyph) in APPLET_KINDS {
        let k = kind.to_string();
        flex = flex.child(
            InputArea::new(
                DecoratedBox::new()
                    .child(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("add-applet-icon")).child(Text::new(*label).max_lines(2).class("add-applet-label")))
                    .class("add-applet"),
            )
            .pointer()
            .on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    add(panel, Applet::new(&k));
                    ShellCtx::get().close_popup();
                }
            }),
        );
    }
    flex
}

fn labeled(label: &str, w: impl Widget + 'static) -> impl Widget {
    Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(label.to_string()).class("form-label")).child(w)
}

fn choice(options: &'static [(&'static str, &'static str)], sig: RwSignal<String>) -> impl Widget {
    let mut row = Row::new().gap(4.0).class("form-choice");
    for (v, l) in options {
        row = row.child(move || {
            let on = sig.get() == *v;
            InputArea::new(boxed(if on { "chip chip-on" } else { "chip" }, Text::new(*l).class("chip-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    sig.set(v.to_string());
                }
            })
        });
    }
    row
}

const VIEWS: &[(&str, &str)] = &[("grid", "Сетка"), ("list", "Список"), ("fan", "Веер")];
const OPENS: &[(&str, &str)] = &[("click", "По клику"), ("hover", "Наведением")];

/// Форма раздела: новый (`index` = None) или правка существующего.
pub fn group_form(panel: usize, index: Option<usize>) -> impl Widget {
    let cur = index.and_then(|i| applets_of(panel).get(i).cloned()).unwrap_or_else(|| Applet::new("group"));
    let name = use_signal(cur.str("name").unwrap_or("").to_string());
    let glyph = use_signal(cur.str("icon").unwrap_or("").to_string());
    let view = use_signal(cur.str_or("view", "grid").to_string());
    let open = use_signal(cur.str_or("open", "click").to_string());
    let items = use_signal(cur.strings("items"));
    let mut glyphs = Row::new().gap(4.0).class("form-glyphs");
    glyphs = glyphs.child(move || {
        let on = glyph.get().is_empty();
        InputArea::new(boxed(if on { "glyph-pick glyph-pick-on" } else { "glyph-pick" }, icon("\u{E8F0}"))).pointer().on_click(move |_, _, _| glyph.set(String::new()))
    });
    for g in GROUP_GLYPHS {
        glyphs = glyphs.child(move || {
            let on = glyph.get() == *g;
            InputArea::new(boxed(if on { "glyph-pick glyph-pick-on" } else { "glyph-pick" }, icon(g))).pointer().on_click(move |_, _, _| glyph.set(g.to_string()))
        });
    }
    let base = cur.clone();
    Column::new()
        .gap(8.0)
        .class("form")
        .child(labeled("Название", TextField::with_text(name.get_untracked()).placeholder("Разработка").on_change(move |t| name.set(t.to_string())).class("form-field grow")))
        .child(labeled("Значок", glyphs))
        .child(labeled("Вид", choice(VIEWS, view)))
        .child(labeled("Открывать", choice(OPENS, open)))
        .child(Text::new("Приложения раздела:").class("form-label"))
        .child(app_picker(
            move |id| {
                let mut v = items.get_untracked();
                if let Some(p) = v.iter().position(|x| *x == id) {
                    v.remove(p);
                } else {
                    v.push(id);
                }
                items.set(v);
            },
            Some(items),
        ))
        .child(form_buttons(
            panel,
            index,
            move || {
                let mut a = base.clone();
                a.kind = "group".into();
                set_opt(&mut a, "name", name.get_untracked());
                set_opt(&mut a, "icon", glyph.get_untracked());
                a.options.insert("view".into(), toml::Value::String(view.get_untracked()));
                a.options.insert("open".into(), toml::Value::String(open.get_untracked()));
                a.options.insert("items".into(), toml::Value::Array(items.get_untracked().into_iter().map(toml::Value::String).collect()));
                a
            },
            if index.is_some() { "Сохранить" } else { "Создать раздел" },
        ))
}

/// Форма папки.
pub fn folder_form(panel: usize, index: Option<usize>) -> impl Widget {
    let cur = index.and_then(|i| applets_of(panel).get(i).cloned()).unwrap_or_else(|| Applet::new("folder"));
    let path = use_signal(cur.str_or("path", "xdg:DOWNLOAD").to_string());
    let name = use_signal(cur.str("name").unwrap_or("").to_string());
    let view = use_signal(cur.str_or("view", "grid").to_string());
    let open = use_signal(cur.str_or("open", "click").to_string());
    // Поле пути пересоздаётся (новым текстом) только при выборе готовой
    // папки — не на каждую букву, иначе теряло бы фокус.
    let path_ver = use_signal(0u32);
    let quick: &[(&str, &str)] = &[
        ("~", "Домашняя"),
        ("xdg:DOWNLOAD", "Загрузки"),
        ("xdg:DOCUMENTS", "Документы"),
        ("xdg:PICTURES", "Изображения"),
        ("xdg:DESKTOP", "Рабочий стол"),
        ("trash:", "Корзина"),
    ];
    let mut chips = Flex::new().wrap().gap(4.0).class("form-choice");
    for (p, l) in quick {
        let (p, l) = (p.to_string(), l.to_string());
        chips = chips.child(move || {
            let on = path.get() == p;
            let (p2, l2) = (p.clone(), l.clone());
            InputArea::new(boxed(if on { "chip chip-on" } else { "chip" }, Text::new(l.clone()).class("chip-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    path.set(p2.clone());
                    path_ver.set(path_ver.get_untracked() + 1);
                    if name.get_untracked().is_empty() || quick_name(&name.get_untracked()) {
                        name.set(l2.clone());
                    }
                }
            })
        });
    }
    let base = cur.clone();
    Column::new()
        .gap(8.0)
        .class("form")
        .child(labeled(
            "Путь",
            crate::ui::rx(move || {
                let _ = path_ver.get();
                Box::new(TextField::with_text(path.get_untracked()).placeholder("~/Проекты").on_change(move |t| path.set(t.to_string())).class("form-field grow"))
            }),
        ))
        .child(chips)
        .child(labeled("Название", TextField::with_text(name.get_untracked()).placeholder("По имени папки").on_change(move |t| name.set(t.to_string())).class("form-field grow")))
        .child(labeled("Вид", choice(VIEWS, view)))
        .child(labeled("Открывать", choice(OPENS, open)))
        .child(form_buttons(
            panel,
            index,
            move || {
                let mut a = base.clone();
                a.kind = "folder".into();
                a.options.insert("path".into(), toml::Value::String(path.get_untracked()));
                set_opt(&mut a, "name", name.get_untracked());
                a.options.insert("view".into(), toml::Value::String(view.get_untracked()));
                a.options.insert("open".into(), toml::Value::String(open.get_untracked()));
                a
            },
            if index.is_some() { "Сохранить" } else { "Добавить папку" },
        ))
}

fn quick_name(n: &str) -> bool {
    matches!(n, "Домашняя" | "Загрузки" | "Документы" | "Изображения" | "Рабочий стол" | "Корзина")
}

/// Форма значка приложения: подпись, значок, команда.
pub fn app_form(panel: usize, index: usize) -> impl Widget {
    let cur = applets_of(panel).get(index).cloned().unwrap_or_else(|| Applet::new("app"));
    let l = Launchable::from_applet(&cur);
    let name = use_signal(cur.str("name").unwrap_or("").to_string());
    let icon_name = use_signal(cur.str("icon").unwrap_or("").to_string());
    let command = use_signal(cur.str("command").unwrap_or("").to_string());
    let base = cur.clone();
    Column::new()
        .gap(8.0)
        .class("form")
        .child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(launchers::icon_widget(&l.icon, &l.glyph, "form-app-icon", 48.0))
                .child(Column::new().gap(0.0).child(Text::new(l.name.clone()).class("popup-title")).child(Text::new(l.app_id.clone()).class("add-app-hint"))),
        )
        .child(labeled("Подпись", TextField::with_text(name.get_untracked()).placeholder(l.name.clone()).on_change(move |t| name.set(t.to_string())).class("form-field grow")))
        .child(labeled("Значок", TextField::with_text(icon_name.get_untracked()).placeholder("имя из темы значков или путь").on_change(move |t| icon_name.set(t.to_string())).class("form-field grow")))
        .child(labeled("Команда", TextField::with_text(command.get_untracked()).placeholder(l.command.clone()).on_change(move |t| command.set(t.to_string())).class("form-field grow")))
        .child(form_buttons(
            panel,
            Some(index),
            move || {
                let mut a = base.clone();
                set_opt(&mut a, "name", name.get_untracked());
                set_opt(&mut a, "icon", icon_name.get_untracked());
                set_opt(&mut a, "command", command.get_untracked());
                a
            },
            "Сохранить",
        ))
}

fn set_opt(a: &mut Applet, key: &str, v: String) {
    if v.trim().is_empty() {
        a.options.remove(key);
    } else {
        a.options.insert(key.into(), toml::Value::String(v.trim().to_string()));
    }
}

fn form_buttons(panel: usize, index: Option<usize>, build: impl Fn() -> Applet + Send + Sync + 'static, ok: &str) -> impl Widget {
    let mut row = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).class("form-buttons");
    if let Some(i) = index {
        row = row.child(
            InputArea::new(boxed("btn btn-danger", Text::new("Удалить").class("btn-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    remove(panel, i);
                    ShellCtx::get().close_popup();
                }
            }),
        );
    }
    row.child(
        InputArea::new(boxed("btn btn-primary", Text::new(ok.to_string()).class("btn-label"))).pointer().on_click(move |b, _, _| {
            if b != MouseButton::Left {
                return;
            }
            let a = build();
            match index {
                Some(i) => replace(panel, i, a),
                None => add(panel, a),
            }
            ShellCtx::get().close_popup();
        }),
    )
}

/// Окно настройки значка (клик по значку в режиме редактирования).
pub fn edit_view(ctx: ShellCtx, panel: usize, index: usize) -> Box<dyn Widget> {
    let kind = ctx.cfg().panels.get(panel).and_then(|p| p.applets.get(index)).map(|a| a.kind.clone()).unwrap_or_default();
    let body: Box<dyn Widget> = match kind.as_str() {
        "app" => Box::new(app_form(panel, index)),
        "group" => Box::new(group_form(panel, Some(index))),
        "folder" => Box::new(folder_form(panel, Some(index))),
        other => Box::new(
            Column::new()
                .gap(8.0)
                .child(Text::new(applet_label(other)).class("popup-title"))
                .child(Text::new("Настройки апплета — в «Параметрах системы» → «Панели».").class("add-hint"))
                .child(form_buttons(panel, Some(index), move || applets_of(panel).get(index).cloned().unwrap_or_else(|| Applet::new("spacer")), "Готово")),
        ),
    };
    let title = match kind.as_str() {
        "app" => "Значок приложения",
        "group" => "Раздел",
        "folder" => "Папка",
        _ => "Апплет",
    };
    Box::new(Column::new().gap(10.0).child(Text::new(title).class("popup-title")).child(body))
}
