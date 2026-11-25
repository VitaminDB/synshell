//! name: Цвет
//! description: Цветокоррекция: большой просмотр с «до/после», колёса и кривые снизу, LUT слева, инспектор справа
//! order: 38
fn layout() {
    column(
        row(
            tabs(["luts", "media", "effects"]),
            row(tabs(["viewer"], "viewer"), tabs(["inspector", "ai"]), 0.74),
            0.2
        ),
        row(tabs(["color"]), tabs(["timeline", "curves"]), 0.6),
        0.56
    )
}
