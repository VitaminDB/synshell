//! name: Телефон
//! description: Для телефона: просмотр сверху, таймлайн посередине, остальные панели — вкладками снизу
//! order: 5
fn layout() {
    column(
        tabs(["viewer"], "viewer"),
        column(
            tabs(["timeline", "curves"]),
            tabs(["media", "inspector", "ai", "templates", "effects", "color", "luts", "layers"], "tools"),
            0.48
        ),
        0.34
    )
}
