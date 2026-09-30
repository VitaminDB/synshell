//! Проба вибромотора: `haptic [сила 0–100] [key|gesture|hold|long|tick|notify]`.
use synshell_common::haptics::{self, Feedback};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strength = args.first().and_then(|s| s.parse().ok()).unwrap_or(70);
    match args.get(1).map(String::as_str) {
        None => haptics::test(strength),
        Some(k) => {
            let f = match k {
                "key" => Feedback::Key,
                "gesture" => Feedback::Gesture,
                "hold" => Feedback::GestureHold,
                "long" => Feedback::LongPress,
                "notify" => Feedback::Notification,
                _ => Feedback::Tick,
            };
            haptics::set_config(&synshell_common::config::Haptics { strength, ..Default::default() });
            haptics::play(f);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
}
