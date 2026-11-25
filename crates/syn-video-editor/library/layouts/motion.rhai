//! name: Моушн
//! description: Моушн-графика: дерево слоёв слева, просмотр и инспектор сверху, таймлайн и кривые рядом снизу
//! order: 20
fn layout() {
    column(
        row(
            row(tabs(["layers", "templates", "effects", "luts", "media"]), tabs(["viewer"], "viewer"), 0.26),
            tabs(["inspector", "ai"]),
            0.74
        ),
        row(tabs(["timeline"]), tabs(["curves"]), 0.56),
        0.56
    )
}
