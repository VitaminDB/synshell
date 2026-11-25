//! name: ИИ-монтаж
//! description: Широкая панель ИИ-агента справа для монтажа и правок текстом
//! order: 40
fn layout() {
    row(
        column(
            row(tabs(["media", "templates", "effects", "luts"]), tabs(["viewer"], "viewer"), 0.32),
            tabs(["timeline", "curves"]),
            0.58
        ),
        tabs(["ai", "inspector"]),
        0.68
    )
}
