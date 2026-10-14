//! Запуск QEMU: командная строка из настроек машины, TPM (swtpm), прошивка UEFI,
//! проверка процесса. QEMU уходит в фон (`-daemonize`) и живёт без окна менеджера:
//! экран — VNC на unix-сокете, управление — QMP.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use syngui::prelude::*;

use crate::model::{DiskBus, DisplayModel, Firmware, NetMode, OsKind, Vm};

pub const QEMU: &str = "qemu-system-x86_64";

/// Прошивка UEFI: код (только чтение) и шаблон переменных.
pub struct Ovmf {
    pub code: PathBuf,
    pub vars: PathBuf,
}

/// Найти OVMF: Arch (`edk2-ovmf`), Debian/Ubuntu (`ovmf`), Fedora (`edk2-ovmf`), сборки QEMU.
pub fn firmware(fw: Firmware) -> Option<Ovmf> {
    let secure = fw == Firmware::UefiSecure;
    let dirs = ["/usr/share/edk2/x64", "/usr/share/edk2-ovmf/x64", "/usr/share/OVMF", "/usr/share/edk2/ovmf", "/usr/share/qemu"];
    let codes: &[&str] = if secure {
        &["OVMF_CODE.secboot.4m.fd", "OVMF_CODE_4M.secboot.fd", "OVMF_CODE.secboot.fd", "OVMF_CODE_4M.ms.fd"]
    } else {
        &["OVMF_CODE.4m.fd", "OVMF_CODE_4M.fd", "OVMF_CODE.fd", "edk2-x86_64-code.fd"]
    };
    let vars = ["OVMF_VARS.4m.fd", "OVMF_VARS_4M.fd", "OVMF_VARS.fd", "edk2-i386-vars.fd"];
    for d in dirs {
        let d = Path::new(d);
        for c in codes {
            let code = d.join(c);
            if !code.exists() {
                continue;
            }
            // Шаблон переменных того же размера (4M к 4M).
            let four = c.contains("4m") || c.contains("4M");
            for v in vars {
                let vp = d.join(v);
                if vp.exists() && (v.contains("4m") || v.contains("4M")) == four {
                    return Some(Ovmf { code, vars: vp });
                }
            }
            if let Some(v) = vars.iter().map(|v| d.join(v)).find(|p| p.exists()) {
                return Some(Ovmf { code, vars: v });
            }
        }
    }
    None
}

pub fn kvm_available() -> bool {
    std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm").is_ok()
}

pub fn qemu_installed() -> bool {
    which(QEMU).is_some()
}

pub fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(bin)).find(|p| p.is_file()))
}

/// Аргументы QEMU (без имени программы).
pub fn args(vm: &Vm) -> Result<Vec<String>> {
    let kvm = kvm_available();
    let run = vm.run_dir();
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let name = format!("{},process=syn-vm:{}", vm.name.replace(',', ",,"), vm.id);
    push(&["-name", &name]);
    let mut machine = String::from("q35");
    machine.push_str(if kvm { ",accel=kvm" } else { ",accel=tcg" });
    if vm.firmware == Firmware::UefiSecure {
        machine.push_str(",smm=on");
    }
    push(&["-machine", &machine]);
    let cpu = if !kvm {
        "max".to_string()
    } else if vm.os.is_windows() {
        "host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time,hv_vpindex,hv_synic,hv_stimer,hv_frequencies".to_string()
    } else {
        "host".to_string()
    };
    push(&["-cpu", &cpu]);
    push(&["-smp", &format!("{}", vm.cpus.max(1))]);
    push(&["-m", &format!("{}M", vm.memory_mb.max(256))]);
    push(&["-pidfile", &run.join("qemu.pid").to_string_lossy()]);
    push(&["-daemonize"]);
    push(&["-qmp", &format!("unix:{},server=on,wait=off", vm.qmp_path().display())]);
    push(&["-display", "none"]);
    push(&["-vnc", &format!("unix:{}", vm.vnc_path().display())]);
    push(&["-rtc", if vm.os.is_windows() { "base=localtime" } else { "base=utc" }]);

    // Прошивка.
    if vm.firmware != Firmware::Bios {
        let fw = firmware(vm.firmware).context(t!("Не найдена прошивка UEFI (пакет edk2-ovmf)"))?;
        if !vm.vars_path().exists() {
            crate::model::make_vars(&fw.vars, &vm.vars_path())?;
        }
        if vm.firmware == Firmware::UefiSecure {
            push(&["-global", "driver=cfi.pflash01,property=secure,value=on"]);
        }
        push(&["-drive", &format!("if=pflash,format=raw,unit=0,readonly=on,file={}", esc(&fw.code))]);
        push(&["-drive", &format!("if=pflash,format=qcow2,unit=1,file={}", esc(&vm.vars_path()))]);
    }

    // Видео.
    match vm.display {
        DisplayModel::Virtio => push(&["-vga", "none", "-device", "virtio-vga,xres=1280,yres=800"]),
        DisplayModel::Std => push(&["-vga", "none", "-device", "VGA,vgamem_mb=64"]),
        DisplayModel::Qxl => push(&["-vga", "none", "-device", "qxl-vga,vgamem_mb=64"]),
    }

    // Диски.
    let mut boot = 1;
    // «С привода»: первая загрузка — с образа; через полминуты привод уходит в
    // конец очереди (`after_boot`), и перезагрузка установщика попадает на диск.
    let cd_boot = if vm.boot_cdrom && !vm.cdrom.is_empty() { 0 } else { 100 };
    for (i, d) in vm.disks.iter().enumerate() {
        let path = vm.disk_path(d);
        if !path.exists() {
            bail!("{}", t!("Нет файла диска: {p}", p = path.display()));
        }
        let fmt = if path.extension().is_some_and(|e| e == "qcow2") { "qcow2" } else { "raw" };
        push(&["-drive", &format!("file={},if=none,id=disk{i},format={fmt},discard=unmap,detect-zeroes=unmap,cache=writeback,aio=threads", esc(&path))]);
        match vm.disk_bus {
            DiskBus::Virtio => push(&["-device", &format!("virtio-blk-pci,drive=disk{i},bootindex={boot}")]),
            DiskBus::Sata => push(&["-device", &format!("ide-hd,drive=disk{i},bus=ide.{},bootindex={boot}", i.min(3))]),
            DiskBus::Nvme => push(&["-device", &format!("nvme,drive=disk{i},serial=synvm{i},bootindex={boot}")]),
        }
        boot += 1;
    }
    // Привод — всегда (образ можно вставить на ходу).
    let cd = if vm.cdrom.is_empty() || !Path::new(&vm.cdrom).exists() {
        "if=none,id=cd0,media=cdrom".to_string()
    } else {
        format!("file={},if=none,id=cd0,media=cdrom,readonly=on", esc(Path::new(&vm.cdrom)))
    };
    push(&["-drive", &cd]);
    push(&["-device", &format!("ide-cd,drive=cd0,bus=ide.4,id=cdrom,bootindex={cd_boot}")]);

    // Сеть.
    match vm.net {
        NetMode::Nat => {
            let mut nd = String::from("user,id=net0");
            for f in &vm.forwards {
                if f.host > 0 && f.guest > 0 {
                    nd.push_str(&format!(",hostfwd={}::{}-:{}", if f.udp { "udp" } else { "tcp" }, f.host, f.guest));
                }
            }
            push(&["-netdev", &nd]);
            push(&["-device", &format!("{},netdev=net0", vm.nic.device())]);
        }
        NetMode::None => push(&["-nic", "none"]),
    }

    // USB: планшет — абсолютный указатель, курсор гостя совпадает с мышью.
    push(&["-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0"]);

    if vm.sound {
        let backend = if which("pipewire").is_some() || Path::new(&synshell_common::paths::runtime_dir()).join("pipewire-0").exists() { "pipewire" } else { "pa" };
        push(&["-audiodev", &format!("{backend},id=snd0"), "-device", "ich9-intel-hda", "-device", "hda-duplex,audiodev=snd0"]);
    }

    if vm.tpm {
        push(&["-chardev", &format!("socket,id=chrtpm,path={}", run.join("swtpm.sock").display())]);
        push(&["-tpmdev", "emulator,id=tpm0,chardev=chrtpm", "-device", "tpm-tis,tpmdev=tpm0"]);
    }

    if vm.clipboard {
        push(&["-device", "virtio-serial-pci", "-chardev", "qemu-vdagent,id=vdagent,name=vdagent,clipboard=on"]);
        push(&["-device", "virtserialport,chardev=vdagent,name=com.redhat.spice.0"]);
    }

    if !vm.shared_dir.is_empty() && Path::new(&vm.shared_dir).is_dir() {
        push(&["-virtfs", &format!("local,path={},mount_tag=hostshare,security_model=none,id=share0", esc(Path::new(&vm.shared_dir)))]);
    }

    if matches!(vm.os, OsKind::Linux | OsKind::Android | OsKind::Bsd) {
        push(&["-device", "virtio-rng-pci"]);
    }
    push(&["-device", "virtio-balloon-pci"]);

    a.extend(split_args(&vm.extra_args));
    Ok(a)
}

/// Запятая в путях `-drive file=…` удваивается.
fn esc(p: &Path) -> String {
    p.to_string_lossy().replace(',', ",,")
}

/// Разбор дополнительных аргументов: пробелы, кавычки "…" и '…'.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut any = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Командная строка для показа («Подробнее»).
pub fn command_line(vm: &Vm) -> String {
    match args(vm) {
        Ok(a) => std::iter::once(QEMU.to_string())
            .chain(a.into_iter().map(|s| if s.contains(' ') || s.is_empty() { format!("'{s}'") } else { s }))
            .collect::<Vec<_>>()
            .join(" "),
        Err(e) => e.to_string(),
    }
}

/// Запустить машину. Возвращается, когда QEMU ушёл в фон (или упал при запуске).
pub fn start(vm: &Vm) -> Result<()> {
    if !qemu_installed() {
        bail!("{}", t!("QEMU не установлен — нужен пакет qemu-desktop или qemu-full"));
    }
    if pid(vm).is_some() {
        return Ok(());
    }
    let run = vm.run_dir();
    std::fs::create_dir_all(&run)?;
    for f in ["qmp.sock", "vnc.sock", "qemu.pid", "swtpm.sock"] {
        let _ = std::fs::remove_file(run.join(f));
    }
    let args = args(vm)?;
    if vm.tpm {
        start_tpm(vm)?;
    }
    tracing::info!("{QEMU} {}", args.join(" "));
    let out = Command::new(QEMU)
        .args(&args)
        .current_dir(vm.dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context(QEMU)?;
    let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::write(vm.dir().join("qemu.log"), &log);
    if !out.status.success() {
        let msg = log.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim_start_matches("qemu-system-x86_64: ")).collect::<Vec<_>>().join("\n");
        bail!("{}", if msg.is_empty() { t!("QEMU завершился с ошибкой") } else { msg });
    }
    Ok(())
}

/// swtpm на сокете машины: завершается сам, когда QEMU закрывает соединение.
fn start_tpm(vm: &Vm) -> Result<()> {
    if which("swtpm").is_none() {
        bail!("{}", t!("Для TPM нужен пакет swtpm"));
    }
    let state = vm.dir().join("tpm");
    std::fs::create_dir_all(&state)?;
    let sock = vm.run_dir().join("swtpm.sock");
    let st = Command::new("swtpm")
        .args(["socket", "--tpm2", "--daemon", "--terminate"])
        .arg("--tpmstate")
        .arg(format!("dir={}", state.display()))
        .arg("--ctrl")
        .arg(format!("type=unixio,path={}", sock.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if !st.status.success() {
        bail!("swtpm: {}", String::from_utf8_lossy(&st.stderr).trim());
    }
    for _ in 0..50 {
        if sock.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    bail!("swtpm: {}", t!("сокет не появился"))
}

/// После первой загрузки с привода — привод в конец очереди загрузки: прошивка
/// перечитывает порядок при каждом сбросе, и перезагрузка идёт уже с диска.
pub fn after_boot(vm: &Vm) {
    if !(vm.boot_cdrom && !vm.cdrom.is_empty()) {
        return;
    }
    // Установщик Windows ждёт «Press any key to boot from CD» всего пять секунд —
    // нажать за пользователя, пока подсказка на экране.
    let t0 = std::time::Instant::now();
    if vm.os.is_windows() {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        while t0.elapsed() < std::time::Duration::from_secs(9) {
            let _ = crate::qmp::call(vm, "send-key", serde_json::json!({ "keys": [{ "type": "qcode", "data": "ret" }] }));
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(25).saturating_sub(t0.elapsed()));
    let r = crate::qmp::call(vm, "qom-set", serde_json::json!({ "path": "/machine/peripheral/cdrom", "property": "bootindex", "value": 100 }));
    if let Err(e) = r {
        tracing::info!("bootindex: {e}");
    }
}

/// PID работающего QEMU машины.
pub fn pid(vm: &Vm) -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(vm.pid_path()).ok()?.trim().parse().ok()?;
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let marker = format!("syn-vm:{}", vm.id);
    cmd.split(|b| *b == 0).any(|a| String::from_utf8_lossy(a).contains(&marker)).then_some(pid)
}

/// Выключить сразу (как выдернуть шнур).
pub fn kill(vm: &Vm) {
    if let Some(p) = pid(vm) {
        unsafe {
            libc::kill(p, libc::SIGTERM);
        }
    }
}

/// Счётчики процесса для графиков.
#[derive(Clone, Copy, Default, Debug)]
pub struct ProcStat {
    /// Тики процессора (utime + stime).
    pub ticks: u64,
    /// Резидентная память, байт.
    pub rss: u64,
    /// Чтение и запись диска, байт.
    pub read: u64,
    pub write: u64,
}

pub fn proc_stat(pid: i32) -> Option<ProcStat> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let ticks = f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
    let rss = f.get(21)?.parse::<u64>().ok()? * page;
    let (mut read, mut write) = (0, 0);
    if let Ok(io) = std::fs::read_to_string(format!("/proc/{pid}/io")) {
        for l in io.lines() {
            if let Some(v) = l.strip_prefix("read_bytes: ") {
                read = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("write_bytes: ") {
                write = v.trim().parse().unwrap_or(0);
            }
        }
    }
    Some(ProcStat { ticks, rss, read, write })
}

pub fn clk_tck() -> f64 {
    (unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        assert_eq!(split_args(r#"-device "usb-host,vendorid=0x1234" -s  ''"#), vec!["-device", "usb-host,vendorid=0x1234", "-s", ""]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn args_have_sockets() {
        let mut vm = OsKind::Linux.preset();
        vm.id = "t".into();
        vm.firmware = Firmware::Bios;
        let a = args(&vm).unwrap();
        assert!(a.iter().any(|s| s.starts_with("unix:") && s.ends_with("vnc.sock")));
        assert!(a.contains(&"-daemonize".to_string()));
    }
}
