//! Режимы окон телефона (`[mobile] mode`): `pages` — каждое приложение во
//! весь экран, листание вбок, «страница 0» — домашний экран оболочки;
//! `tiles` — окна друг под другом (раскладка `rows`); `free` — свободные
//! окна (виртуальный стол — `camera`).
//!
//! Страница — корневое окно приложения вместе с его диалогами
//! (transient-дети остаются на странице родителя). Порядок страниц —
//! порядок появления окон. Переход между страницами рисуется сдвигом, как
//! переключение столов (`render.rs`), и может вестись пальцем
//! (`page_drag_*`).

use std::time::Duration;

use smithay::utils::{Logical, Point};
use synshell_common::action::{LayoutKind, MobileMode, PageTarget};
use synshell_common::config::FormFactor;
use synshell_common::ipc::{Event, MobileInfo};

use super::{Wm, WindowId};
use crate::anim::{Animation, Curve};
use crate::state::State;

/// Переход между страницами: откуда, куда, направление (+1 — новая справа).
pub struct PageTransition {
    pub from: Option<WindowId>,
    pub to: Option<WindowId>,
    pub dir: i32,
    /// Анимация 0→1 (после отпускания пальца — от текущей доли).
    pub anim: Animation,
}

/// Перетаскивание страниц пальцем: сдвиг в логических px (<0 — влево).
pub struct PageDrag {
    pub from: Option<WindowId>,
    pub dx: f64,
}

#[derive(Default)]
pub struct MobileState {
    /// Телефонный форм-фактор: режимы действуют.
    pub enabled: bool,
    pub mode: MobileMode,
    /// Корневое окно показанной страницы; `None` — домашний экран.
    pub page: Option<WindowId>,
    pub transition: Option<PageTransition>,
    pub drag: Option<PageDrag>,
    /// Сдвиг виртуального стола (`free`), логические px.
    pub camera: Point<f64, Logical>,
    /// Последнее разосланное состояние.
    pub last_sent: Option<MobileInfo>,
}

impl MobileState {
    pub fn pages_mode(&self) -> bool {
        self.enabled && self.mode == MobileMode::Pages
    }
}

impl Wm {
    /// Корневое окно: поднимаемся по родителям xdg (диалоги — к окну).
    pub fn root_of(&self, id: WindowId) -> WindowId {
        let mut cur = id;
        for _ in 0..8 {
            let parent = self
                .get(cur)
                .and_then(|m| m.window.toplevel().and_then(|t| t.parent()))
                .and_then(|p| self.id_by_surface(&p));
            match parent {
                Some(p) if p != cur => cur = p,
                _ => break,
            }
        }
        cur
    }

    /// Страницы по порядку: корневые показанные окна активного стола.
    pub fn page_order(&self) -> Vec<WindowId> {
        let mut v: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|m| m.mapped && !m.override_redirect && m.on_workspace(self.active))
            .map(|m| m.id)
            .filter(|id| self.root_of(*id) == *id)
            .collect();
        v.sort_unstable();
        v
    }

    /// Окно `id` на странице `page`.
    pub fn in_page(&self, id: WindowId, page: Option<WindowId>) -> bool {
        page.is_some_and(|p| self.root_of(id) == p)
    }

    /// Соседняя страница для `page` в сторону сдвига (`dx < 0` — следующая):
    /// `Some(None)` — домашний экран, `None` — дальше некуда.
    pub fn page_neighbor_of(&self, page: Option<WindowId>, dx: f64) -> Option<Option<WindowId>> {
        let order = self.page_order();
        let cur = page.and_then(|p| order.iter().position(|x| *x == p).map(|i| i + 1)).unwrap_or(0);
        let n = if dx < 0.0 { cur + 1 } else { cur.checked_sub(1)? };
        if n > order.len() {
            return None;
        }
        Some(if n == 0 { None } else { Some(order[n - 1]) })
    }

    /// Окна страницы по порядку стопки (для рисования перехода).
    pub fn ids_on_page(&self, page: Option<WindowId>) -> Vec<WindowId> {
        self.stack
            .iter()
            .copied()
            .filter(|id| self.get(*id).is_some_and(|w| w.mapped && w.on_workspace(self.active)) && self.in_page(*id, page))
            .collect()
    }
}

impl State {
    pub fn mobile_info(&self) -> MobileInfo {
        let wm = &self.core.wm;
        MobileInfo {
            mode: wm.mobile.mode,
            page: wm.mobile.page,
            pages: wm.page_order(),
            camera: [wm.mobile.camera.x.round() as i32, wm.mobile.camera.y.round() as i32],
        }
    }

    /// Разослать состояние, если изменилось.
    pub fn broadcast_mobile(&mut self) {
        if !self.core.wm.mobile.enabled {
            return;
        }
        let info = self.mobile_info();
        if self.core.wm.mobile.last_sent.as_ref() != Some(&info) {
            self.core.wm.mobile.last_sent = Some(info.clone());
            self.core.ipc.broadcast(&Event::MobileChanged { mobile: info });
        }
    }

    /// Начальное состояние по форм-фактору и конфигу.
    pub fn init_mobile(&mut self) {
        let enabled = self.core.form_factor == FormFactor::Phone;
        self.core.wm.mobile.enabled = enabled;
        if enabled {
            let mode = self.core.config.mobile.mode;
            self.core.wm.mobile.mode = MobileMode::Tiles; // любой отличный — чтобы применить
            self.set_mobile_mode(mode, false);
        }
    }

    fn mode_layout(mode: MobileMode) -> LayoutKind {
        match mode {
            MobileMode::Pages => LayoutKind::Monocle,
            MobileMode::Tiles => LayoutKind::Rows,
            MobileMode::Free => LayoutKind::Floating,
        }
    }

    /// Сменить режим окон. `remember` — записать в config.toml (если
    /// `[mobile] remember_mode`).
    pub fn set_mobile_mode(&mut self, mode: MobileMode, remember: bool) {
        if !self.core.wm.mobile.enabled {
            tracing::debug!("режимы окон — только на телефоне");
            return;
        }
        let prev = self.core.wm.mobile.mode;
        if prev == mode {
            return;
        }
        let ids: Vec<WindowId> = self.core.wm.windows.iter().filter(|m| m.mapped).map(|m| m.id).collect();
        if prev == MobileMode::Free {
            for id in &ids {
                self.remember_float(*id);
            }
            self.core.wm.mobile.camera = Point::from((0.0, 0.0));
        }
        let layout = Self::mode_layout(mode);
        for i in 0..self.core.wm.count() {
            self.core.wm.workspace_mut(i).layout = layout;
        }
        self.core.wm.mobile.mode = mode;
        self.core.wm.mobile.transition = None;
        self.core.wm.mobile.drag = None;
        if mode == MobileMode::Free {
            for id in &ids {
                let (max, fs) = self.core.wm.get(*id).map(|m| (m.maximized, m.fullscreen)).unwrap_or((false, false));
                if !max && !fs {
                    self.restore_float(*id);
                }
            }
        }
        if mode == MobileMode::Pages {
            // Страница — окно в фокусе; без него — домашний экран.
            self.core.wm.mobile.page = self.core.wm.focused.map(|f| self.core.wm.root_of(f));
        }
        if (prev == MobileMode::Free) != (mode == MobileMode::Free) {
            self.apply_mode_decorations();
        }
        tracing::info!(mode = mode.as_str(), "режим окон");
        self.relayout();
        self.core.ipc.broadcast(&Event::ShellCommand { command: format!("mode {}", mode.as_str()) });
        if remember && self.core.config.mobile.remember_mode {
            let v = toml_edit::Value::from(mode.as_str());
            if let Err(e) = synshell_common::config_edit::set_value(&["mobile", "mode"], v) {
                tracing::warn!(?e, "не удалось запомнить режим окон");
            }
        }
        self.broadcast_mobile();
        self.core.ipc_dirty = true;
        self.core.queue_redraw_all();
    }

    /// Рамки по режиму: в свободном — серверные (ручка для пальца), в
    /// остальных — как настроено (`[windows] decorations`, правила окон).
    fn apply_mode_decorations(&mut self) {
        use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        let want_server = self.core.default_decoration_mode() == Mode::ServerSide;
        let ids: Vec<WindowId> = self.core.wm.windows.iter().map(|m| m.id).collect();
        for id in ids {
            let Some(m) = self.core.wm.get_mut(id) else { continue };
            let rule = None::<bool>;
            let ssd = rule.unwrap_or(want_server);
            m.ssd = ssd;
            if let Some(t) = m.window.toplevel() {
                t.with_pending_state(|s| s.decoration_mode = Some(if ssd { Mode::ServerSide } else { Mode::ClientSide }));
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
        }
    }

    fn page_anim(&self) -> Duration {
        if !self.core.config.animations.pages {
            return Duration::ZERO;
        }
        crate::anim::duration(&self.core.config.animations, 240)
    }

    /// Показать страницу `to` (`None` — домашний экран) со сдвигом.
    pub fn go_to_page(&mut self, to: Option<WindowId>, animate: bool) {
        if !self.core.wm.mobile.pages_mode() {
            return;
        }
        let from = self.core.wm.mobile.page;
        if from == to {
            return;
        }
        let order = self.core.wm.page_order();
        let pos = |p: Option<WindowId>| p.and_then(|p| order.iter().position(|x| *x == p)).map(|i| i as i64 + 1).unwrap_or(0);
        let dir = if pos(to) >= pos(from) { 1 } else { -1 };
        let dur = self.page_anim();
        self.core.wm.mobile.page = to;
        self.core.wm.mobile.transition = (animate && !dur.is_zero()).then(|| PageTransition {
            from,
            to,
            dir,
            anim: Animation::new(0.0, 1.0, dur, Curve::EaseOutCubic),
        });
        if to.is_none() && self.core.wm.focused.is_some() {
            self.focus_window(None);
        }
        self.relayout();
        self.broadcast_mobile();
        self.core.queue_redraw_all();
    }

    /// Действие `page home|next|prev|N`.
    pub fn page_action(&mut self, t: PageTarget) {
        if !self.core.wm.mobile.pages_mode() {
            // В других режимах «домой» — свернуть окна (рабочий стол).
            if t == PageTarget::Home {
                for id in self.core.wm.visible_ids() {
                    self.minimize(id);
                }
            }
            return;
        }
        let order = self.core.wm.page_order();
        let cur = self.core.wm.mobile.page.and_then(|p| order.iter().position(|x| *x == p).map(|i| i + 1)).unwrap_or(0);
        let target = match t {
            PageTarget::Home => 0,
            PageTarget::Next => (cur + 1).min(order.len()),
            PageTarget::Prev => cur.saturating_sub(1),
            PageTarget::Index(i) => (i as usize).min(order.len()),
        };
        match target {
            0 => self.go_to_page(None, true),
            n => {
                let id = order[n - 1];
                self.focus_window(Some(id));
            }
        }
    }

    /// Фокус ушёл к окну: в режиме страниц — показать его страницу.
    pub fn mobile_focus_changed(&mut self, id: Option<WindowId>) {
        if !self.core.wm.mobile.pages_mode() {
            return;
        }
        if let Some(id) = id {
            let root = self.core.wm.root_of(id);
            if self.core.wm.mobile.page != Some(root) {
                self.go_to_page(Some(root), true);
            }
        }
    }

    /// Страница закрылась — на соседнюю (или домой).
    pub fn mobile_fix_page(&mut self) {
        let m = &self.core.wm.mobile;
        if !m.pages_mode() {
            return;
        }
        let order = self.core.wm.page_order();
        if let Some(p) = m.page {
            if !order.contains(&p) {
                let next = self.core.wm.focused.map(|f| self.core.wm.root_of(f)).filter(|r| order.contains(r));
                self.core.wm.mobile.page = None;
                match next {
                    Some(n) => self.go_to_page(Some(n), true),
                    None => {
                        self.core.wm.mobile.transition = None;
                        self.broadcast_mobile();
                    }
                }
            }
        }
        self.broadcast_mobile();
    }

    // ─── Перелистывание пальцем ─────────────────────────────────────────────

    pub fn page_drag_begin(&mut self) {
        if !self.core.wm.mobile.pages_mode() {
            return;
        }
        let from = self.core.wm.mobile.page;
        self.core.wm.mobile.transition = None;
        self.core.wm.mobile.drag = Some(PageDrag { from, dx: 0.0 });
    }

    /// Соседняя страница в сторону сдвига (`dx < 0` — следующая).
    pub fn page_neighbor(&self, dx: f64) -> Option<Option<WindowId>> {
        self.core.wm.page_neighbor_of(self.core.wm.mobile.page, dx)
    }

    pub fn page_drag_update(&mut self, dx: f64) {
        let has_neighbor = self.page_neighbor(dx).is_some();
        if let Some(d) = self.core.wm.mobile.drag.as_mut() {
            // За крайней страницей — сопротивление.
            d.dx = if has_neighbor { dx } else { dx * 0.25 };
        }
        self.core.queue_redraw_all();
    }

    /// Отпустили: пролистать, если протащили больше трети или бросили
    /// быстрее `velocity` (px/мс), иначе вернуть.
    pub fn page_drag_end(&mut self, velocity: f64, width: f64) {
        let Some(drag) = self.core.wm.mobile.drag.take() else { return };
        let threshold = self.core.config.gestures.velocity as f64;
        let go = drag.dx.abs() > width / 3.0 || (velocity.abs() > threshold && velocity.signum() == drag.dx.signum());
        let target = if go { self.page_neighbor(drag.dx) } else { None };
        let dur = self.page_anim().max(Duration::from_millis(120));
        let done = (drag.dx.abs() / width.max(1.0)).clamp(0.0, 0.95);
        match target {
            Some(to) => {
                let dir = if drag.dx < 0.0 { 1 } else { -1 };
                self.core.wm.mobile.page = to;
                // Продолжить с того места, где отпустили палец.
                self.core.wm.mobile.transition = Some(PageTransition {
                    from: drag.from,
                    to,
                    dir,
                    anim: Animation::new(done, 1.0, dur.mul_f64(1.0 - done), Curve::EaseOutCubic),
                });
                match to {
                    Some(id) => self.focus_window(Some(id)),
                    None => self.focus_window(None),
                }
                self.relayout();
            }
            None => {
                // Вернуть: соседняя страница уезжает обратно.
                if drag.dx.abs() > 1.0 {
                    if let Some(nb) = self.page_neighbor(drag.dx) {
                        let dir = if drag.dx < 0.0 { 1 } else { -1 };
                        self.core.wm.mobile.transition = Some(PageTransition {
                            from: nb,
                            to: drag.from,
                            dir: -dir,
                            anim: Animation::new(1.0 - done, 1.0, dur.mul_f64(1.0 - (1.0 - done)).max(Duration::from_millis(80)), Curve::EaseOutCubic),
                        });
                    }
                }
            }
        }
        self.broadcast_mobile();
        self.core.queue_redraw_all();
    }
}
