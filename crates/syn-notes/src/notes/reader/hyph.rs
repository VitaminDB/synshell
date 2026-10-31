//! Переносы слов: русские — по правилам П. Христенко (слоги по гласным,
//! согласным и «й/ь/ъ»), английские — простые слоговые правила (VC-CV,
//! V-CV). Минимум две буквы до переноса и две после.

fn is_vowel_ru(c: char) -> bool {
    "аеёиоуыэюя".contains(c)
}
fn is_cons_ru(c: char) -> bool {
    "бвгджзклмнпрстфхцчшщ".contains(c)
}
fn is_x_ru(c: char) -> bool {
    "йъь".contains(c)
}
fn is_letter_ru(c: char) -> bool {
    is_vowel_ru(c) || is_cons_ru(c) || is_x_ru(c)
}
fn is_vowel_en(c: char) -> bool {
    "aeiouy".contains(c)
}

/// Места переноса в слове — индексы символов, перед которыми можно
/// поставить дефис. Слово — только буквы одного алфавита; иначе пусто.
pub fn hyphen_points(word: &str) -> Vec<usize> {
    let w: Vec<char> = word.chars().flat_map(|c| c.to_lowercase()).collect();
    if w.len() < 4 || w.len() != word.chars().count() {
        return Vec::new();
    }
    let mut pts = Vec::new();
    if w.iter().all(|&c| is_letter_ru(c)) {
        let n = w.len();
        let v = |i: usize| i < n && is_vowel_ru(w[i]);
        let s = |i: usize| i < n && is_cons_ru(w[i]);
        let x = |i: usize| i < n && is_x_ru(w[i]);
        let l = |i: usize| i < n;
        for i in 1..n {
            // Правила как пары «левая часть | правая часть» вокруг места i.
            let ok = (i >= 1 && x(i - 1) && l(i) && l(i + 1))
                || (i >= 2 && v(i - 2) && s(i - 1) && s(i) && v(i + 1))
                || (i >= 2 && s(i - 2) && v(i - 1) && s(i) && v(i + 1))
                || (i >= 2 && v(i - 2) && s(i - 1) && s(i) && s(i + 1) && v(i + 2))
                || (i >= 3 && v(i - 3) && s(i - 2) && s(i - 1) && s(i) && s(i + 1) && v(i + 2))
                || (i >= 3 && v(i - 3) && s(i - 2) && s(i - 1) && s(i) && s(i + 1) && s(i + 2) && v(i + 3))
                || (i >= 1 && v(i - 1) && v(i) && i + 1 < n);
            // «й/ь/ъ» не отрываются от предыдущей буквы.
            let ok = ok && !x(i);
            if ok && i >= 2 && n - i >= 2 && w[..i].iter().any(|&c| is_vowel_ru(c)) && w[i..].iter().any(|&c| is_vowel_ru(c)) {
                pts.push(i);
            }
        }
    } else if w.iter().all(|c| c.is_ascii_lowercase()) {
        let n = w.len();
        let v = |i: usize| i < n && is_vowel_en(w[i]);
        let c = |i: usize| i < n && !is_vowel_en(w[i]);
        for i in 3..n.saturating_sub(2) {
            let ok = (v(i - 2) && c(i - 1) && c(i) && v(i + 1) && !matches!((w[i - 1], w[i]), ('c', 'h') | ('s', 'h') | ('t', 'h') | ('p', 'h') | ('c', 'k') | ('w', 'h')))
                || (v(i - 1) && c(i) && v(i + 1) && i >= 3);
            if ok && w[..i].iter().any(|&c| is_vowel_en(c)) && w[i..].iter().any(|&c| is_vowel_en(c)) {
                pts.push(i);
            }
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hy(w: &str) -> String {
        let pts = hyphen_points(w);
        let mut s = String::new();
        for (i, c) in w.chars().enumerate() {
            if pts.contains(&i) {
                s.push('-');
            }
            s.push(c);
        }
        s
    }

    #[test]
    fn russian() {
        assert_eq!(hy("молоко"), "мо-ло-ко");
        assert_eq!(hy("Электронная"), "Элек-трон-ная");
        assert_eq!(hy("подъезд"), "подъ-езд");
        assert_eq!(hy("майка"), "май-ка");
        assert_eq!(hy("кот"), "кот");
    }

    #[test]
    fn english_and_mixed() {
        assert_eq!(hy("number"), "num-ber");
        assert!(hyphen_points("abc123").is_empty());
        assert!(hyphen_points("tea").is_empty());
    }
}
