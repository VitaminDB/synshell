//! «Гостевые инструменты syn-gpu»: драйвер паравиртуальной видеокарты syn-gpu в госте Windows
//! одной командой (проект syn-virt, docs/guest-tools.md). Образ
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
    let r = crate::qga::exec(vm, &argv, None, &[], Duration::from_secs(1800));
    let res = match r {
        Err(e) => Err(e),
        Ok(r) => match r.code {
            0 => Ok(()),
            3010 => finish_after_reboot(vm, step),
            2 => Err(anyhow::anyhow!("{}", t!("В госте включён Secure Boot — тестовая подпись драйвера не включится (прошивка: UEFI без Secure Boot)"))),
            4 => Err(anyhow::anyhow!("{}", t!("В госте нет устройства syn-gpu — включите «3D-ускорение (syn-gpu)» и перезапустите машину"))),
            5 => Err(anyhow::anyhow!("{}\n{}", t!("Устройство syn-gpu не запустилось"), log_tail(vm))),
            code => Err(anyhow::anyhow!("{}", t!("Установка не удалась (код {code}):\n{tail}", code = code, tail = log_tail(vm)))),
        },
    };
    // Образ — из привода: иначе в госте висит окно Проводника с диском.
    let _ = crate::qmp::change_cdrom(vm, "");
    res?;
    Ok(status(vm).map(|s| s.summary()).unwrap_or_else(|_| t!("Установлено")))
}

fn log_tail(vm: &Vm) -> String {
    ps(vm, r"Get-Content C:\ProgramData\syn-gpu\install.log -Tail 15 -EA 0", Duration::from_secs(30)).map(|r| r.1).unwrap_or_default()
}

/// Код 3010: перезагрузка гостя; установку доводит задача планировщика, итог — в install-state.txt.
fn finish_after_reboot(vm: &Vm, step: &dyn Fn(String)) -> Result<()> {
    step(t!("Перезагрузка гостя (тестовая подпись)…"));
    crate::qga::shutdown(vm, "reboot")?;
    std::thread::sleep(Duration::from_secs(15));
    step(t!("Ждём гостевой агент…"));
    crate::qga::wait(vm, Duration::from_secs(600))?;
    step(t!("Установка доводится после перезагрузки…"));
    let until = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        let st = ps(vm, r"Get-Content C:\ProgramData\syn-gpu\install-state.txt -EA 0", Duration::from_secs(30)).map(|r| r.1).unwrap_or_default();
        let st = st.trim();
        if st == "installed" {
            return Ok(());
        }
        if let Some(e) = st.strip_prefix("error:") {
            bail!("{}", t!("Установка не удалась: {e}\n{tail}", e = e.trim(), tail = log_tail(vm)));
        }
        if std::time::Instant::now() > until {
            bail!("{}", t!("Установка не закончилась за 5 минут после перезагрузки:\n{tail}", tail = log_tail(vm)));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// Состояние в госте (`syn-gpu-agent status --json`).
#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Status {
    pub agent: Agent,
    pub driver: Driver,
    pub icd: Icd,
    pub payload: Payload,
    pub games: Vec<Game>,
}

#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Agent {
    pub version: String,
    pub service: String,
}

#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Driver {
    pub installed: bool,
    pub version: String,
    pub started: bool,
    pub problem: i64,
}

#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Icd {
    pub found: bool,
    pub device: String,
    pub api: String,
    pub error: String,
}

#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Payload {
    pub dxvk: String,
    pub vkd3d: String,
}

#[derive(serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Game {
    pub path: String,
    pub api: String,
    pub bits: u32,
    /// `on` — DXVK подложен, `off` — нет.
    pub dxvk: String,
    pub disabled: bool,
    pub version: String,
}

impl Status {
    /// Одной строкой: драйвер, ICD, игры.
    pub fn summary(&self) -> String {
        let drv = if !self.driver.installed {
            t!("драйвер не установлен")
        } else if self.driver.started {
            t!("драйвер {v} работает", v = self.driver.version)
        } else {
            t!("драйвер {v} не запущен (код {p})", v = self.driver.version, p = self.driver.problem)
        };
        let icd = if self.icd.found { self.icd.device.clone() } else if self.icd.error.is_empty() { t!("ICD не найден") } else { self.icd.error.clone() };
        let on = self.games.iter().filter(|g| g.dxvk == "on").count();
        t!("{drv}; {icd}; игр с DXVK: {on} из {n}", drv = drv, icd = icd, on = on, n = self.games.len())
    }
}

/// Состояние в госте: `syn-gpu-agent status --json`.
pub fn status(vm: &Vm) -> Result<Status> {
    let (code, out) = ps(vm, &format!("if (Test-Path '{AGENT}') {{ & '{AGENT}' status --json }} else {{ exit 4 }}"), Duration::from_secs(30))?;
    if code == 4 {
        bail!("{}", t!("Гостевые инструменты syn-gpu не установлены"));
    }
    let json = out.find('{').map(|i| &out[i..]).unwrap_or(&out);
    serde_json::from_str(json).with_context(|| format!("status --json: {out}"))
}

/// Подключить (`true`) или вернуть исходные DLL (`false`) в папке игры.
pub fn set_game(vm: &Vm, path: &str, on: bool) -> Result<()> {
    let p = path.replace('\'', "''");
    let (code, out) = ps(vm, &format!("& '{AGENT}' {} '{p}'", if on { "enable" } else { "disable" }), Duration::from_secs(60))?;
    if code != 0 {
        bail!("{out}");
    }
    Ok(())
}
