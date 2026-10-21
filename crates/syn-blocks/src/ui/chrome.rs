//! Своё окно игры (CSD): заголовок со знаком и названием, перетаскивание, кнопка «во весь экран»,
//! системные кнопки окна, изменение размера за край. Во весь экран заголовка нет — играют на всём
//! экране; выйти — кнопкой на стартовой странице и в меню игры, F11 или Esc на стартовой странице.
//! Только для настольной сборки: на Android окно всегда во весь экран.

use super::{theme, toggle_fullscreen, GameCtx, GameView};
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;

type W = Box<dyn Widget>;

/// Запущено на телефоне (или в игровом режиме оболочки synshell): по умолчанию — во весь экран,
/// без рамки за краем (её пальцем не поймать, а края нужны игре).
pub fn touch_device() -> bool {
    matches!(std::env::var("SYNSHELL_FORM_FACTOR").as_deref(), Ok("phone" | "game"))
}

/// Полноэкранный режим при запуске: выбор пользователя, иначе на телефоне — да.
pub fn start_fullscreen(ctx: GameCtx) -> bool {
    ctx.stats.get_untracked().fullscreen.unwrap_or_else(touch_device)
}

/// Стили рамки и заголовка (цвета — из темы игры).
pub fn mss() -> String {
    format!(
        r#"
.window-frame {{ border-radius: 14px; border: 1px solid #ffffff2e; background-color: {bg}; }}
.window-frame.flat {{ border-radius: 0px; border-width: 0px; }}
.window-frame:window-maximized, .window-frame:window-fullscreen {{ border-radius: 0px; border-width: 0px; }}
.game {{ flex-grow: 1; }}
.titlebar {{ height: 42px; background: linear-gradient(180deg, {panel_hi}, {panel}); border-bottom: 1px solid #ffffff1a; }}
.tb-left {{ height: 42px; padding: 0px 8px 0px 14px; align-items: center; }}
.tb-drag {{ height: 42px; flex-grow: 1; min-width: 40px; }}
.tb-tools {{ height: 42px; padding: 0px 4px; align-items: center; }}
.tb-title {{ font-size: 14px; font-weight: 800; color: {gold}; }}
.bm {{ width: 8px; height: 8px; border-radius: 2px; }}
.bm-a {{ background-color: #5b8cff; }}
.bm-b {{ background-color: #ff8a3d; }}
.bm-c {{ background-color: #43c463; }}
.bm-d {{ background-color: #ffc93c; }}
.tb-btn {{ width: 34px; height: 30px; padding: 0px; border-radius: 8px; border-width: 0px; background-color: #00000000; icon-size: 20px; icon-color: {dim}; color: {dim}; justify-content: center; align-items: center; }}
.tb-btn:hover {{ background-color: #ffffff1f; icon-color: #ffffff; color: #ffffff; }}
.window-controls {{ height: 42px; padding: 0px 6px 0px 2px; align-items: center; }}
SystemWindowControls {{ color: #ffffff; background-color: #ffffff24; }}
"#,
        bg = theme::BG_BOTTOM,
        panel = theme::PANEL,
        panel_hi = theme::CARD,
        gold = theme::GOLD,
        dim = theme::TEXT_DIM,
    )
}

/// Знак игры: четыре кубика 2×2.
fn brand_mark() -> W {
    let cube = |c: &'static str| DecoratedBox::new().class(&format!("bm {c}"));
    Box::new(
        Column::new()
            .gap(2.0)
            .child(Row::new().gap(2.0).child(cube("bm-a")).child(cube("bm-b")))
            .child(Row::new().gap(2.0).child(cube("bm-c")).child(cube("bm-d"))),
    )
}

fn titlebar(ctx: GameCtx) -> W {
    let fs_button = Tooltip::new(
        Button::new("").icon(theme::ICON_FULLSCREEN).class("tb-btn").on_click(move || toggle_fullscreen(ctx)),
        "Во весь экран (F11)",
    )
    .delay_ms(500);
    let controls = Reactive::new(move || -> Vec<W> {
        let w = ctx.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    Box::new(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    DecoratedBox::new()
                        .class("tb-left")
                        .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(brand_mark()).child(Text::new("Syn Blocks").class("tb-title"))),
                )
                .child(DecoratedBox::new().class("tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().class("tb-drag"))))
                .child(DecoratedBox::new().class("tb-tools").child(fs_button))
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Окно: рамка, заголовок (кроме полноэкранного режима) и игра. Строение не меняется при
/// переключении режима — `GameView` не пересоздаётся (анимации и перетаскивание не сбрасываются).
pub fn root(ctx: GameCtx) -> W {
    let touch = touch_device();
    // заголовок перестраивается только при смене полноэкранного режима, а не фокуса
    let full = use_signal(ctx.window.get_untracked().fullscreen);
    create_effect(move || {
        let v = ctx.window.get().fullscreen;
        if full.get_untracked() != v {
            full.set(v);
        }
    });
    let bar = Reactive::new(move || -> Vec<W> { if full.get() { vec![] } else { vec![titlebar(ctx)] } });
    let window = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(bar)
        .child(DecoratedBox::new().class("game").child(GameView));
    let frame = DecoratedBox::new().class(if touch { "window-frame flat" } else { "window-frame" }).clip(true).child(window);
    Box::new(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(!touch).child(frame))
}
