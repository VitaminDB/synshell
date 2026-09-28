//! Игровой словарь: слова и реплики по-английски с переводом и озвучкой.
//!
//! Основа обучения английскому в игровой форме: всё, что «Эльвира» говорит
//! и показывает, — пара `en`/`ru` плюс голосовой клип. Реплики похвалы
//! подбираются по силе хода (`praise`), названия отделок — в `ui::finish`.

use crate::audio::Voice;
use crate::game::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    pub en: &'static str,
    pub ru: &'static str,
    pub voice: Option<Voice>,
}

const fn w(en: &'static str, ru: &'static str, voice: Voice) -> Word {
    Word { en, ru, voice: Some(voice) }
}

/// Уровни похвалы: по числу линий и комбо за ход.
pub const TIER_NICE: &[Word] = &[w("Nice!", "неплохо!", Voice::Nice), w("Good one!", "хороший ход!", Voice::GoodOne)];
pub const TIER_GREAT: &[Word] = &[
    w("Beautiful!", "красиво!", Voice::Beautiful),
    w("Wonderful!", "чудесно!", Voice::Wonderful),
    w("Excellent!", "отлично!", Voice::Excellent),
];
pub const TIER_AMAZING: &[Word] = &[
    w("Amazing!", "потрясающе!", Voice::Amazing),
    w("Incredible!", "невероятно!", Voice::Incredible),
    w("Magnificent!", "великолепно!", Voice::Magnificent),
];
pub const TIER_FIRE: &[Word] = &[
    w("You're on fire!", "ты в ударе!", Voice::OnFire),
    w("Unstoppable!", "не остановить!", Voice::Unstoppable),
    w("You're beautiful!", "ты прекрасна!", Voice::YouBeautiful),
];
pub const PERFECT: &[Word] = &[w("Perfect!", "идеально!", Voice::Perfect), w("Flawless!", "безупречно!", Voice::Flawless)];
pub const NEW_RECORD: Word = w("New record!", "новый рекорд!", Voice::NewRecord);
pub const GAME_OVER: Word = w("Game over", "игра окончена", Voice::GameOver);
pub const WELCOME: Word = w("Welcome back!", "с возвращением!", Voice::Welcome);
pub const COMBO: Word = Word { en: "Combo", ru: "комбо", voice: None };
pub const LEVEL: Word = Word { en: "Level", ru: "уровень", voice: None };
pub const NO_MOVES: Word = Word { en: "No moves", ru: "нет ходов", voice: None };

/// Уровень похвалы за ход: `None` — ход без очистки или одна линия без комбо.
pub fn tier(lines: usize, combo: u32, perfect: bool) -> Option<&'static [Word]> {
    if perfect {
        Some(PERFECT)
    } else if combo >= 5 || lines >= 4 {
        Some(TIER_FIRE)
    } else if combo >= 3 || lines >= 3 {
        Some(TIER_AMAZING)
    } else if combo >= 2 || lines >= 2 {
        Some(TIER_GREAT)
    } else if lines >= 1 && combo >= 1 {
        Some(TIER_NICE)
    } else {
        None
    }
}

/// Случайная реплика уровня, не совпадающая с `last`.
pub fn praise(rng: &mut Rng, lines: usize, combo: u32, perfect: bool, last: Option<Word>) -> Option<Word> {
    let t = tier(lines, combo, perfect)?;
    let mut w = t[rng.index(t.len())];
    if Some(w) == last && t.len() > 1 {
        w = t[(t.iter().position(|x| *x == w).unwrap_or(0) + 1) % t.len()];
    }
    Some(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_grow_with_strength() {
        assert!(tier(0, 0, false).is_none());
        assert!(tier(1, 0, false).is_none());
        assert_eq!(tier(1, 1, false), Some(TIER_NICE));
        assert_eq!(tier(2, 1, false), Some(TIER_GREAT));
        assert_eq!(tier(1, 3, false), Some(TIER_AMAZING));
        assert_eq!(tier(4, 1, false), Some(TIER_FIRE));
        assert_eq!(tier(1, 5, false), Some(TIER_FIRE));
        assert_eq!(tier(1, 1, true), Some(PERFECT));
    }

    #[test]
    fn praise_avoids_repeat_and_has_translation() {
        let mut rng = Rng::new(5);
        let mut last = None;
        for _ in 0..50 {
            let w = praise(&mut rng, 3, 3, false, last).unwrap();
            assert_ne!(Some(w), last);
            assert!(!w.ru.is_empty() && w.voice.is_some());
            last = Some(w);
        }
        for t in [TIER_NICE, TIER_GREAT, TIER_AMAZING, TIER_FIRE, PERFECT] {
            for w in t {
                assert!(w.en.ends_with('!') && !w.ru.is_empty());
            }
        }
    }
}
