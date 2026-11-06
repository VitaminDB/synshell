//! «Гостевые инструменты syn-vm» — свой диск для Windows-гостей вместо virtio-win: драйвер
//! virtio-serial syn-vm-serial (проект syn-virt), гостевой агент syn-vm-agent, при наличии —
//! инструменты видеокарты syn-gpu (каталог `syn-gpu\`). Образ собирается здесь, из установленных
//! частей, в `~/.cache/syn-vm/syn-vm-guest-tools.iso` (метка SYNVM) и пересобирается, когда
//! часть изменилась. Установка в госте — `install.cmd` с диска (или через агента).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use syngui::t;

pub const LABEL: &str = "SYNVM";

const INSTALL_CMD: &str = include_str!("../guest-tools/install.cmd");
const INSTALL_PS1: &str = include_str!("../guest-tools/install.ps1");
const README: &str = include_str!("../guest-tools/README.txt");

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_else(|| PathBuf::from("/tmp")).join("syn-vm")
}

pub fn iso_path() -> PathBuf {
    cache_dir().join("syn-vm-guest-tools.iso")
}

/// Драйвер syn-vm-serial (каталог с .sys, .inf и открытым сертификатом): SYN_VM_SERIAL_DIR,
/// пакеты syn-virt, рядом со сборкой syn-gpu (`out/syn-vm-serial` проекта syn-virt).
pub fn serial_dir() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("SYN_VM_SERIAL_DIR") {
        c.push(PathBuf::from(p));
    }
    c.push(PathBuf::from("/usr/share/syn-virt/syn-vm-serial"));
    c.push(PathBuf::from("/usr/share/syn-gpu/syn-vm-serial"));
    if let Some(h) = std::env::var_os("HOME") {
        c.push(PathBuf::from(&h).join(".local/share/syn-virt/syn-vm-serial"));
    }
    if let Some(bin) = crate::qemu::syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()) {
        // syn-virt/target/<профиль>/syn-gpu → syn-virt/out/syn-vm-serial
        if let Some(root) = bin.parent().and_then(Path::parent).and_then(Path::parent) {
            c.push(root.join("out/syn-vm-serial"));
        }
    }
    c.into_iter().find(|d| d.join("syn-vm-serial.sys").is_file() && d.join("syn-vm-serial.inf").is_file())
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Собрать (или взять готовый) образ. Нет агента — ошибка; нет драйвера или syn-gpu — без них.
pub fn iso() -> Result<PathBuf> {
    let agent = crate::agent::agent_binary(true).context(t!("нет сборки агента syn-vm-agent.exe (/usr/share/syn-vm/agent)"))?;
    let serial = serial_dir();
    let gpu = crate::gputools::iso();
    // Отпечаток частей: пути, времена изменения, скрипты.
    let mut stamp = format!("v1\n{}\n{}\n", agent.display(), mtime(&agent));
    if let Some(d) = &serial {
        stamp += &format!("{}\n{}\n{}\n", d.display(), mtime(&d.join("syn-vm-serial.sys")), mtime(&d.join("syn-vm-serial.inf")));
    }
    if let Some(g) = &gpu {
        stamp += &format!("{}\n{}\n", g.display(), mtime(&std::fs::canonicalize(g).unwrap_or(g.clone())));
    }
    stamp += &format!("{}\n", INSTALL_PS1.len() + INSTALL_CMD.len() + README.len());
    let out = iso_path();
    let stamp_path = out.with_extension("stamp");
    if out.is_file() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp) {
        return Ok(out);
    }
    let dir = cache_dir();
    let stage = dir.join("guest-tools-stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(stage.join("agent"))?;
    std::fs::copy(&agent, stage.join("agent/syn-vm-agent.exe"))?;
    if let Some(d) = &serial {
        let to = stage.join("drivers/syn-vm-serial");
        std::fs::create_dir_all(&to)?;
        for f in ["syn-vm-serial.sys", "syn-vm-serial.inf", "syn-virt-test.cer"] {
            if d.join(f).is_file() {
                std::fs::copy(d.join(f), to.join(f))?;
            }
        }
    }
    if let Some(g) = &gpu {
        // Содержимое диска syn-gpu — в каталог syn-gpu\ (его install.ps1 зовёт наш).
        let to = stage.join("syn-gpu");
        std::fs::create_dir_all(&to)?;
        let st = Command::new("xorriso").args(["-osirrox", "on", "-indev"]).arg(g).args(["-extract", "/"]).arg(&to).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().context("xorriso")?;
        if !st.success() {
            bail!("{}", t!("не удалось распаковать {p}", p = g.display()));
        }
        // xorriso переносит права «только чтение» — вернуть запись, чтобы образ пересобирался.
        let _ = Command::new("chmod").args(["-R", "u+w"]).arg(&to).status();
    }
    std::fs::write(stage.join("install.cmd"), INSTALL_CMD)?;
    std::fs::write(stage.join("install.ps1"), INSTALL_PS1)?;
    std::fs::write(stage.join("README.txt"), README)?;
    let tmp = out.with_extension("iso.part");
    let st = Command::new("xorriso")
        .args(["-as", "mkisofs", "-quiet", "-V", LABEL, "-J", "-joliet-long", "-R", "-o"])
        .arg(&tmp)
        .arg(&stage)
        .status()
        .context(t!("xorriso (пакет libisoburn) — нужен для образа «Гостевые инструменты syn-vm»"))?;
    if !st.success() {
        bail!("{}", t!("xorriso не собрал образ"));
    }
    std::fs::rename(&tmp, &out)?;
    std::fs::write(&stamp_path, stamp)?;
    let _ = std::fs::remove_dir_all(&stage);
    tracing::info!("гостевые инструменты syn-vm: {} (драйвер: {}, syn-gpu: {})", out.display(), serial.is_some(), gpu.is_some());
    Ok(out)
}
