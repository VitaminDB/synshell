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

fn read_capped<R: AsyncRead + Unpin>(stream: Option<R>, cap: usize) -> (impl std::future::Future<Output = Vec<u8>>, Arc<AtomicU64>) {
    let dropped = Arc::new(AtomicU64::new(0));
    let counter = dropped.clone();
    let fut = async move {
        let mut kept = Vec::new();
        let Some(mut stream) = stream else { return kept };
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = cap.saturating_sub(kept.len()).min(n);
                    kept.extend_from_slice(&buf[..room]);
                    counter.fetch_add((n - room) as u64, Ordering::Relaxed);
                }
            }
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
    let mut cmd = Command::new("bash");
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
    let (stdout, stdout_cut) = read_capped(child.stdout.take(), cap);
    let (stderr, stderr_cut) = read_capped(child.stderr.take(), cap);
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
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| BashError::Spawn(e.to_string()))?;
    rt.block_on(async {
        let watch = async {
            while !cancel.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        tokio::select! {
            r = run(command, cwd, OUTPUT_CAP) => r,
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
