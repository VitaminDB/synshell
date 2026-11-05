use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub const OUTPUT_CAP: usize = 256 * 1024;
pub const DEFAULT_TIMEOUT_SECS: u64 = 600;
pub const MAX_TIMEOUT_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BashError {
    Spawn(String),
    Timeout(u64),
    Cancelled,
}

impl std::fmt::Display for BashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BashError::Spawn(e) => write!(f, "Failed to spawn process: {e}"),
            BashError::Timeout(s) => write!(
                f,
                "Timed out after {s} s: the call was stopped (bash: the process group was killed). If it legitimately needs longer, call again with a larger \"timeout_sec\"; for long jobs run them in the background (`cmd > /tmp/log 2>&1 &`) and poll the log."
            ),
            BashError::Cancelled => write!(f, "Cancelled by the user: the process group was killed."),
        }
    }
}

struct ProcessGroupGuard(Option<u32>);

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// Live output sink: receives stdout/stderr text as it arrives, before the
/// command finishes. Only for display — the result string is unchanged.
pub type Tap = Arc<dyn Fn(Stream, &str) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Valid UTF-8 prefix of `pending` for the tap; an incomplete trailing
/// sequence stays in `pending` until the next chunk.
fn drain_utf8(pending: &mut Vec<u8>) -> String {
    let valid = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => pending.len(),
    };
    let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
    pending.drain(..valid);
    text
}

fn read_capped<R: AsyncRead + Unpin>(
    stream: Option<R>,
    cap: usize,
    tap: Option<(Tap, Stream)>,
) -> (impl std::future::Future<Output = Vec<u8>>, Arc<AtomicU64>) {
    let dropped = Arc::new(AtomicU64::new(0));
    let counter = dropped.clone();
    let fut = async move {
        let mut kept = Vec::new();
        let Some(mut stream) = stream else { return kept };
        let mut buf = vec![0u8; 64 * 1024];
        let mut pending = Vec::new();
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Some((tap, which)) = &tap {
                        pending.extend_from_slice(&buf[..n]);
                        let text = drain_utf8(&mut pending);
                        if !text.is_empty() {
                            tap(*which, &text);
                        }
                    }
                    let room = cap.saturating_sub(kept.len()).min(n);
                    kept.extend_from_slice(&buf[..room]);
                    counter.fetch_add((n - room) as u64, Ordering::Relaxed);
                }
            }
        }
        if let (Some((tap, which)), false) = (&tap, pending.is_empty()) {
            tap(*which, &String::from_utf8_lossy(&pending));
        }
        kept
    };
    (fut, dropped)
}

fn with_cut_note(mut text: String, dropped: &AtomicU64) -> String {
    let n = dropped.load(Ordering::Relaxed);
    if n > 0 {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("…(truncated {n} bytes)\n"));
    }
    text
}

pub async fn run(command: &str, cwd: Option<&Path>, cap: usize) -> Result<String, BashError> {
    run_tapped(command, cwd, cap, None).await
}

/// `stdbuf` from coreutils: with a tap, glibc stdio of every process in the
/// command is line-buffered, so `echo connected` from a child shows up at once
/// instead of after a 4 KB pipe buffer fills or the process exits.
fn stdbuf() -> Option<&'static str> {
    static PATH: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *PATH.get_or_init(|| ["/usr/bin/stdbuf", "/bin/stdbuf"].into_iter().find(|p| Path::new(p).exists()))
}

/// [`run`] with a live output sink (see [`Tap`]).
pub async fn run_tapped(command: &str, cwd: Option<&Path>, cap: usize, tap: Option<Tap>) -> Result<String, BashError> {
    let mut cmd = match (tap.is_some(), stdbuf()) {
        (true, Some(stdbuf)) => {
            let mut c = Command::new(stdbuf);
            c.args(["-oL", "-eL", "bash"]);
            c
        }
        _ => Command::new("bash"),
    };
    if tap.is_some() {
        cmd.env("PYTHONUNBUFFERED", "1");
    }
    cmd.arg("-lc")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().map_err(|e| BashError::Spawn(e.to_string()))?;
    let mut group = ProcessGroupGuard(child.id());
    let (stdout, stdout_cut) = read_capped(child.stdout.take(), cap, tap.clone().map(|t| (t, Stream::Stdout)));
    let (stderr, stderr_cut) = read_capped(child.stderr.take(), cap, tap.map(|t| (t, Stream::Stderr)));
    let (stdout, stderr) = tokio::join!(stdout, stderr);
    let status = child.wait().await.map_err(|e| BashError::Spawn(e.to_string()))?;
    group.0 = None;
    let stdout = with_cut_note(String::from_utf8_lossy(&stdout).into_owned(), &stdout_cut);
    let stderr = with_cut_note(String::from_utf8_lossy(&stderr).into_owned(), &stderr_cut);
    let mut out = format!("$ {command}\nexit: {}\n", status.code().unwrap_or(-1));
    for (label, body) in [("stdout", &stdout), ("stderr", &stderr)] {
        if !body.is_empty() {
            out.push_str(&format!("--- {label} ---\n"));
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("(no output)\n");
    }
    Ok(out)
}

pub fn timeout_secs(asked: Option<&serde_json::Value>) -> u64 {
    let asked = asked.and_then(|t| t.as_f64().or_else(|| t.as_str().and_then(|s| s.trim().parse::<f64>().ok())));
    match asked {
        Some(t) if t > 0.0 => (t.ceil() as u64).clamp(1, MAX_TIMEOUT_SECS),
        _ => DEFAULT_TIMEOUT_SECS,
    }
}

pub fn run_blocking(command: &str, cwd: Option<&Path>, timeout: u64, cancel: &AtomicBool) -> Result<String, BashError> {
    run_blocking_tapped(command, cwd, timeout, cancel, None)
}

/// Как [`run_blocking`], но вывод по мере появления уходит в `tap` (живой
/// вывод в интерфейсе); результат — тот же.
pub fn run_blocking_tapped(command: &str, cwd: Option<&Path>, timeout: u64, cancel: &AtomicBool, tap: Option<Tap>) -> Result<String, BashError> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| BashError::Spawn(e.to_string()))?;
    rt.block_on(async {
        let watch = async {
            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::select! {
            r = run_tapped(command, cwd, OUTPUT_CAP, tap) => r,
            _ = tokio::time::sleep(Duration::from_secs(timeout)) => Err(BashError::Timeout(timeout)),
            _ = watch => Err(BashError::Cancelled),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_and_exit_code() {
        let out = run_blocking("echo hi; echo err >&2; exit 3", None, 10, &AtomicBool::new(false)).unwrap();
        assert!(out.contains("exit: 3") && out.contains("--- stdout ---\nhi") && out.contains("--- stderr ---\nerr"), "{out}");
        assert!(run_blocking("true", None, 10, &AtomicBool::new(false)).unwrap().contains("(no output)"));
    }

    #[test]
    fn tap_sees_output_before_exit() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(Stream, String, Duration)>::new()));
        let sink = seen.clone();
        let t0 = std::time::Instant::now();
        let tap: Tap = Arc::new(move |s, text| sink.lock().unwrap().push((s, text.to_string(), t0.elapsed())));
        let cmd = "echo connected; python3 -c 'print(\"py\")' 2>/dev/null || echo py; echo ош >&2; sleep 1; echo done";
        let out = rt.block_on(run_tapped(cmd, None, 4096, Some(tap))).unwrap();
        assert!(out.contains("connected\npy\ndone") && out.contains("--- stderr ---\nош"), "{out}");
        let seen = seen.lock().unwrap();
        let first = seen.iter().find(|(s, t, _)| *s == Stream::Stdout && t.contains("connected")).expect("tap");
        assert!(first.2 < Duration::from_millis(800), "{:?}", first.2);
        assert!(seen.iter().any(|(s, t, _)| *s == Stream::Stderr && t == "ош\n"));
    }

    #[test]
    fn utf8_split_is_held_back() {
        let mut p = "яя".as_bytes()[..3].to_vec();
        assert_eq!(drain_utf8(&mut p), "я");
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn output_is_capped() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let out = rt.block_on(run("head -c 3000000 /dev/zero | tr '\\0' a", None, 4096)).unwrap();
        assert!(out.len() < 4096 + 512 && out.contains("…(truncated"), "{}", out.len());
    }

    #[test]
    fn timeout_and_cancel_kill_the_group() {
        let marker = std::env::temp_dir().join(format!("synassist-bash-kill-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let cmd = format!("sleep 1.5 && touch {}", marker.display());
        let t0 = std::time::Instant::now();
        assert_eq!(run_blocking(&cmd, None, 1, &AtomicBool::new(false)), Err(BashError::Timeout(1)));
        assert_eq!(run_blocking("sleep 5", None, 60, &AtomicBool::new(true)), Err(BashError::Cancelled));
        assert!(t0.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "потомок пережил остановку");
        assert_eq!(timeout_secs(Some(&serde_json::json!("7"))), 7);
        assert_eq!(timeout_secs(None), DEFAULT_TIMEOUT_SECS);
    }
}
