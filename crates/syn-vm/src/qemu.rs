//! Запуск QEMU: командная строка из настроек машины, TPM (swtpm), прошивка UEFI,
//! проверка процесса. QEMU уходит в фон (`-daemonize`) и живёт без окна менеджера:
//! экран — VNC на unix-сокете, управление — QMP.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use syngui::prelude::*;

use crate::model::{DiskBus, DisplayModel, Firmware, NetMode, OsKind, ShareMode, Vm};

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
        // Просветления Hyper-V: без них Windows 11 заметно медленнее (таймеры, IPI, сброс TLB).
        "host,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_time,hv_vpindex,hv_synic,hv_stimer,hv_stimer_direct,hv_frequencies,hv_reset,hv_runtime,hv_tlbflush,hv_ipi".to_string()
    } else {
        "host".to_string()
    };
    push(&["-cpu", &cpu]);
    push(&["-smp", &format!("{}", vm.cpus.max(1))]);
    // UEFI (OVMF) нужно не меньше 256 МБ; BIOS-машинам (DOS) хватает и 16.
    let mem_min = if vm.firmware == Firmware::Bios { 16 } else { 256 };
    push(&["-m", &format!("{}M", vm.memory_mb.max(mem_min))]);
    let share = vm.share();
    if share == Some(ShareMode::Virtiofs) {
        // virtiofs (vhost-user) читает память гостя напрямую — память общая.
        push(&["-object", &format!("memory-backend-memfd,id=mem,size={}M,share=on", vm.memory_mb.max(mem_min)), "-numa", "node,memdev=mem"]);
    }
    push(&["-pidfile", &run.join("qemu.pid").to_string_lossy()]);
    push(&["-qmp", &format!("unix:{},server=on,wait=off", vm.qmp_path().display())]);
    push(&["-qmp", &format!("unix:{},server=on,wait=off", vm.qmp_input_path().display())]);
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
        // Единственный экран — syn-gpu (устройство vfio-user ниже): второй монитор у Windows
        // (virtio-vga, фреймбуфер прошивки) ломает DWM на карте syn-gpu.
        DisplayModel::SynGpu => push(&["-vga", "none"]),
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
        // virtio: свой поток ввода-вывода и прямой доступ к файлу (без двойного кэша);
        // SATA/NVMe — как раньше.
        let io = if vm.disk_bus == DiskBus::Virtio { "cache=none,aio=native" } else { "cache=writeback,aio=threads" };
        push(&["-drive", &format!("file={},if=none,id=disk{i},format={fmt},discard=unmap,detect-zeroes=unmap,{io}", esc(&path))]);
        match vm.disk_bus {
            DiskBus::Virtio => {
                push(&["-object", &format!("iothread,id=io{i}")]);
                push(&["-device", &format!("virtio-blk-pci,drive=disk{i},iothread=io{i},bootindex={boot}")]);
            }
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
    // «Гостевые инструменты» — второй привод: установщику Windows нужны драйверы с него,
    // пока в первом приводе стоит сам установщик.
    if vm.tools_cd {
        // Свои инструменты — образ syn-vm (собирается из установленных частей), иначе virtio-win.
        let iso = if vm.syn_tools() {
            crate::guesttools::iso().map_err(|e| tracing::warn!("гостевые инструменты syn-vm: {e}")).ok()
        } else {
            tools_iso()
        };
        if let Some(iso) = iso {
            push(&["-drive", &format!("file={},if=none,id=cd1,media=cdrom,readonly=on", esc(&iso))]);
            push(&["-device", "ide-cd,drive=cd1,bus=ide.5,id=toolscd"]);
        }
    }
    // BIOS и syn-gpu: «Диск инструментов DOS» (SYNGPU.COM и GLIDE2X.OVL).
    if let Some(d) = crate::dostools::qemu_args(vm) {
        push(&d.iter().map(String::as_str).collect::<Vec<_>>());
    }

    // Сеть.
    match vm.net {
        NetMode::Nat => {
            let mut nd = String::from("user,id=net0");
            if share == Some(ShareMode::Smb) {
                // Свой smbd на 10.0.2.4:445 (как делает QEMU для smb=, но со всеми папками машины).
                let cmd = smb_command(vm)?;
                nd.push_str(&format!(",guestfwd=tcp:10.0.2.4:445-cmd:{}", cmd.replace(',', ",,")));
            }
            for f in &vm.forwards {
                if f.host > 0 && f.guest > 0 {
                    nd.push_str(&format!(",hostfwd={}:{}:{}-:{}", if f.udp { "udp" } else { "tcp" }, if f.public { "" } else { "127.0.0.1" }, f.host, f.guest));
                }
            }
            // Канал своего агента по сети: гость подключается к 10.0.2.100:7777 — без драйверов
            // virtio (Windows без virtio-win: сетевая карта e1000e — встроенный драйвер).
            if vm.guest_agent {
                nd.push_str(&format!(",guestfwd=tcp:{}-unix:{},,server=on,,wait=off", crate::agent::NET_ADDR, crate::agent::net_path(vm).display()));
            }
            push(&["-netdev", &nd]);
            push(&["-device", &format!("{},netdev=net0", vm.nic.device())]);
        }
        NetMode::None => push(&["-nic", "none"]),
    }

    // USB: планшет — абсолютный указатель, курсор гостя совпадает с мышью.
    push(&["-device", "qemu-xhci,id=xhci", "-device", "usb-tablet,bus=xhci.0"]);
    // Относительная мышь для игр подключается на ходу в режиме захвата (ui::capture): постоянная
    // USB-мышь становится активной после каждого сброса USB гостем и отнимает указатель у планшета.
    for u in &vm.usb {
        if let Some(dev) = usb_host_device(&u.port) {
            push(&["-device", &dev]);
        }
    }

    if vm.sound {
        let backend = if which("pipewire").is_some() || Path::new(&synshell_common::paths::runtime_dir()).join("pipewire-0").exists() { "pipewire" } else { "pa" };
        let mut ad = format!("{backend},id=snd0");
        if !vm.audio_out.is_empty() {
            ad.push_str(&format!(",out.name={}", vm.audio_out.replace(',', ",,")));
        }
        if !vm.audio_in.is_empty() {
            ad.push_str(&format!(",in.name={}", vm.audio_in.replace(',', ",,")));
        }
        push(&["-audiodev", &ad]);
        match vm.sound_card.resolve(vm.firmware) {
            // DOS-игры: цифровой звук SB16 и музыка FM-синтезом AdLib (OPL2, порт 0x388).
            crate::model::SoundCard::Sb16 => push(&["-device", "sb16,audiodev=snd0", "-device", "adlib,audiodev=snd0"]),
            crate::model::SoundCard::Ac97 => push(&["-device", "AC97,audiodev=snd0"]),
            _ => push(&["-device", "ich9-intel-hda", "-device", "hda-duplex,audiodev=snd0"]),
        }
    }

    if vm.tpm && vm.tpm_host {
        let dev = if Path::new("/dev/tpm0").exists() { "/dev/tpm0" } else { bail!("{}", t!("У компьютера нет TPM (/dev/tpm0)")) };
        push(&["-tpmdev", &format!("passthrough,id=tpm0,path={dev},cancel-path=/dev/null"), "-device", "tpm-tis,tpmdev=tpm0"]);
    } else if vm.tpm {
        push(&["-chardev", &format!("socket,id=chrtpm,path={}", run.join("swtpm.sock").display())]);
        push(&["-tpmdev", "emulator,id=tpm0,chardev=chrtpm", "-device", "tpm-tis,tpmdev=tpm0"]);
    }

    if vm.clipboard || vm.guest_agent {
        push(&["-device", "virtio-serial-pci,id=vser"]);
    }
    // Свои инструменты: в госте нет spice-agent и qemu-ga — их портов нет (буфер обмена — позже
    // через агента syn-vm).
    if vm.clipboard && !vm.syn_tools() {
        // mouse=off: канал — только для буфера обмена; иначе он регистрирует свою «мышь» и
        // становится активной, а клики и VNC-указатель уходят в неё (Windows их теряет).
        push(&["-chardev", "qemu-vdagent,id=vdagent,name=vdagent,clipboard=on,mouse=off"]);
        push(&["-device", "virtserialport,bus=vser.0,chardev=vdagent,name=com.redhat.spice.0"]);
    }
    if vm.guest_agent {
        if !vm.syn_tools() {
            push(&["-chardev", &format!("socket,id=qga0,path={},server=on,wait=off", vm.qga_path().display())]);
            push(&["-device", "virtserialport,bus=vser.0,chardev=qga0,name=org.qemu.guest_agent.0"]);
        }
        // Свой агент syn-vm (syn-vm-agent) — отдельный порт; посредник запускается после QEMU.
        for x in crate::agent::qemu_args(vm) {
            push(&[&x]);
        }
    }

    match share {
        Some(ShareMode::P9) | Some(ShareMode::Auto) => {
            for (i, (d, tag)) in vm.share_dirs().iter().zip(vm.share_names(false)).enumerate() {
                push(&["-virtfs", &format!("local,path={},mount_tag={tag},security_model=none,id=share{i}", esc(Path::new(d)))]);
            }
        }
        Some(ShareMode::Virtiofs) => {
            for (i, tag) in vm.share_names(false).iter().enumerate() {
                push(&["-chardev", &format!("socket,id=vfs{i},path={}", run.join(virtiofs_sock(i)).display())]);
                push(&["-device", &format!("vhost-user-fs-pci,chardev=vfs{i},tag={tag}")]);
            }
        }
        Some(ShareMode::Smb) | None => {}
    }

    for d in &vm.devices {
        a.extend(crate::devices::args(d));
    }
    let mut push = |items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));

    if matches!(vm.os, OsKind::Linux | OsKind::Android | OsKind::Bsd) {
        push(&["-device", "virtio-rng-pci"]);
    }
    // Без virtio-win у balloon в Windows нет драйвера.
    if !vm.syn_tools() {
        push(&["-device", "virtio-balloon-pci"]);
    }
    // Запасные разъёмы PCIe — для дисков, подключаемых на ходу. В конце, чтобы не сдвигать
    // адреса остальных устройств у уже установленных систем.
    for i in 0..crate::devices::HOTPLUG_PORTS {
        push(&["-device", &format!("pcie-root-port,id=hp{i},chassis={},slot={}", 20 + i, 20 + i)]);
    }

    a.extend(split_args(&vm.extra_args));
    Ok(a)
}

/// virtio-win: драйверы virtio, qemu-ga, spice-vdagent для Windows. Ищется рядом с
/// образами установщиков машин, в «Загрузках», в папке syn-vm и в системе; новее — лучше.
pub fn tools_iso() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![crate::model::root(), synshell_common::paths::user_dir_or_default("DOWNLOAD"), PathBuf::from("/usr/share/virtio"), PathBuf::from("/usr/share/virtio-win")];
    for vm in crate::model::load_all() {
        if let Some(p) = Path::new(&vm.cdrom).parent() {
            dirs.push(p.to_path_buf());
            if let Some(pp) = p.parent() {
                dirs.push(pp.to_path_buf());
            }
        }
    }
    // Домашний каталог на две папки вглубь («Образы систем», ISO, Загрузки…).
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for d in std::fs::read_dir(&home).into_iter().flatten().flatten() {
            let n = d.file_name().to_string_lossy().into_owned();
            if n.starts_with('.') || !d.path().is_dir() {
                continue;
            }
            dirs.push(d.path());
            for d2 in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                if !d2.file_name().to_string_lossy().starts_with('.') && d2.file_type().is_ok_and(|t| t.is_dir()) {
                    dirs.push(d2.path());
                }
            }
        }
    }
    for p in [std::env::var("SYN_VM_TOOLS_ISO").unwrap_or_default(), std::fs::read_to_string(crate::model::root().join("tools-iso.path")).unwrap_or_default().trim().to_string()] {
        if !p.is_empty() && Path::new(&p).is_file() {
            return Some(PathBuf::from(p));
        }
    }
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n.starts_with("virtio-win") && n.ends_with(".iso") {
                let ver: Vec<u32> = n.trim_start_matches("virtio-win").trim_end_matches(".iso").split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect();
                if best.as_ref().is_none_or(|(v, _)| ver > *v) {
                    best = Some((ver, e.path()));
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Где взять virtio-win, если его нет.
pub const TOOLS_URL: &str = "https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/stable-virtio/virtio-win.iso";

/// Скачать virtio-win в папку syn-vm.
pub fn download_tools() -> Result<PathBuf> {
    let out = crate::model::root().join("virtio-win.iso");
    let part = out.with_extension("iso.part");
    let st = Command::new("curl").args(["-fL", "--retry", "3", "-o"]).arg(&part).arg(TOOLS_URL).status().context("curl")?;
    if !st.success() {
        let _ = std::fs::remove_file(&part);
        bail!("{}", t!("Не удалось скачать virtio-win"));
    }
    std::fs::rename(&part, &out)?;
    Ok(out)
}

pub fn smbd() -> Option<PathBuf> {
    which("smbd").or_else(|| ["/usr/bin/smbd", "/usr/sbin/smbd"].iter().map(PathBuf::from).find(|p| p.exists()))
}

pub fn virtiofsd() -> Option<PathBuf> {
    which("virtiofsd").or_else(|| ["/usr/lib/virtiofsd", "/usr/libexec/virtiofsd", "/usr/lib/qemu/virtiofsd"].iter().map(PathBuf::from).find(|p| p.exists()))
}

/// `usb-host` на физическом порту: QEMU сам подхватывает устройство после переподключения
/// и смены режима (fastboot, recovery), пока машина работает.
pub fn usb_host_device(port: &str) -> Option<String> {
    let (bus, path) = crate::usb::split_port(port)?;
    Some(format!("usb-host,bus=xhci.0,hostbus={bus},hostport={path},id={}", crate::usb::qemu_id(port)))
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
        Ok(a) => std::iter::once(qemu_bin(vm).map(|p| p.display().to_string()).unwrap_or_else(|_| QEMU.to_string()))
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
    for f in ["qmp.sock", "vnc.sock", "qemu.pid", "swtpm.sock", "qga.sock", "virtiofs.sock", LAUNCHED] {
        let _ = std::fs::remove_file(run.join(f));
    }
    let mut args = args(vm)?;
    if let Some(e) = port_conflicts(vm).into_iter().next() {
        bail!("{e}");
    }
    for d in &vm.devices {
        crate::devices::check(d)?;
    }
    crate::devices::grant(&crate::devices::missing_access(&vm.devices))?;
    if vm.tpm && vm.tpm_host {
        crate::devices::grant(&["/dev/tpm0".to_string()]).ok();
    }
    crate::devices::vfio_prepare(vm)?;
    if vm.tpm && !vm.tpm_host {
        start_tpm(vm)?;
    }
    if args.iter().any(|a| a.starts_with("vhost-user-fs-pci")) {
        start_virtiofsd(vm)?;
    }
    // Устройство уже в «Аргументах QEMU» (ручная настройка) — второе не добавлять.
    if vm.gpu_on() && !args.iter().any(|a| a.contains("vfio-user-pci")) {
        let sock = start_syn_gpu(vm)?;
        args.push("-device".into());
        // BIOS: старые VGA-адреса — устройству (свой VGA/VBE BIOS syn-gpu); UEFI — не нужно (GOP).
        let xvga = if needs_xvga(vm) {
            match vga_accel_lib() {
                Some(lib) => format!(",\"x-vga\":true,\"x-vga-accel\":\"{}\"", lib.display()),
                None => ",\"x-vga\":true".to_string(),
            }
        } else {
            String::new()
        };
        args.push(format!("{{\"driver\":\"vfio-user-pci\",\"socket\":{{\"path\":\"{}\",\"type\":\"unix\"}}{xvga}}}", sock.display()));
    }
    let qemu = qemu_bin(vm)?;
    tracing::info!("{} {}", qemu.display(), args.join(" "));
    // QEMU — отдельной сессией (переживает syn-vm), весь его вывод — в qemu.log: с -daemonize
    // ошибки после ухода в фон (устройство vfio-user, ещё не готовый сокет) пропадали, и машина
    // молча оставалась выключенной. Запуск удался, когда машина отвечает по QMP.
    let log_path = vm.dir().join("qemu.log");
    let log_file = std::fs::File::create(&log_path).with_context(|| log_path.display().to_string())?;
    let mut child = {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new(&qemu);
        c.args(&args).current_dir(vm.dir()).stdin(Stdio::null()).stdout(log_file.try_clone()?).stderr(log_file);
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        c.spawn().with_context(|| qemu.display().to_string())?
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(st) = child.try_wait()? {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            let msg = log.lines().filter(|l| !l.trim().is_empty()).map(|l| l.trim_start_matches("qemu-system-x86_64: ")).collect::<Vec<_>>().join("\n");
            bail!("{}", if msg.is_empty() { t!("QEMU завершился с ошибкой ({st})", st = st) } else { msg });
        }
        if crate::qmp::status(vm).is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            bail!("{}", t!("QEMU не ответил за минуту — смотрите {p}", p = log_path.display()));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    // Процесс QEMU дожидается поток (без зомби, пока жив syn-vm); дальше он — сам по себе.
    std::thread::spawn(move || child.wait());
    // С чем запущена машина: изменения в vm.toml, которые вступят после перезапуска, видны окну.
    if let Ok(t) = toml::to_string_pretty(vm) {
        let _ = std::fs::write(run.join(LAUNCHED), t);
    }
    // Указатель — планшет: у машин с 3D последней добавлена относительная мышь, и QEMU делает
    // активной её (она нужна только в режиме захвата для игр).
    crate::qmp::ensure_tablet(vm);
    // PCI-устройства вернутся компьютеру, когда машина выключится, — даже без окна.
    if !crate::devices::vfio_addrs(vm).is_empty() {
        spawn_watcher(vm, "--vfio-watch");
    }
    // USB-порты: новое устройство в порту (телефон → fastboot) — гостю, даже если QEMU не подхватил.
    if !vm.usb.is_empty() {
        spawn_watcher(vm, "--usb-watch");
    }
    // Посредник своего гостевого агента: постоянное соединение, журнал гостя в guest.log.
    if vm.guest_agent {
        spawn_watcher(vm, "--agent-broker");
    }
    Ok(())
}

/// Фоновый наблюдатель машины (`syn-vm --vfio-watch|--usb-watch МАШИНА`), отдельной сессией.
pub fn spawn_watcher(vm: &Vm, flag: &str) {
    use std::os::unix::process::CommandExt;
    if let Ok(exe) = std::env::current_exe() {
        let mut c = Command::new(exe);
        c.arg(flag).arg(&vm.id).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        if let Ok(mut ch) = c.spawn() {
            std::thread::spawn(move || ch.wait());
        }
    }
}

/// syn-gpu у машины с BIOS: старые VGA-адреса (текст, режимы 13h/12h, VBE) должны доходить до
/// устройства — свойство `x-vga` у `vfio-user-pci` (патч QEMU из syn-virt, пока его нет в upstream).
pub fn needs_xvga(vm: &Vm) -> bool {
    vm.gpu_on() && vm.firmware == Firmware::Bios
}

/// Умеет ли QEMU `x-vga` у `vfio-user-pci` (ответ кэшируется на программу).
fn supports_xvga(bin: &Path) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, bool>>> = std::sync::OnceLock::new();
    let c = CACHE.get_or_init(Default::default);
    if let Some(v) = c.lock().unwrap().get(bin) {
        return *v;
    }
    let ok = Command::new(bin)
        .args(["-device", "vfio-user-pci,help"])
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("x-vga") || String::from_utf8_lossy(&o.stderr).contains("x-vga"))
        .unwrap_or(false);
    c.lock().unwrap().insert(bin.to_path_buf(), ok);
    ok
}

/// Программа QEMU для машины: `SYN_VM_QEMU`; машине с BIOS и syn-gpu — первый QEMU с `x-vga`
/// (системный, сборка syn-virt рядом с сервером syn-gpu, /usr/lib/syn-gpu/qemu); иначе системный.
pub fn qemu_bin(vm: &Vm) -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("SYN_VM_QEMU").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    let system = which(QEMU).unwrap_or_else(|| PathBuf::from(QEMU));
    if !needs_xvga(vm) {
        return Ok(system);
    }
    let mut c = vec![system];
    if let Some(bin) = syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()) {
        // syn-virt/target/<профиль>/syn-gpu → syn-virt/out/qemu/bin
        if let Some(root) = bin.parent().and_then(Path::parent).and_then(Path::parent) {
            c.push(root.join("out/qemu/bin").join(QEMU));
        }
        if let Some(dir) = bin.parent() {
            c.push(dir.join("qemu/bin").join(QEMU));
        }
    }
    c.push(PathBuf::from("/usr/lib/syn-gpu/qemu/bin").join(QEMU));
    c.into_iter().find(|p| p.is_file() && supports_xvga(p)).with_context(|| {
        t!("Машине с BIOS и видеокартой syn-gpu нужен QEMU со свойством x-vga у vfio-user-pci (патч syn-virt): укажите его в переменной SYN_VM_QEMU или выберите прошивку UEFI")
    })
}

/// Библиотека VGA syn-gpu для QEMU (`x-vga-accel`): VGA исполняется в самом QEMU над общей
/// памятью, без обращения к серверу на каждое чтение и запись — для игр DOS обязательно.
/// Рядом с реальной программой syn-gpu, затем /usr/lib/syn-gpu; нет — без неё (медленнее).
pub fn vga_accel_lib() -> Option<PathBuf> {
    const LIB: &str = "libsyn_gpu_qemu.so";
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(dir) = syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()).and_then(|b| b.parent().map(Path::to_path_buf)) {
        c.push(dir.join(LIB));
    }
    c.push(PathBuf::from("/usr/lib/syn-gpu").join(LIB));
    c.into_iter().find(|p| p.is_file())
}

/// Программа сервера syn-gpu: `SYN_GPU` или `syn-gpu` в PATH.
pub fn syn_gpu_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SYN_GPU").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    // Ссылка на сборку syn-virt (~/.local/bin/syn-gpu → target/…) на время пересборки повисает —
    // сервер всё равно есть.
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("syn-gpu")).find(|p| p.is_file() || p.is_symlink()))
}

/// Запустить сервер видеокарты syn-gpu машины (уже работает — оставить) и дождаться его сокета
/// vfio-user. Сервер живёт, пока QEMU подключён, и выходит после выключения машины сам.
fn start_syn_gpu(vm: &Vm) -> Result<PathBuf> {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp")).join("syn-gpu");
    std::fs::create_dir_all(&run)?;
    let sock = run.join(format!("{}.sock", vm.id));
    let pidf = run.join(format!("{}.pid", vm.id));
    // QEMU этой машины ещё не запущен — живой сервер остался от прошлого запуска (он как раз
    // выходит после отключения QEMU): подключение к нему роняет QEMU. Всегда свежий сервер.
    if let Some(pid) = std::fs::read_to_string(&pidf).ok().and_then(|s| s.trim().parse::<i32>().ok()) {
        if unsafe { libc::kill(pid, 0) } == 0 {
            unsafe { libc::kill(pid, libc::SIGTERM) };
            for _ in 0..100 {
                if unsafe { libc::kill(pid, 0) } != 0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if unsafe { libc::kill(pid, 0) } == 0 {
                unsafe { libc::kill(pid, libc::SIGKILL) };
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
    let bin = syn_gpu_bin().context(t!("Нет сервера 3D-видеокарты syn-gpu (переменная SYN_GPU или syn-gpu в PATH)"))?;
    let _ = std::fs::remove_file(&sock);
    let log = vm.dir().join("syn-gpu.log");
    let mut c = Command::new(&bin);
    c.arg("--socket").arg(&sock).arg("--log").arg(&log);
    let extra: Vec<String> = vm.gpu3d_args.split_whitespace().map(String::from).collect();
    if !extra.iter().any(|a| a == "--cpu-latency-us") {
        c.args(["--cpu-latency-us", "0"]);
    }
    // syn-gpu — основная видеокарта: сервер даёт гостю монитор (рабочий стол — кадрами DESKTOP).
    if vm.display == DisplayModel::SynGpu && !extra.iter().any(|a| a == "--display") {
        c.arg("--display");
    }
    c.args(&extra).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let mut child = c.spawn().with_context(|| bin.display().to_string())?;
    let _ = std::fs::write(&pidf, child.id().to_string());
    for _ in 0..100 {
        if sock.exists() {
            std::thread::spawn(move || child.wait());
            return Ok(sock);
        }
        if let Some(st) = child.try_wait()? {
            let tail = std::fs::read_to_string(&log).unwrap_or_default().lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            bail!("{}", t!("Сервер syn-gpu завершился ({st}): {tail}", st = st, tail = tail));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!("{}", t!("Сервер syn-gpu не открыл сокет за 5 с — журнал: {p}", p = log.display()))
}

/// Кто держит порт компьютера: другая машина syn-vm или процесс.
pub fn port_holder(port: u16, udp: bool, except: &str) -> Option<String> {
    for o in crate::model::load_all() {
        if o.id != except && o.net == NetMode::Nat && o.forwards.iter().any(|f| f.host == port && f.udp == udp) && pid(&o).is_some() {
            return Some(t!("машина «{n}»", n = o.name));
        }
    }
    let free = if udp { std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok() } else { std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() };
    if free {
        return None;
    }
    // Процесс — по ss (если есть).
    let out = Command::new("ss").args([if udp { "-Hlunp" } else { "-Hltnp" }, &format!("sport = :{port}")]).output().ok();
    let who = out
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).into_owned();
            let name = s.split("users:((\"").nth(1)?.split('"').next()?.to_string();
            let pid = s.split("pid=").nth(1)?.split(',').next()?.to_string();
            Some(format!("{name} (pid {pid})"))
        })
        .unwrap_or_else(|| t!("другая программа"));
    Some(who)
}

/// Занятые пробросы машины — по сообщению на каждый.
pub fn port_conflicts(vm: &Vm) -> Vec<String> {
    if vm.net != NetMode::Nat {
        return vec![];
    }
    vm.forwards
        .iter()
        .filter(|f| f.host > 0)
        .filter_map(|f| {
            let who = port_holder(f.host, f.udp, &vm.id)?;
            Some(t!("Порт {p} компьютера занят: {w} — выберите другой (syn-vm --set {id} forwards=…)", p = format!("{}{}", if f.udp { "udp:" } else { "" }, f.host), w = who, id = vm.id))
        })
        .collect()
}

/// Те же порты в настройках других машин (запущенных или нет).
pub fn port_shared_with(vm: &Vm) -> Vec<String> {
    let all = crate::model::load_all();
    vm.forwards
        .iter()
        .filter_map(|f| {
            let o = all.iter().find(|o| o.id != vm.id && o.forwards.iter().any(|g| g.host == f.host && g.udp == f.udp))?;
            Some(t!("порт {p} есть и у машины «{n}» — одновременно они не запустятся", p = f.host, n = o.name))
        })
        .collect()
}

/// virtiofsd общей папки: завершается сам, когда QEMU закрывает соединение.
fn virtiofs_sock(i: usize) -> String {
    if i == 0 { "virtiofs.sock".into() } else { format!("virtiofs{i}.sock") }
}

/// Свой smbd для общих папок по SMB: конфигурация в каталоге запуска, команда для guestfwd
/// (QEMU запускает её на каждое соединение гостя с 10.0.2.4:445, как для smb=).
fn smb_command(vm: &Vm) -> Result<String> {
    let bin = smbd().context(t!("Для общей папки по SMB нужен пакет samba (smbd)"))?;
    let dir = vm.run_dir().join("smb");
    for d in ["private", "lock", "state", "cache", "ncalrpc"] {
        std::fs::create_dir_all(dir.join(d))?;
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());
    let mut conf = format!(
        "[global]\nprivate dir={d}/private\ninterfaces=127.0.0.1\nbind interfaces only=yes\npid directory={d}\nlock directory={d}/lock\nstate directory={d}/state\ncache directory={d}/cache\nncalrpc dir={d}/ncalrpc\nlog file={d}/log.smbd\nsmb passwd file={d}/smbpasswd\nsecurity = user\nmap to guest = Bad User\nload printers = no\nprinting = bsd\ndisable spoolss = yes\nusershare max shares = 0\n",
        d = dir.display()
    );
    for (path, name) in vm.share_dirs().iter().zip(vm.share_names(true)) {
        conf += &format!("[{name}]\npath={path}\nread only=no\nguest ok=yes\nforce user={user}\n");
    }
    let cf = dir.join("smb.conf");
    std::fs::write(&cf, conf)?;
    Ok(format!("{} -l {} -s {}", bin.display(), dir.display(), cf.display()))
}

fn start_virtiofsd(vm: &Vm) -> Result<()> {
    for (i, d) in vm.share_dirs().iter().enumerate() {
        start_one_virtiofsd(vm, i, d)?;
    }
    Ok(())
}

fn start_one_virtiofsd(vm: &Vm, i: usize, shared: &str) -> Result<()> {
    let bin = virtiofsd().context(t!("Для virtiofs нужен пакет virtiofsd"))?;
    let sock = vm.run_dir().join(virtiofs_sock(i));
    let log = std::fs::File::create(vm.dir().join(if i == 0 { "virtiofsd.log".to_string() } else { format!("virtiofsd{i}.log") }))?;
    let mut child = Command::new(bin)
        .arg(format!("--socket-path={}", sock.display()))
        .arg(format!("--shared-dir={shared}"))
        .args(["--sandbox=none", "--cache=auto"])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("virtiofsd")?;
    std::thread::spawn(move || child.wait());
    for _ in 0..100 {
        if sock.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    bail!("virtiofsd: {}", t!("сокет не появился"))
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

/// Настройки, с которыми запущен QEMU машины (в папке запуска).
const LAUNCHED: &str = "launched.toml";

/// С какими настройками работает машина (`None` — выключена). Машина, запущенная до появления
/// этого файла (или из чужой командной строки), — по командной строке QEMU: видеокарта и syn-gpu.
pub fn launched(vm: &Vm) -> Option<Vm> {
    let pid = pid(vm)?;
    if let Some(mut l) = std::fs::read_to_string(vm.run_dir().join(LAUNCHED)).ok().and_then(|s| toml::from_str::<Vm>(&s).ok()) {
        l.id = vm.id.clone();
        return Some(l);
    }
    let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = cmd.split(|b| *b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    let mut l = vm.clone();
    let (display, gpu) = display_of_args(&args);
    l.display = display;
    l.gpu3d = gpu && display != DisplayModel::SynGpu;
    Some(l)
}

/// Видеокарта по аргументам QEMU и есть ли устройство syn-gpu (vfio-user-pci).
pub fn display_of_args(args: &[String]) -> (DisplayModel, bool) {
    let gpu = args.iter().any(|a| a.contains("vfio-user-pci"));
    let dev = |name: &str| args.iter().any(|a| a.split(',').next() == Some(name));
    let d = if dev("virtio-vga") || dev("virtio-vga-gl") {
        DisplayModel::Virtio
    } else if dev("qxl-vga") {
        DisplayModel::Qxl
    } else if dev("VGA") {
        DisplayModel::Std
    } else if gpu {
        DisplayModel::SynGpu
    } else {
        DisplayModel::Std
    };
    (d, gpu)
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
    fn display_from_args() {
        let a = |s: &str| split_args(s);
        assert_eq!(display_of_args(&a("-vga none -device virtio-vga,xres=1280,yres=800")), (DisplayModel::Virtio, false));
        assert_eq!(display_of_args(&a(r#"-vga none -device virtio-vga,xres=1280 -device {"driver":"vfio-user-pci"}"#)), (DisplayModel::Virtio, true));
        assert_eq!(display_of_args(&a(r#"-vga none -device {"driver":"vfio-user-pci","socket":{}}"#)), (DisplayModel::SynGpu, true));
        assert_eq!(display_of_args(&a("-vga none -device VGA,vgamem_mb=64")), (DisplayModel::Std, false));
        // Наши аргументы QEMU: что запишется в командную строку, то и распознаётся.
        let mut vm = OsKind::Linux.preset();
        vm.id = "t".into();
        vm.firmware = Firmware::Bios;
        for d in DisplayModel::ALL {
            vm.display = d;
            let mut args = args(&vm).unwrap();
            if vm.gpu_on() {
                args.push(r#"{"driver":"vfio-user-pci"}"#.into());
            }
            assert_eq!(display_of_args(&args).0, d);
        }
    }

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
        // Без -daemonize: QEMU — отдельной сессией, вывод в qemu.log (см. start).
        assert!(!a.contains(&"-daemonize".to_string()));
    }

    #[test]
    fn several_shares() {
        let base = std::env::temp_dir().join("syn-vm-test-shares");
        let (a, b) = (base.join("Один"), base.join("Два"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut vm = OsKind::Windows11.preset();
        vm.id = "shares-test".into();
        vm.firmware = Firmware::Bios;
        vm.tpm = false;
        vm.tools_cd = false;
        vm.shared_dir = a.to_string_lossy().into_owned();
        vm.shares = vec![b.to_string_lossy().into_owned()];
        assert_eq!(vm.share(), Some(ShareMode::Smb));
        assert_eq!(vm.share_names(true), vec!["qemu".to_string(), "Два".to_string()]);
        if smbd().is_some() {
            let j = args(&vm).unwrap().join(" ");
            assert!(j.contains("guestfwd=tcp:10.0.2.4:445-cmd:"));
            let conf = std::fs::read_to_string(vm.run_dir().join("smb/smb.conf")).unwrap();
            assert!(conf.contains("[qemu]") && conf.contains("[Два]"));
        }
        vm.os = OsKind::Linux;
        vm.share_mode = ShareMode::P9;
        let j = args(&vm).unwrap().join(" ");
        assert!(j.contains("mount_tag=hostshare,") && j.contains("mount_tag=hostshare1,"));
    }

    #[test]
    fn syn_tools_args() {
        let mut vm = OsKind::Windows11.preset();
        vm.id = "w".into();
        vm.firmware = Firmware::Bios;
        vm.tpm = false;
        vm.tools_cd = false;
        assert_eq!(vm.guest_tools, crate::model::GuestTools::SynVm);
        let j = args(&vm).unwrap().join(" ");
        assert!(!j.contains("virtio-balloon"));
        assert!(!j.contains("org.qemu.guest_agent.0"));
        assert!(!j.contains("com.redhat.spice.0"));
        assert!(j.contains("org.syn.vm.agent.0") && j.contains("guestfwd=tcp:10.0.2.100:7777"));
        vm.guest_tools = crate::model::GuestTools::VirtioWin;
        let j = args(&vm).unwrap().join(" ");
        assert!(j.contains("virtio-balloon") && j.contains("org.qemu.guest_agent.0") && j.contains("com.redhat.spice.0"));
    }
}
