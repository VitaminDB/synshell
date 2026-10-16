pub mod files;
pub mod media;
pub mod shell;
pub mod system;
pub mod view_media;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use crate::tools::Tool;

pub fn all() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(system::SystemTool),
        Arc::new(system::WindowsTool),
        Arc::new(files::FilesTool),
        Arc::new(shell::ShellTool),
        Arc::new(crate::tools::web::WebTool::new()),
        Arc::new(media::MediaTool),
        Arc::new(view_media::ViewMediaTool),
    ]
}

pub fn enabled(disabled: &[String]) -> Vec<Arc<dyn Tool>> {
    all().into_iter().filter(|t| !disabled.iter().any(|d| d == t.name())).collect()
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn expand(path: &str) -> PathBuf {
    let p = path.trim();
    if p == "~" {
        return home();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return home().join(rest);
    }
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        home().join(pb)
    }
}

pub fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd}: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("{cmd}: код {}", out.status.code().unwrap_or(-1)) } else { err })
    }
}

pub fn which(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}
