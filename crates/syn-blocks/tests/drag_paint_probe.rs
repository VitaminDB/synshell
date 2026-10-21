use syn_blocks::game::{Board, Family, Piece};
use syn_blocks::persist::Stats;
use syn_blocks::ui::layout;
use syn_blocks::{GameCtx, GameView};
use syngui::prelude::*;
use syngui::render::display_list::DrawCommand;
use syngui::testing::TestHarness;

#[test]
fn dragged_piece_commands_are_emitted_after_preview() {
    std::env::set_var("SYN_BLOCKS_DATA", std::env::temp_dir().join("syn_blocks_probe"));
    syngui::signal::allow_signal_reads_on_this_thread();
    let ctx = GameCtx::new(1, Stats::default());
    let mut g = ctx.game.get_untracked();
    g.board = Board::new();
    for c in 0..6 {
        g.board.set(7, c, 2);
    }
    g.tray = [Some(Piece::of(Family::I2)), None, None];
    ctx.game.set(g);
    provide_context(ctx);
    let mut h = TestHarness::new(Box::new(GameView));
    h.layout(460.0, 820.0);
    let geo = layout::compute(Rect::new(Point::new(0.0, 0.0), Size::new(460.0, 820.0)));
    let o = geo.piece_origin_in_slot(0, &Family::I2.base());
    let from = Point::new(o.x + geo.tray_cell * 0.5, o.y + geo.tray_cell * 0.5);
    let to = Point::new(geo.grid.x + 6.0 * geo.cell + geo.tray_cell * 0.5, geo.grid.y + 7.0 * geo.cell + geo.tray_cell * 0.5);
    h.send_event(&Event::MouseDown { button: MouseButton::Left, position: from });
    h.send_event(&Event::MouseMove(Point::new(from.x + 8.0, from.y)));
    h.send_event(&Event::MouseMove(to));
    let list = h.paint();
    let cmds = list.commands();
    let cell = geo.cube_rect(7, 6);
    let mut i_first_canvas_at_target = None;
    let mut i_last_rect_at_target = None;
    for (i, c) in cmds.iter().enumerate() {
        match c {
            DrawCommand::Canvas { vertices, .. } => {
                let inside = vertices.iter().filter(|v| v.position[0] >= cell.origin.x - 1.0 && v.position[0] <= cell.origin.x + cell.width() + 1.0 && v.position[1] >= cell.origin.y - 1.0 && v.position[1] <= cell.origin.y + cell.height() + 1.0).count();
                if inside > 3 && i_first_canvas_at_target.is_none() {
                    i_first_canvas_at_target = Some(i);
                }
            }
            DrawCommand::Rect { rect, color, .. } => {
                if rect.contains(Point::new(cell.origin.x + 2.0, cell.origin.y + 2.0)) && color.a < 0.99 {
                    i_last_rect_at_target = Some((i, *rect, *color));
                }
            }
            _ => {}
        }
    }
    eprintln!("commands={} canvas@target={:?} last translucent rect@target={:?}", cmds.len(), i_first_canvas_at_target, i_last_rect_at_target);
    let canvases = cmds.iter().filter(|c| matches!(c, DrawCommand::Canvas { .. })).count();
    eprintln!("canvas commands: {canvases}");
    assert!(i_first_canvas_at_target.is_some(), "нет полигонов перетаскиваемого кубика в (7,6)");
}
