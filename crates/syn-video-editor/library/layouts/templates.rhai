//! name: Шаблоны
//! description: Галерея шаблонов, LUT и эффектов широко слева, просмотр и инспектор справа — подбирать оформление
//! order: 35
fn layout() {
    column(
        row(
            tabs(["templates", "luts", "effects", "media"]),
            row(tabs(["viewer"], "viewer"), tabs(["inspector", "ai"]), 0.66),
            0.46
        ),
        tabs(["timeline", "curves"]),
        0.66
    )
}
