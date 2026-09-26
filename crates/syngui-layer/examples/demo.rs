//! Панель-демо: `SYNGUI_LAYER_HEADLESS=1280x720 SYNGUI_LAYER_DUMP=/tmp/d cargo run --example demo`.

use syngui::prelude::*;
use syngui_layer::{Anchor, Layer, SurfaceSpec};

const MSS: &str = r#"
.bar { background-color: #202329ee; border-radius: 12px; padding: 6px 12px; }
.label { color: #e8eaef; font-size: 14px; }
"#;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    syngui_layer::run(Default::default(), MSS, || {
        let count = use_signal(0);
        syngui_layer::add_timer(std::time::Duration::from_millis(300), move || {
            count.set(count.get_untracked() + 1);
            Some(std::time::Duration::from_millis(300))
        });
        let c2 = count;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            c2.set(1000);
        });
        syngui_layer::create_surface(
            SurfaceSpec {
                namespace: "demo-panel".into(),
                layer: Layer::Top,
                anchor: Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                size: (0, 48),
                margin: [0, 8, 8, 8],
                exclusive_zone: 56,
                ..Default::default()
            },
            move || {
                Box::new(
                    DecoratedBox::new().class("bar").child(
                        Row::new()
                            .gap(12.0)
                            .child(Text::new("syngui-layer").class("label"))
                            .child(Button::new("Нажми").on_click(move || count.set(count.get_untracked() + 1)))
                            .child(move || Text::new(format!("нажато: {}", count.get())).class("label")),
                    ),
                )
            },
        );
    })
}
