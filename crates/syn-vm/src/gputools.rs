//! «Гостевые инструменты syn-gpu»: драйвер паравиртуальной видеокарты syn-gpu в госте Windows
//! одной командой (проект qemu-vulkan-driver, docs/guest-tools.md). Образ
//! `syn-gpu-guest-tools.iso` (метка тома SYNGPU) — в привод, затем через гостевой агент
//! `<буква>:\install.cmd /quiet`: 0 — готово, 3010 — нужна перезагрузка (тестовая подпись;
//! после неё установка доводится сама), иное — ошибка (журнал C:\ProgramData\syn-gpu\install.log).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use syngui::prelude::*;

use crate::model::Vm;

const ISO: &str = "syn-gpu-guest-tools.iso";
const AGENT: &str = r"C:\Program Files\syn-gpu\syn-gpu-agent.exe";

/// Образ инструментов: SYN_GPU_TOOLS, /usr/share/syn-gpu, ~/.local/share/syn-gpu, рядом с syn-gpu.
pub fn iso() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("SYN_GPU_TOOLS") {
        c.push(PathBuf::from(p));
    }
    c.push(PathBuf::from("/usr/share/syn-gpu").join(ISO));
    if let Some(h) = std::env::var_os("HOME") {
        c.push(PathBuf::from(h).join(".local/share/syn-gpu").join(ISO));
    }
    if let Some(bin) = crate::qemu::syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()) {
        if let Some(dir) = bin.parent() {
            c.push(dir.join(ISO));
            c.push(dir.join("../share/syn-gpu").join(ISO));
        }
    }
    c.into_iter().find(|p| p.is_file())
}

/// Команда PowerShell в госте → вывод (UTF-8) и код.
fn ps(vm: &Vm, script: &str, timeout: Duration) -> Result<(i32, String)> {
    let argv = vec!["powershell.exe".to_string(), "-NoProfile".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-Command".into(), script.into()];
    let r = crate::qga::exec(vm, &argv, None, &[], timeout)?;
    let out = String::from_utf8_lossy(&crate::qga::decode_output(vm, r.stdout)).trim().to_string();
    Ok((r.code, out))
}

/// Поставить инструменты: образ в привод, install.cmd, при 3010 — перезагрузка и ожидание
/// агента. `step` — ход для пользователя. Итог — строка состояния.
pub fn install(vm: &Vm, step: &dyn Fn(String)) -> Result<String> {
    if !vm.os.is_windows() {
        bail!("{}", t!("Гостевые инструменты syn-gpu — только для Windows"));
    }
    let iso = iso().with_context(|| t!("Нет образа {iso} (SYN_GPU_TOOLS, /usr/share/syn-gpu или ~/.local/share/syn-gpu)", iso = ISO))?;
    step(t!("Образ в привод…"));
    crate::qmp::change_cdrom(vm, &iso.to_string_lossy())?;
    step(t!("Ждём гостевой агент…"));
    crate::qga::wait(vm, Duration::from_secs(300))?;
    // Буква привода по метке тома (Windows монтирует его не сразу).
    let mut letter = String::new();
    for _ in 0..20 {
        let (_, out) = ps(vm, "(Get-Volume -FileSystemLabel SYNGPU -EA 0 | select -First 1).DriveLetter", Duration::from_secs(30))?;
        if out.len() == 1 {
            letter = out;
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if letter.is_empty() {
        bail!("{}", t!("Гость не видит диск SYNGPU — образ вставлен?"));
    }
    step(t!("Установка в госте…"));
    let argv = vec!["cmd.exe".to_string(), "/c".into(), format!(r"{letter}:\install.cmd"), "/quiet".into()];
    let r = crate::qga::exec(vm, &argv, None, &[], Duration::from_secs(1800))?;
    match r.code {
        0 => {}
        3010 => {
            step(t!("Перезагрузка гостя (тестовая подпись)…"));
            crate::qga::shutdown(vm, "reboot")?;
            std::thread::sleep(Duration::from_secs(15));
            step(t!("Ждём гостевой агент…"));
            crate::qga::wait(vm, Duration::from_secs(600))?;
        }
        code => {
            let (_, tail) = ps(vm, r"Get-Content C:\ProgramData\syn-gpu\install.log -Tail 15 -EA 0", Duration::from_secs(30)).unwrap_or_default();
            bail!("{}", t!("Установка не удалась (код {code}):\n{tail}", code = code, tail = tail));
        }
    }
    Ok(status(vm).unwrap_or_else(|_| t!("Установлено")))
}

/// Состояние в госте: `syn-gpu-agent status --json` (драйвер, ICD, игры с DXVK).
pub fn status(vm: &Vm) -> Result<String> {
    let (code, out) = ps(vm, &format!("if (Test-Path '{AGENT}') {{ & '{AGENT}' status --json }} else {{ exit 4 }}"), Duration::from_secs(30))?;
    if code == 4 {
        bail!("{}", t!("Гостевые инструменты syn-gpu не установлены"));
    }
    Ok(out)
}
