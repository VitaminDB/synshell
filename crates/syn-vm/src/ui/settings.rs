//! Настройки машины. Сохраняются сразу; у работающей — вступают в силу при следующем
//! запуске (привод меняется на ходу). Страница не перестраивается от каждого
//! изменения — иначе ползунок пересоздавался бы под пальцем; живые части
//! (списки дисков, портов, привод) перестраиваются сами.

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, row, row_wide, W};
use crate::model::{self, DiskBus, DisplayModel, Firmware, NetMode, NicModel, OsKind, PortForward, Vm, PALETTE};
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
        ],
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
                    .child(Text::new(t!("localhost:{h} → гость:{g}", h = f.host, g = f.guest)).max_lines(1).class("row-title grow"))
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
            v.forwards.push(PortForward { udp: false, host, guest });
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
    parts::section(
        icons::DEVICES,
        t!("Экран и устройства"),
        vec![
            row(t!("Видеокарта"), t!("virtio-vga меняет разрешение под окно; Windows лучше со стандартной"), display),
            row(t!("Разрешение под окно"), t!("Гость получает размер окна (нужен драйвер virtio-gpu или QXL)"), Toggle::with_state(vm.auto_resize).on_change(move |b| state::edit(st, |v| v.auto_resize = b))),
            row(t!("Звук"), t!("Через PipeWire компьютера"), Toggle::with_state(vm.sound).on_change(move |b| state::edit(st, |v| v.sound = b))),
            row(t!("Общий буфер обмена"), t!("Копирование между гостем и компьютером; в госте нужен spice-vdagent"), Toggle::with_state(vm.clipboard).on_change(move |b| state::edit(st, |v| v.clipboard = b))),
            row(
                t!("Общая папка"),
                if vm.shared_dir.is_empty() {
                    t!("В госте: mount -t 9p -o trans=virtio hostshare /mnt")
                } else {
                    t!("{p} · в госте: mount -t 9p -o trans=virtio hostshare /mnt", p = shared_name)
                },
                share,
            ),
        ],
    )
}

fn advanced(st: St, vm: &Vm) -> W {
    let field = TextField::with_text(vm.extra_args.clone())
        .placeholder(t!("например: -device usb-host,vendorid=0x1234,productid=0x5678"))
        .submit_on_focus_lost(true)
        .on_submit(move |s: &str| {
            let s = s.trim().to_string();
            state::edit(st, |v| v.extra_args = s);
        })
        .class("field mono-field");
    parts::section(icons::TERMINAL, t!("Дополнительно"), vec![row_wide(t!("Аргументы QEMU"), t!("Добавляются в конец командной строки — для USB-устройств, своих дисков и прочего"), field)])
}
