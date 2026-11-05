//! Мелочи: запуск программ, поиск в PATH, естественная сортировка.

use std::process::{Command, Stdio};

/// Есть ли программа в `PATH`.
pub fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Вывод программы (stdout), если она завершилась успешно.
pub fn output(bin: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(bin).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Сравнение строк с числами по значению: `cpu2` < `cpu10`.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| {
        let digits: String = s.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        let prefix = &s[..s.len() - digits.len()];
        (prefix.to_string(), digits.parse::<u64>().unwrap_or(0), s.to_string())
    };
    key(a).cmp(&key(b))
}

#[cfg(test)]
mod tests {
    #[test]
    fn natural() {
        let mut v = vec!["cpu10", "cpu2", "cpu1"];
        v.sort_by(|a, b| super::natural_cmp(a, b));
        assert_eq!(v, ["cpu1", "cpu2", "cpu10"]);
    }
}
