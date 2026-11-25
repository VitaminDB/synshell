//! name: Граф
//! description: Узловой граф эффектов в центре, просмотр и инспектор справа
//! order: 30
fn layout() {
    column(
        row(
            tabs(["media", "templates", "effects", "luts"]),
            row(tabs(["graph"], "graph"), tabs(["viewer", "inspector", "ai"]), 0.7),
            0.2
        ),
        tabs(["timeline", "curves"]),
        0.7
    )
}
