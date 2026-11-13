//! «Гостевые инструменты syn-vm» — один диск для всех гостей вместо virtio-win и отдельных образов
//! syn-gpu, разделённый по каталогам систем:
//!
//! - `<система>\agent\` — гостевой агент syn-vm этой системы;
//! - `win10\` — Windows 10/11: `syn-vm-serial\` (virtio-serial для порта агента) и `syn-gpu\`
//!   (инструменты видеокарты syn-gpu); ставит `install.cmd` в корне;
//! - `win7\`, `winxp\`, `win9x\`, `dos\`, `linux\` — драйверы и утилиты syn-gpu этих систем,
//!   как их собирает проект syn-virt (каталог `out/guest-tools/<система>/`).
//!
//! Образ собирается здесь, из установленных частей, в `~/.cache/syn-vm/syn-vm-guest-tools.iso`
//! (метка SYNVM) и пересобирается, когда часть изменилась.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use syngui::t;

pub const LABEL: &str = "SYNVM";

const INSTALL_CMD: &str = include_str!("../guest-tools/install.cmd");
const INSTALL_PS1: &str = include_str!("../guest-tools/install.ps1");
const README: &str = include_str!("../guest-tools/README.txt");

pub fn cache_dir() -> PathBuf {
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

/// Каталоги систем от syn-virt (`<корень>/<система>/`): SYN_VM_GUEST_TOOLS, пакеты syn-virt,
/// рядом со сборкой syn-gpu (`syn-virt/out/guest-tools`). Корень узнаётся по каталогу хотя бы одной
/// системы.
pub fn systems_root() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("SYN_VM_GUEST_TOOLS") {
        c.push(PathBuf::from(p));
    }
    c.push(PathBuf::from("/usr/share/syn-virt/guest-tools"));
    c.push(PathBuf::from("/usr/share/syn-gpu/guest-tools"));
    if let Some(h) = std::env::var_os("HOME") {
        c.push(PathBuf::from(&h).join(".local/share/syn-virt/guest-tools"));
    }
    if let Some(bin) = crate::qemu::syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()) {
        if let Some(root) = bin.parent().and_then(Path::parent).and_then(Path::parent) {
            c.push(root.join("out/guest-tools"));
        }
    }
    c.into_iter().find(|d| SYSTEMS.iter().any(|s| d.join(s).is_dir()))
}

/// Каталоги систем на диске.
pub const SYSTEMS: [&str; 6] = ["win10", "win7", "winxp", "win9x", "dos", "linux"];

/// Отпечаток каталога: самое позднее изменение и число файлов (для пересборки образа).
fn tree_stamp(dir: &Path) -> (u64, usize) {
    let mut newest = mtime(dir);
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let (m, k) = tree_stamp(&p);
                newest = newest.max(m);
                n += k;
            } else {
                newest = newest.max(mtime(&p));
                n += 1;
            }
        }
    }
    (newest, n)
}

/// Скопировать каталог целиком (права «только чтение» не переносятся — образ пересобирается).
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
            if let Ok(m) = std::fs::metadata(&dst) {
                let mut p = m.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                p.set_readonly(false);
                let _ = std::fs::set_permissions(&dst, p);
            }
        }
    }
    Ok(())
}

fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Собрать (или взять готовый) образ. Нет агента — ошибка; нет драйвера или syn-gpu — без них.
pub fn iso() -> Result<PathBuf> {
    let agent = crate::agent::agent_binary(true).context(t!("нет сборки агента syn-vm-agent.exe (/usr/lib/syn-vm/agent)"))?;
    let agent_linux = crate::agent::agent_binary(false);
    let agent_9x = crate::agent::agent_9x();
    let agent_dos = crate::agent::agent_dos();
    let systems = systems_root();
    // Прежние части (пока syn-virt не кладёт каталоги систем): драйвер и образ syn-gpu.
    let serial = if systems.as_ref().is_some_and(|r| r.join("win10/syn-vm-serial").is_dir()) { None } else { serial_dir() };
    let gpu = if systems.as_ref().is_some_and(|r| r.join("win10/syn-gpu").is_dir()) { None } else { crate::gputools::iso() };
    // Отпечаток частей: пути, времена изменения, скрипты.
    let mut stamp = format!("v2\n{}\n{}\n", agent.display(), mtime(&agent));
    for a in [&agent_linux, &agent_9x, &agent_dos].into_iter().flatten() {
        stamp += &format!("{}\n{}\n", a.display(), mtime(a));
    }
    if let Some(r) = &systems {
        let (m, n) = tree_stamp(r);
        stamp += &format!("{}\n{m}\n{n}\n", r.display());
    }
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
    // Агент — в каталоге каждой системы (`<система>\agent\`): Windows 7–11 — один exe, Linux — свой.
    for sys in ["win10", "win7"] {
        std::fs::create_dir_all(stage.join(sys).join("agent"))?;
        std::fs::copy(&agent, stage.join(sys).join("agent/syn-vm-agent.exe"))?;
    }
    if let Some(a) = &agent_linux {
        std::fs::create_dir_all(stage.join("linux/agent"))?;
        std::fs::copy(a, stage.join("linux/agent/syn-vm-agent"))?;
    }
    // Windows 95/98/ME и XP — SYNAGENT.EXE (имя 8.3: диск читают и без длинных имён).
    if let Some(a) = &agent_9x {
        for sys in ["win9x", "winxp"] {
            std::fs::create_dir_all(stage.join(sys).join("AGENT"))?;
            std::fs::copy(a, stage.join(sys).join("AGENT/SYNAGENT.EXE"))?;
        }
    }
    if let Some(a) = &agent_dos {
        std::fs::create_dir_all(stage.join("dos/AGENT"))?;
        std::fs::copy(a, stage.join("dos/AGENT/SYNAGENT.COM"))?;
    }
    if let Some(r) = &systems {
        for sys in SYSTEMS {
            if r.join(sys).is_dir() {
                copy_tree(&r.join(sys), &stage.join(sys))?;
            }
        }
    }
    if let Some(d) = &serial {
        let to = stage.join("win10/syn-vm-serial");
        std::fs::create_dir_all(&to)?;
        for f in ["syn-vm-serial.sys", "syn-vm-serial.inf", "syn-virt-test.cer"] {
            if d.join(f).is_file() {
                std::fs::copy(d.join(f), to.join(f))?;
            }
        }
    }
    if let Some(g) = &gpu {
        // Содержимое диска syn-gpu — в каталог syn-gpu\ (его install.ps1 зовёт наш).
        let to = stage.join("win10/syn-gpu");
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
    let have_dirs: Vec<String> = SYSTEMS.iter().filter(|s| stage.join(s).is_dir()).map(|s| s.to_string()).collect();
    let _ = std::fs::remove_dir_all(&stage);
    tracing::info!("гостевые инструменты syn-vm: {} (каталоги систем: {})", out.display(), have_dirs.join(", "));
    Ok(out)
}

#[cfg(test)]
mod tests {
    /// Сборка образа по каталогам систем (нужны xorriso и сборки агентов: SYN_VM_AGENT_EXE, SYN_VM_AGENT_9X, SYN_VM_AGENT_DOS,
    /// SYN_VM_GUEST_TOOLS — каталог систем, XDG_CACHE_HOME — куда класть образ).
    #[test]
    #[ignore]
    fn iso_has_system_dirs() {
        let iso = super::iso().unwrap();
        let out = std::process::Command::new("xorriso").args(["-indev"]).arg(&iso).args(["-find", "/", "-type", "f"]).output().unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        for f in ["/win10/agent/syn-vm-agent.exe", "/win7/agent/syn-vm-agent.exe", "/win9x/AGENT/SYNAGENT.EXE", "/winxp/AGENT/SYNAGENT.EXE", "/dos/AGENT/SYNAGENT.COM", "/win9x/SYNGPU.INF", "/win7/install.cmd", "/dos/SYNGPU.COM", "/install.cmd", "/README.txt"] {
            assert!(list.contains(f), "{f} нет в образе:\n{list}");
        }
    }
}
