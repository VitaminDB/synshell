//! name: Монтаж
//! description: Классический монтаж: медиатека и эффекты слева, просмотр в центре, инспектор справа, таймлайн снизу
//! order: 10
// Вид редактора: tabs([панели], "имя", активная), row(слева, справа, доля), column(сверху, снизу, доля).
fn layout() {
    let left = tabs(["media", "templates", "effects", "luts"]);
    let right = tabs(["inspector", "ai"]);
    column(
        row(row(left, tabs(["viewer", "graph"], "viewer"), 0.26), right, 0.74),
        tabs(["timeline", "curves"]),
        0.58
    )
}
