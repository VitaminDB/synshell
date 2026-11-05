//! Настройки машины. Сохраняются сразу; у работающей — вступают в силу при следующем
//! запуске (привод меняется на ходу). Страница не перестраивается от каждого
//! изменения — иначе ползунок пересоздавался бы под пальцем; живые части
//! (списки дисков, портов, привод) перестраиваются сами.

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, row, row_wide, W};
use crate::model::{self, DiskBus, DisplayModel, Firmware, HostDev, NetMode, NicModel, OsKind, PortForward, ShareMode, Vm, PALETTE};
use crate::state::{self, icons, St};

pub fn page(st: St, narrow: bool) -> W {
    let body = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = st.sel.get() else { return vec![] };
        let Some(vm) = st.vm(&id) else { return vec![] };
        let mut col = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(running_note(st, id.clone()));
        col = col.child(general(st, &vm));
        col = col.child(hardware(st, &vm));
        col = col.child(live(st, move |st, vm| disks(st, vm)));
        col = col.child(live(st, move |st, vm| cdrom(st, vm)));
        col = col.child(live(st, move |st, vm| boot(st, vm)));
        col = col.child(network(st, &vm));
        col = col.child(live(st, move |st, vm| devices(st, vm)));
        col = col.child(live(st, move |st, vm| guest(st, vm)));
        col = col.child(live(st, move |st, vm| usb(st, vm)));
        col = col.child(host_devices(st, &vm));
        col = col.child(advanced(st, &vm));
        vec![boxed(DecoratedBox::new().child(col).class(if narrow { "page narrow" } else { "page settings" }))]
    });
    boxed(ScrollView::new().vertical().child(body))
}

/// Часть страницы, которая перестраивается при каждом изменении машины.
fn live(st: St, f: impl Fn(St, &Vm) -> W + Send + Sync + 'static) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let vms = st.vms.get();
        st.live.get();
        let Some(id) = st.sel.get_untracked() else { return vec![] };
        match vms.iter().find(|v| v.id == id) {
            Some(vm) => vec![f(st, vm)],
            None => vec![],
        }
    }))
}

fn off(st: St, vm: &Vm) -> bool {
    !st.power(&vm.id).on()
}

fn running_note(st: St, id: String) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if !st.live.get().get(&id).is_some_and(|l| l.power.on()) {
            return vec![];
        }
        vec![boxed(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(icons::INFO).class("banner-icon"))
                        .child(Text::new(t!("Машина работает: изменения вступят в силу после её выключения и запуска. Образ в приводе меняется сразу.")).max_lines(3).class("row-hint grow")),
                )
                .class("banner info"),
        )]
    }))
}

fn dropdown<T: Copy + PartialEq + Send + Sync + 'static>(all: &[T], cur: T, title: fn(T) -> &'static str, key: fn(T) -> String, width: f32, f: impl Fn(T) + Send + Sync + 'static) -> Dropdown {
    let mut dd = Dropdown::new().width(width);
    for v in all {
        dd = dd.item(DropdownItem::new(key(*v), syngui::i18n::t(title(*v))));
    }
    let items: Vec<T> = all.to_vec();
    dd.selected(key(cur)).on_change(move |s: &str| {
        if let Some(v) = items.iter().find(|v| key(**v) == s) {
            f(*v);
        }
    })
}

fn general(st: St, vm: &Vm) -> W {
    let name = TextField::with_text(vm.name.clone())
        .submit_on_focus_lost(true)
        .on_submit(move |s: &str| {
            let s = s.trim().to_string();
            if !s.is_empty() {
                state::edit(st, |v| v.name = s);
            }
        })
        .width(260.0)
        .class("field");
    let id = vm.id.clone();
    let colors = Reactive::new(move || -> Vec<W> {
        let vms = st.vms.get();
        let cur = vms.iter().find(|v| v.id == id).map(|v| v.color.clone()).unwrap_or_default();
        let mut r = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for c in PALETTE {
            let sel = cur == c;
            r = r.child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || state::edit(st, |v| v.color = c.to_string()))
                    .child(
                        DecoratedBox::new()
                            .child(if sel { boxed(Icon::new(icons::CHECK).class("swatch-check")) } else { boxed(DecoratedBox::new()) })
                            .class(if sel { "swatch active" } else { "swatch" })
                            .style("background-color", Color::from_hex(c)),
                    ),
            );
        }
        vec![boxed(r)]
    });
    let os = dropdown(&OsKind::ALL, vm.os, OsKind::title, |o| format!("{o:?}"), 220.0, move |o| state::edit(st, |v| v.os = o));
    parts::section(
        icons::EDIT,
        t!("Основное"),
        vec![row(t!("Имя"), String::new(), name), row(t!("Цвет"), t!("Карточка и значок машины"), colors), row(t!("Система"), t!("От неё зависят часы (Windows — местное время) и подсказки"), os)],
    )
}

fn hardware(st: St, vm: &Vm) -> W {
    let host_cpu = model::host_cpus();
    let host_mem = model::host_memory_mb();
    let cpus = use_value(vm.cpus as f32);
    let mem = use_value(vm.memory_mb as f32);
    let id = vm.id.clone();
    let cpu_label = {
        let id = id.clone();
        Reactive::new(move || -> Vec<W> {
            let v = st.vms.get().into_iter().find(|v| v.id == id).map(|v| v.cpus).unwrap_or(1);
            vec![boxed(Text::new(parts::cores(v)).class("value-label"))]
        })
    };
    let mem_label = Reactive::new(move || -> Vec<W> {
        let v = st.vms.get().into_iter().find(|v| v.id == id).map(|v| v.memory_mb).unwrap_or(0);
        vec![boxed(Text::new(parts::mem_mb(v)).class("value-label"))]
    });
    let cpu_slider = Slider::new().range(1.0, host_cpu as f32).step(1.0).value(cpus).width(240.0).on_change(move |v| {
        let n = v.round().max(1.0) as u32;
        state::edit(st, |vm| vm.cpus = n);
    });
    let mem_slider = Slider::new().range(512.0, host_mem.max(1024) as f32).step(256.0).value(mem).width(240.0).on_change(move |v| {
        let n = ((v / 256.0).round() * 256.0).max(512.0) as u32;
        state::edit(st, |vm| vm.memory_mb = n);
    });
    let ctl = |s: Slider, label: Reactive| Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(s).child(label);
    parts::section(
        icons::CPU,
        t!("Процессор и память"),
        vec![
            row(t!("Ядра процессора"), t!("У компьютера — {n}; оставьте одно-два себе", n = host_cpu), ctl(cpu_slider, cpu_label)),
            row(t!("Память"), t!("У компьютера — {m}; гость берёт её по мере надобности", m = parts::mem_mb(host_mem)), ctl(mem_slider, mem_label)),
        ],
    )
}

fn use_value(v: f32) -> f32 {
    v
}

fn disks(st: St, vm: &Vm) -> W {
    let can = off(st, vm);
    let mut rows: Vec<W> = Vec::new();
    for (i, d) in vm.disks.iter().enumerate() {
        let path = vm.disk_path(d);
        let virt = model::disk_virtual_size(&path);
        use std::os::unix::fs::MetadataExt;
        let used = std::fs::metadata(&path).map(|m| m.blocks() * 512).unwrap_or(0);
        let frac = virt.map(|v| used as f32 / v.max(1) as f32).unwrap_or(0.0);
        let title = format!("{} · {}", parts::file_name(&d.file), virt.map(parts::bytes).unwrap_or_else(|| "—".into()));
        let hint = t!("Занято на диске компьютера: {u}", u = parts::bytes(used));
        let mut acts = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if can && path.extension().is_some_and(|e| e == "qcow2") {
            let p = path.clone();
            let cur_gb = virt.unwrap_or(0) / (1 << 30);
            let id = vm.id.clone();
            acts = acts.child(btn(icons::ADD, t!("+10 ГБ"), "ghost", move || {
                let p = p.clone();
                state::background(st, &id, t!("Увеличение диска…"), move || {
                    model::resize_disk(&p, cur_gb + 10)?;
                    run_on_main_thread(move || st.vms.update(|_| {}));
                    Ok(Some(t!("Диск увеличен до {n} ГБ — расширьте раздел в госте", n = cur_gb + 10)))
                });
            }));
        }
        if can && i > 0 {
            acts = acts.child(super::parts::icon_btn(icons::CLOSE, t!("Отключить диск (файл останется)"), move || state::edit(st, |v| {
                if i < v.disks.len() {
                    v.disks.remove(i);
                }
            })));
        }
        let accent = vm.accent();
        rows.push(boxed(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(parts::badge(icons::STORAGE, Some(accent), 34.0))
                        .child(Column::new().gap(2.0).child(Text::new(title).max_lines(1).class("row-title")).child(Text::new(hint).max_lines(1).class("row-hint")).class("grow"))
                        .child(acts),
                )
                .child(parts::meter(frac, accent, 6.0))
                .class("row"),
        ));
    }
    if vm.disks.is_empty() {
        rows.push(boxed(Text::new(t!("Дисков нет — машина грузится только с привода")).class("row-hint row")));
    }
    let bus = dropdown(&DiskBus::ALL, vm.disk_bus, DiskBus::title, |b| format!("{b:?}"), 160.0, move |b| state::edit(st, |v| v.disk_bus = b));
    rows.push(row(t!("Подключение"), t!("virtio — быстрее всего, но Windows нужен драйвер; SATA работает везде"), bus));
    if can {
        let id = vm.id.clone();
        rows.push(boxed(Row::new().child(btn(icons::ADD, t!("Добавить диск 32 ГБ"), "soft", move || {
            let Some(mut vm) = st.vm(&id) else { return };
            state::background(st, &id, t!("Создание диска…"), move || {
                model::add_disk(&mut vm, 32)?;
                run_on_main_thread(move || state::reload(st));
                Ok(Some(t!("Диск добавлен")))
            });
        })).class("row")));
    }
    parts::section(icons::STORAGE, t!("Диски"), rows)
}

fn cdrom(st: St, vm: &Vm) -> W {
    let id = vm.id.clone();
    let id2 = vm.id.clone();
    let name = if vm.cdrom.is_empty() { t!("Пусто") } else { parts::file_name(&vm.cdrom) };
    let missing = !vm.cdrom.is_empty() && !std::path::Path::new(&vm.cdrom).exists();
    let hint = if missing { t!("Файл не найден: {p}", p = vm.cdrom) } else if vm.cdrom.is_empty() { t!("Вставьте образ установки или драйверов") } else { vm.cdrom.clone() };
    let mut acts = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(btn(icons::DISC, t!("Выбрать образ…"), "soft", move || {
        let id = id.clone();
        std::thread::spawn(move || {
            if let Some(p) = parts::choose_file(&t!("Образ диска"), &t!("Образы дисков"), &["*.iso", "*.ISO", "*.img"], false) {
                run_on_main_thread(move || state::set_cdrom(st, &id, p));
            }
        });
    }));
    if !vm.cdrom.is_empty() {
        acts = acts.child(super::parts::icon_btn(icons::EJECT, t!("Извлечь"), move || state::set_cdrom(st, &id2, String::new())));
    }
    parts::section(
        icons::DISC,
        t!("Привод"),
        vec![boxed(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(parts::badge(icons::DISC, None, 34.0))
                .child(Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("row-title")).child(Text::new(hint).max_lines(2).class(if missing { "row-hint bad" } else { "row-hint" })).class("grow"))
                .child(acts)
                .class("row"),
        )],
    )
}

fn boot(st: St, vm: &Vm) -> W {
    let can = off(st, vm);
    let fw = dropdown(&Firmware::ALL, vm.firmware, Firmware::title, |f| format!("{f:?}"), 200.0, move |f| {
        state::edit(st, |v| {
            v.firmware = f;
            if f == Firmware::Bios {
                v.tpm = false;
            }
        })
    });
    let fw: W = if can { boxed(fw) } else { boxed(Text::new(syngui::i18n::t(vm.firmware.title())).class("value-label")) };
    let fw = DecoratedBox::new().child(fw);
    parts::section(
        icons::SECURITY,
        t!("Загрузка и безопасность"),
        vec![
            row(t!("Сначала с привода"), t!("Включите, чтобы загрузиться с образа, когда на диске уже есть система"), Toggle::with_state(vm.boot_cdrom).on_change(move |b| state::edit(st, |v| v.boot_cdrom = b))),
            row(t!("Прошивка"), t!("Меняется только у выключенной машины; установленная система может перестать загружаться"), fw),
            row(t!("TPM 2.0"), t!("Нужен Windows 11 и BitLocker; эмулирует swtpm"), Toggle::with_state(vm.tpm).on_change(move |b| state::edit(st, |v| v.tpm = b))),
        ]
        .into_iter()
        .chain(vm.tpm.then(|| {
            row(
                t!("TPM компьютера"),
                if std::path::Path::new("/dev/tpm0").exists() { t!("Вместо эмулятора — настоящий TPM (/dev/tpm0); пока машина работает, у компьютера его нет") } else { t!("У компьютера нет TPM") },
                Toggle::with_state(vm.tpm_host).disabled(!std::path::Path::new("/dev/tpm0").exists()).on_change(move |b| state::edit(st, |v| v.tpm_host = b)),
            )
        }))
        .collect(),
    )
}

fn network(st: St, vm: &Vm) -> W {
    let mode = Toggle::with_state(vm.net == NetMode::Nat).on_change(move |b| state::edit(st, |v| v.net = if b { NetMode::Nat } else { NetMode::None }));
    let nic = dropdown(&NicModel::ALL, vm.nic, NicModel::title, |n| format!("{n:?}"), 180.0, move |n| state::edit(st, |v| v.nic = n));
    let host = TextField::new().placeholder(t!("Порт хоста")).input_filter(|c: char| c.is_ascii_digit()).width(110.0);
    let host_sig = use_signal_once("fwd-host");
    let guest_sig = use_signal_once("fwd-guest");
    let host = host.on_change(move |s: &str| host_sig.set(s.to_string())).class("field");
    let guest = TextField::new().placeholder(t!("Порт гостя")).input_filter(|c: char| c.is_ascii_digit()).width(110.0).on_change(move |s: &str| guest_sig.set(s.to_string())).class("field");
    let add = btn(icons::ADD, t!("Пробросить"), "soft", move || {
        let (h, g) = (host_sig.get_untracked().parse::<u16>().unwrap_or(0), guest_sig.get_untracked().parse::<u16>().unwrap_or(0));
        if h == 0 || g == 0 {
            state::toast_err(st, t!("Укажите оба порта"));
            return;
        }
        add_forward(st, h, g);
    });
    let list = live(st, move |st, vm| {
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (i, f) in vm.forwards.iter().enumerate() {
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().child(Text::new(if f.udp { "UDP" } else { "TCP" }).class("tag-label")).class("tag"))
                    .child(Text::new(if f.public { t!("все адреса:{h} → гость:{g}", h = f.host, g = f.guest) } else { t!("localhost:{h} → гость:{g}", h = f.host, g = f.guest) }).max_lines(1).class("row-title grow"))
                    .child(Text::new(t!("наружу")).class("row-hint"))
                    .child(Toggle::with_state(f.public).on_change(move |b| state::edit(st, |v| {
                        if let Some(f) = v.forwards.get_mut(i) {
                            f.public = b;
                        }
                    })))
                    .child(super::parts::icon_btn(icons::CLOSE, t!("Убрать"), move || state::edit(st, |v| {
                        if i < v.forwards.len() {
                            v.forwards.remove(i);
                        }
                    }))),
            );
        }
        if vm.forwards.is_empty() {
            col = col.child(Text::new(t!("Порты не проброшены — гость виден только изнутри")).class("row-hint"));
        }
        boxed(col)
    });
    let presets = Row::new()
        .gap(8.0)
        .child(btn("", t!("SSH 2222→22"), "ghost", move || add_forward(st, 2222, 22)))
        .child(btn("", t!("RDP 3389"), "ghost", move || add_forward(st, 3389, 3389)))
        .child(btn("", t!("HTTP 8080→80"), "ghost", move || add_forward(st, 8080, 80)));
    parts::section(
        icons::LAN,
        t!("Сеть"),
        vec![
            row(t!("Интернет"), t!("NAT: гость выходит в сеть через компьютер"), mode),
            row(t!("Сетевая карта"), t!("Intel e1000e понимает любая система без драйверов"), nic),
            row_wide(
                t!("Проброс портов"),
                t!("Подключайтесь к службам гостя через порт компьютера"),
                Column::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(list)
                    .child(Flex::new().direction(FlexDirection::Row).wrap().gap(8.0).child(host).child(guest).child(add))
                    .child(presets),
            ),
        ],
    )
}

/// Сигналы полей проброса — по одному на окно (страница пересоздаётся при смене вкладки).
fn use_signal_once(key: &'static str) -> RwSignal<String> {
    thread_local! {
        static M: std::cell::RefCell<std::collections::HashMap<&'static str, RwSignal<String>>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    M.with(|m| *m.borrow_mut().entry(key).or_insert_with(|| use_signal(String::new())))
}

fn add_forward(st: St, host: u16, guest: u16) {
    let used = st.vms.get_untracked().iter().flat_map(|v| v.forwards.iter().map(|f| (v.id.clone(), f.host))).any(|(id, h)| h == host && Some(id) != st.sel.get_untracked());
    if used {
        state::toast_err(st, t!("Порт {p} уже занят другой машиной", p = host));
        return;
    }
    state::edit(st, |v| {
        if !v.forwards.iter().any(|f| f.host == host) {
            v.forwards.push(PortForward { udp: false, host, guest, public: false });
        }
    });
}

fn devices(st: St, vm: &Vm) -> W {
    let display = dropdown(&DisplayModel::ALL, vm.display, DisplayModel::title, |d| format!("{d:?}"), 200.0, move |d| state::edit(st, |v| v.display = d));
    let id = vm.id.clone();
    let shared_name = if vm.shared_dir.is_empty() { t!("Не выбрана") } else { vm.shared_dir.clone() };
    let mut share = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(btn(icons::FOLDER_OPEN, t!("Выбрать…"), "soft", move || {
        let _ = &id;
        std::thread::spawn(move || {
            if let Some(p) = parts::choose_file(&t!("Общая папка"), "", &[], true) {
                run_on_main_thread(move || state::edit(st, |v| v.shared_dir = p));
            }
        });
    }));
    if !vm.shared_dir.is_empty() {
        share = share.child(super::parts::icon_btn(icons::CLOSE, t!("Не делиться"), move || state::edit(st, |v| v.shared_dir.clear())));
    }
    let mode = vm.share_mode.resolve(vm.os);
    let share_hint = match mode {
        ShareMode::Virtiofs => t!("В Windows: драйвер virtio-fs и WinFsp с диска «Гостевые инструменты», служба VirtioFsSvc — диск Z:. В Linux: mount -t virtiofs hostshare /mnt. Снимки с памятью при virtiofs недоступны."),
        ShareMode::Smb => t!("В госте: \\\\10.0.2.4\\qemu (нужен пакет samba на компьютере)"),
        _ => t!("В госте: mount -t 9p -o trans=virtio hostshare /mnt (Windows 9p не умеет — выберите virtiofs или SMB)"),
    };
    let share_mode = dropdown(&ShareMode::ALL, vm.share_mode, ShareMode::title, |m| format!("{m:?}"), 160.0, move |m| state::edit(st, |v| v.share_mode = m));
    let audio = |sources: bool, cur: String| -> W {
        let nodes = crate::devices::audio_nodes(sources);
        let mut dd = Dropdown::new().width(260.0).item(DropdownItem::new(String::new(), t!("По умолчанию")));
        for (n, d) in &nodes {
            dd = dd.item(DropdownItem::new(n.clone(), if d.is_empty() { n.clone() } else { d.clone() }));
        }
        boxed(dd.selected(cur).on_change(move |s: &str| {
            let s = s.to_string();
            state::edit(st, |v| if sources { v.audio_in = s } else { v.audio_out = s });
        }))
    };
    parts::section(
        icons::DEVICES,
        t!("Экран и устройства"),
        vec![
            row(t!("Видеокарта"), t!("virtio-vga — быстрее и меняет разрешение под окно (Windows: драйвер VirtIO GPU из «Гостевых инструментов»); стандартный VGA — медленно"), display),
            row(t!("Разрешение под окно"), t!("Гость получает размер окна (нужен драйвер virtio-gpu или QXL)"), Toggle::with_state(vm.auto_resize).on_change(move |b| state::edit(st, |v| v.auto_resize = b))),
            row(
                t!("3D-ускорение (syn-gpu)"),
                if crate::qemu::syn_gpu_bin().is_some() {
                    t!("Своя видеокарта для игр: Vulkan и DirectX через DXVK, кадры игр — прямо в окно. Windows с драйвером syn-gpu: прошивка UEFI без Secure Boot, в госте bcdedit /set testsigning on")
                } else {
                    t!("Нужен сервер syn-gpu (проект qemu-vulkan-driver): переменная SYN_GPU или syn-gpu в PATH")
                },
                Toggle::with_state(vm.gpu3d).on_change(move |b| {
                    let mut sb = false;
                    state::edit(st, |v| {
                        v.gpu3d = b;
                        // Драйверу syn-gpu нужна тестовая подпись — с Secure Boot она не включится.
                        if b && v.firmware == model::Firmware::UefiSecure {
                            v.firmware = model::Firmware::Uefi;
                            sb = true;
                        }
                    });
                    if sb {
                        state::toast(st, t!("Secure Boot выключен: драйверу syn-gpu нужна тестовая подпись"));
                    }
                }),
            ),
            {
                // Гостевые инструменты syn-gpu: драйвер видеокарты в госте одной кнопкой.
                let (a, b) = (vm.id.clone(), vm.id.clone());
                let on = st.power(&vm.id).on();
                let acts = Row::new()
                    .gap(8.0)
                    .child(if on { btn(icons::SYNC, t!("Установить"), "soft", move || state::gpu_tools(st, &a, true)) } else { parts::btn_disabled(icons::SYNC, t!("Установить"), "soft") })
                    .child(if on { btn(icons::INFO, t!("Состояние в госте"), "ghost", move || state::gpu_tools(st, &b, false)) } else { parts::btn_disabled(icons::INFO, t!("Состояние в госте"), "ghost") });
                row(
                    t!("Гостевые инструменты syn-gpu"),
                    if crate::gputools::iso().is_some() { t!("Драйвер видеокарты в Windows: образ в привод, установка через гостевой агент, при необходимости — перезагрузка") } else { t!("Нет образа syn-gpu-guest-tools.iso (SYN_GPU_TOOLS, /usr/share/syn-gpu или ~/.local/share/syn-gpu)") },
                    acts,
                )
            },
            row(t!("Звук"), t!("Через PipeWire компьютера"), Toggle::with_state(vm.sound).on_change(move |b| state::edit(st, |v| v.sound = b))),
            row(t!("Вывод звука"), t!("Динамики или наушники для гостя"), DecoratedBox::new().child(audio(false, vm.audio_out.clone()))),
            row(t!("Микрофон"), t!("Что гость слышит как вход"), DecoratedBox::new().child(audio(true, vm.audio_in.clone()))),
            row(t!("Общий буфер обмена"), t!("Копирование между гостем и компьютером; в госте нужен spice-vdagent"), Toggle::with_state(vm.clipboard).on_change(move |b| state::edit(st, |v| v.clipboard = b))),
            row(t!("Общая папка"), shared_name, share),
            row(t!("Как гость видит папку"), share_hint, share_mode),
        ],
    )
}

/// USB-устройства компьютера: переключатель на каждый порт. Отданный порт пропадает у компьютера,
/// пока машина работает; выбор привязан к порту, поэтому телефон в режимах fastboot и recovery
/// остаётся у гостя.
fn usb(st: St, vm: &Vm) -> W {
    let host = crate::usb::list();
    let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let mut ports: Vec<(String, String, String, bool)> = host
        .iter()
        .filter(|d| !d.hub)
        .map(|d| {
            let note = if d.writable { format!("{}:{}", d.vendor, d.product) } else { t!("{v}:{p} · нужен доступ (спросит пароль)", v = d.vendor, p = d.product) };
            (d.port.clone(), d.name.clone(), note, true)
        })
        .collect();
    for u in &vm.usb {
        if !ports.iter().any(|p| p.0 == u.port) {
            let name = if u.name.is_empty() { t!("Устройство") } else { u.name.clone() };
            ports.push((u.port.clone(), name, t!("сейчас не подключено — подхватится само"), false));
        }
    }
    for (port, name, note, present) in ports {
        let on = vm.usb.iter().any(|u| u.port == port);
        let id = vm.id.clone();
        let p2 = port.clone();
        col = col.child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::USB).class(if present { "row-icon" } else { "row-icon dim" }))
                .child(
                    Column::new()
                        .gap(2.0)
                        .child(Text::new(name).max_lines(1).class("row-title"))
                        .child(Text::new(t!("порт {p} · {n}", p = port, n = note)).max_lines(1).class("row-hint"))
                        .class("grow"),
                )
                .child(Toggle::with_state(on).on_change(move |b| state::set_usb(st, &id, p2.clone(), b))),
        );
    }
    if host.iter().all(|d| d.hub) && vm.usb.is_empty() {
        col = col.child(Text::new(t!("USB-устройств не найдено")).class("row-hint"));
    }
    parts::section(
        icons::USB,
        t!("USB-устройства"),
        vec![
            row_wide(
                t!("Отдать машине"),
                t!("Устройство привязывается к порту компьютера: телефон в режиме fastboot или recovery остаётся у гостя. Пока машина работает, у компьютера его нет."),
                col,
            ),
            row(t!("Список устройств"), t!("Обновить после подключения нового устройства"), Row::new().child(btn(icons::REFRESH, t!("Обновить"), "ghost", move || state::reload(st)))),
        ],
    )
}

fn advanced(st: St, vm: &Vm) -> W {
    let field = TextField::with_text(vm.extra_args.clone())
        .placeholder(t!("например: -device virtio-rng-pci"))
        .submit_on_focus_lost(true)
        .on_submit(move |s: &str| {
            let s = s.trim().to_string();
            state::edit(st, |v| v.extra_args = s);
        })
        .class("field mono-field");
    parts::section(icons::TERMINAL, t!("Дополнительно"), vec![row_wide(t!("Аргументы QEMU"), t!("Добавляются в конец командной строки — для своих устройств, дисков и прочего"), field)])
}

/// Гостевой агент и «Гостевые инструменты» (virtio-win).
fn guest(st: St, vm: &Vm) -> W {
    let win = vm.os.is_windows();
    let agent_hint = if win {
        t!("Команды, файлы и выключение без сети (syn-vm --exec, MCP). В Windows ставится с диска «Гостевые инструменты»: virtio-win-guest-tools.exe")
    } else {
        t!("Команды, файлы и выключение без сети (syn-vm --exec, MCP). В госте нужен пакет qemu-guest-agent")
    };
    let mut rows = vec![row(t!("Гостевой агент"), agent_hint, Toggle::with_state(vm.guest_agent).on_change(move |b| state::edit(st, |v| v.guest_agent = b)))];
    let iso = crate::qemu::tools_iso();
    let tools_hint = match &iso {
        Some(p) => t!("Второй привод: {p} — драйверы virtio (диск, сеть, экран, общая папка), гостевой агент, spice-vdagent для буфера обмена", p = parts::file_name(&p.to_string_lossy())),
        None => t!("Образ virtio-win не найден — скачайте или выберите файл"),
    };
    let mut acts = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let id = vm.id.clone();
    acts = acts.child(btn(icons::FOLDER_OPEN, t!("Выбрать…"), "ghost", move || {
        let id = id.clone();
        std::thread::spawn(move || {
            if let Some(p) = parts::choose_file(&t!("Образ virtio-win"), &t!("Образы дисков"), &["*.iso", "*.ISO"], false) {
                let _ = std::fs::write(model::root().join("tools-iso.path"), p.as_bytes());
                run_on_main_thread(move || {
                    let _ = &id;
                    state::edit(st, |v| v.tools_cd = true);
                    state::reload(st);
                });
            }
        });
    }));
    if iso.is_none() {
        let id = vm.id.clone();
        acts = acts.child(btn(icons::SYNC, t!("Скачать"), "soft", move || {
            state::background(st, &id, t!("Скачивается virtio-win…"), move || {
                let p = crate::qemu::download_tools()?;
                run_on_main_thread(move || state::reload(st));
                Ok(Some(t!("Скачано: {p}", p = p.display())))
            });
        }));
    } else if st.power(&vm.id).on() {
        let id = vm.id.clone();
        let p = iso.clone().unwrap().to_string_lossy().into_owned();
        acts = acts.child(btn(icons::DISC, t!("В привод сейчас"), "soft", move || state::set_cdrom(st, &id, p.clone())));
    }
    acts = acts.child(DecoratedBox::new().class("grow")).child(Text::new(t!("Вставлять при запуске")).class("row-hint")).child(Toggle::with_state(vm.tools_cd).on_change(move |b| state::edit(st, |v| v.tools_cd = b)));
    rows.push(row_wide(t!("Гостевые инструменты"), tools_hint, acts));
    // Установка/обновление и проверка состояния в госте.
    let mut acts2 = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    let id = vm.id.clone();
    acts2 = acts2.child(btn(icons::SYNC, t!("Установить или обновить"), "soft", move || state::install_tools(st, &id)));
    if st.power(&vm.id).on() {
        let id = vm.id.clone();
        acts2 = acts2.child(btn(icons::INFO, t!("Состояние в госте"), "ghost", move || state::tools_status(st, &id)));
    }
    rows.push(row_wide(
        t!("Установка в госте"),
        if win { t!("Образ в привод и virtio-win-guest-tools.exe — сам, если агент уже отвечает") } else { t!("qemu-guest-agent и spice-vdagent — через агент или вручную") },
        acts2,
    ));
    // «Оптимизировать для Windows»: видео, агент, диск инструментов; советы.
    let (opt, changes, advice) = vm.optimized();
    if !changes.is_empty() || !advice.is_empty() {
        let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for c in changes.iter() {
            col = col.child(Text::new(format!("• {c}")).max_lines(4).class("row-hint"));
        }
        for c in advice.iter() {
            col = col.child(Text::new(format!("○ {c}")).max_lines(5).class("row-hint"));
        }
        if !changes.is_empty() {
            col = col.child(Row::new().child(btn(icons::CHECK, t!("Применить (при следующем запуске)"), "soft", move || {
                let o = opt.clone();
                state::edit(st, move |v| *v = o);
            })));
        }
        rows.push(row_wide(t!("Оптимизировать для Windows"), t!("Быстрее и удобнее: видео, агент, драйверы"), col));
    }
    if !win {
        rows.push(row_wide(
            t!("Установка в Linux"),
            t!("Агент, буфер обмена и разрешение под окно:"),
            Column::new()
                .gap(4.0)
                .child(Text::new("Arch: sudo pacman -S qemu-guest-agent spice-vdagent").class("mono"))
                .child(Text::new("Debian/Ubuntu: sudo apt install qemu-guest-agent spice-vdagent").class("mono"))
                .child(Text::new("Fedora: sudo dnf install qemu-guest-agent spice-vdagent").class("mono"))
                .child(Text::new("sudo systemctl enable --now qemu-guest-agent").class("mono")),
        ));
    }
    parts::section(icons::TERMINAL, t!("Гостевой агент и инструменты"), rows)
}

fn kind_title(kind: &str) -> String {
    match kind {
        "pci" => t!("PCI через VFIO — видеокарта, NVMe, сеть, звук: при выключенной машине, привязка к vfio-pci спросит пароль"),
        "input" => t!("Клавиатура и мышь напрямую — переключение компьютер ↔ гость: оба Ctrl"),
        "block" => t!("Диски и разделы компьютера"),
        "serial" => t!("Последовательные порты (гостю — USB-UART)"),
        _ => t!("USB по VID:PID — найдётся в любом гнезде (запасной вариант к порту)"),
    }
}

fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "pci" => icons::MEMORY,
        "input" => icons::KEYBOARD,
        "block" => icons::STORAGE,
        "serial" => icons::TERMINAL,
        _ => icons::USB,
    }
}

fn dev_kind(d: &HostDev) -> &'static str {
    match d {
        HostDev::Pci { .. } => "pci",
        HostDev::Input { .. } => "input",
        HostDev::Block { .. } => "block",
        HostDev::Serial { .. } => "serial",
        HostDev::UsbId { .. } => "usb-id",
    }
}

/// Устройства компьютера: что уже у машины и что можно отдать — с причинами, если нельзя.
fn host_devices(st: St, vm: &Vm) -> W {
    let id = vm.id.clone();
    let attached = live(st, move |st, vm| {
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for d in &vm.devices {
            let (d1, id) = (d.clone(), vm.id.clone());
            let name = if d.name().is_empty() { d.spec() } else { d.name().to_string() };
            let mode = match d {
                HostDev::Block { readonly: true, .. } => t!(" · только чтение"),
                HostDev::Pci { .. } => t!(" · при запуске"),
                _ => t!(" · на ходу"),
            };
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(kind_icon(dev_kind(d))).class("row-icon"))
                    .child(Column::new().gap(2.0).child(Text::new(name).max_lines(1).class("row-title")).child(Text::new(format!("{}{mode}", d.spec())).max_lines(1).class("row-hint")).class("grow"))
                    .child(super::parts::icon_btn(icons::CLOSE, t!("Вернуть компьютеру"), move || state::set_device(st, &id, d1.clone(), false))),
            );
        }
        if vm.devices.is_empty() {
            col = col.child(Text::new(t!("Машине отданы только USB-порты (раздел выше)")).class("row-hint"));
        }
        boxed(col)
    });
    let list = Reactive::new(move || -> Vec<W> {
        let Some(all) = st.host_devs.get() else {
            return vec![boxed(Row::new().child(btn(icons::SEARCH, t!("Показать устройства компьютера"), "soft", move || state::load_host_devices(st))))];
        };
        let vms = st.vms.get();
        let Some(vm) = vms.iter().find(|v| v.id == id) else { return vec![] };
        let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        let mut kind = "";
        for a in &all {
            if a.kind != kind {
                kind = a.kind;
                col = col.child(Text::new(kind_title(kind)).max_lines(2).class("row-title group-title"));
            }
            let on = vm.devices.iter().any(|d| d.same(&a.dev));
            let mut sub = a.detail.clone();
            if a.needs_access {
                sub.push_str(&t!(" · нужен доступ (спросит пароль)"));
            }
            let mut text = Column::new().gap(2.0).child(Text::new(a.title.clone()).max_lines(1).class("row-title"));
            if !sub.is_empty() {
                text = text.child(Text::new(sub).max_lines(2).class("row-hint"));
            }
            if !a.blocked.is_empty() {
                text = text.child(Text::new(t!("Нельзя: {r}", r = a.blocked)).max_lines(3).class("row-hint bad"));
            } else if !a.warn.is_empty() {
                text = text.child(Text::new(a.warn.clone()).max_lines(3).class("row-hint warn"));
            }
            let (dev, id2) = (a.dev.clone(), vm.id.clone());
            let mut r = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(kind_icon(a.kind)).class(if a.blocked.is_empty() { "row-icon" } else { "row-icon dim" })).child(text.class("grow"));
            if a.kind == "block" && a.blocked.is_empty() {
                let (dev2, id3) = (a.dev.clone(), vm.id.clone());
                let ro = vm.devices.iter().any(|d| matches!(d, HostDev::Block { readonly: true, .. }) && d.same(&a.dev));
                r = r.child(Text::new(t!("чтение")).class("row-hint")).child(Toggle::with_state(ro).on_change(move |b| {
                    if let HostDev::Block { path, name, .. } = dev2.clone() {
                        state::set_device(st, &id3, HostDev::Block { path, readonly: b, name }, true);
                    }
                }));
            }
            r = r.child(Toggle::with_state(on).disabled(!a.blocked.is_empty() && !on).on_change(move |b| state::set_device(st, &id2, dev.clone(), b)));
            col = col.child(r);
        }
        col = col.child(Row::new().child(btn(icons::REFRESH, t!("Обновить"), "ghost", move || state::load_host_devices(st))));
        vec![boxed(col)]
    });
    parts::section(
        icons::DEVICES,
        t!("Устройства компьютера"),
        vec![
            row_wide(t!("У машины"), t!("Отданное гостю пропадает у компьютера, пока машина работает"), DecoratedBox::new().child(attached)),
            row_wide(t!("Отдать машине"), t!("Системный диск, основной видеовыход и занятая видеокарта не отдаются; веб-камера, Bluetooth и смарт-карта — это USB (раздел выше)"), list),
        ],
    )
}
