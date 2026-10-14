//! Обзор машины: экран-превью, живые графики нагрузки, сведения о железе, действия.

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::LiveView;
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, W};
use crate::model::{self, NetMode, Vm};
use crate::qemu;
use crate::state::{self, icons, Confirm, Live, Power, St, Tab, HIST};

const CPU_COLOR: &str = "#4f9dff";
const MEM_COLOR: &str = "#ff9f43";

pub fn page(st: St, narrow: bool) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(id) = st.sel.get() else { return vec![] };
        let vms = st.vms.get();
        let Some(vm) = vms.iter().find(|v| v.id == id).cloned() else { return vec![] };
        let l = st.live.get().get(&id).cloned().unwrap_or_default();
        st.thumb_tick.get();
        let vw = viewport_size().get().width;
        let wide = !narrow && vw >= 1180.0;
        let left = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(preview(st, &vm, &l, narrow)).child(stats(&vm, &l, narrow));
        let right = Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(info(&vm)).child(actions(st, &vm, &l));
        let body: W = if wide {
            boxed(
                Row::new()
                    .gap(16.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(left.class("grow-3"))
                    .child(right.class("grow-2")),
            )
        } else {
            boxed(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(left).child(right))
        };
        vec![boxed(DecoratedBox::new().child(body).class(if narrow { "page narrow" } else { "page" }))]
    });
    boxed(ScrollView::new().vertical().child(content))
}

fn preview(st: St, vm: &Vm, l: &Live, narrow: bool) -> W {
    let id = vm.id.clone();
    let vw = viewport_size().get_untracked().width;
    let w = if narrow { vw - 28.0 } else if vw >= 1180.0 { (vw - 290.0 - 64.0) * 0.6 } else { vw - 290.0 - 56.0 };
    let h = (w.min(900.0) * 10.0 / 16.0).clamp(160.0, 520.0);
    let mut stack = Stack::new().fit(StackFit::Expand);
    match state::thumb(vm) {
        Some(f) => {
            let v = DecoratedBox::new().child(LiveView::new(f).class("thumb-img"));
            stack = stack.child(v.style("opacity", StyleValue::Number(if l.power.on() { 1.0 } else { 0.45 })));
        }
        None if !l.power.on() => {}
        None => {
            stack = stack.child(
                DecoratedBox::new()
                    .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(parts::os_icon(vm.os)).style("icon-color", vm.accent()).style("icon-size", StyleValue::px(h * 0.28))))
                    .style("background-color", vm.accent().with_alpha(0.1)),
            );
        }
    }
    let center: W = if l.power.on() {
        boxed(DecoratedBox::new().child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icons::MONITOR).class("pill-icon")).child(Text::new(t!("Открыть экран")).class("pill-label"))).class("glass-pill"))
    } else {
        boxed(DecoratedBox::new().child(Icon::new(icons::PLAY).class("big-play-icon")).class("big-play"))
    };
    stack = stack.child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(center));
    let on = l.power.on();
    boxed(
        GestureDetector::new()
            .cursor(CursorIcon::Pointer)
            .on_click(move || {
                if on {
                    super::set_tab(st, Tab::Screen);
                } else {
                    state::start(st, &id);
                }
            })
            .child(DecoratedBox::new().child(stack).clip(true).class("preview").style("height", StyleValue::px(h))),
    )
}

fn stats(vm: &Vm, l: &Live, narrow: bool) -> W {
    if !l.power.on() {
        let disk = parts::bytes(vm.used_bytes());
        let tiles = vec![
            tile(icons::CPU, t!("Процессор"), parts::cores(vm.cpus), syngui::i18n::t(vm.firmware.title())),
            tile(icons::MEMORY, t!("Память"), parts::mem_mb(vm.memory_mb), String::new()),
            tile(icons::STORAGE, t!("На диске"), disk, parts::disks(vm.disks.len())),
            tile(icons::SCHEDULE, t!("Запуск"), if vm.last_run == 0 { t!("ещё не было") } else { parts::ago(vm.last_run) }, String::new()),
        ];
        return boxed(Grid::new(if narrow { 2 } else { 4 }).gap(12.0).children(tiles));
    }
    let cpu_c = Color::from_hex(CPU_COLOR);
    let mem_c = Color::from_hex(MEM_COLOR);
    let uptime = model::now().saturating_sub(l.since);
    let chart = |title: String, value: String, sub: String, hist: Vec<f32>, color: Color| -> W {
        parts::card(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::End)
                        .child(Column::new().gap(1.0).child(Text::new(title).class("tile-label")).child(Text::new(value).class("stat-value").style("color", color)).class("grow"))
                        .child(Text::new(sub).max_lines(1).class("muted small")),
                )
                .child(parts::area_chart(hist, HIST, 100.0, color, 84.0)),
        )
    };
    let charts = vec![
        chart(t!("Процессор"), format!("{:.0}%", l.cpu), parts::cores(vm.cpus), l.cpu_hist.clone(), cpu_c),
        chart(t!("Память"), parts::bytes(l.rss), t!("из {m}", m = parts::mem_mb(vm.memory_mb)), l.mem_hist.clone(), mem_c),
    ];
    let tiles = vec![
        tile(icons::SCHEDULE, t!("Работает"), parts::duration(uptime), String::new()),
        tile(icons::STORAGE, t!("Диск"), format!("{}/с", parts::bytes(l.disk_rate as u64)), parts::bytes(vm.used_bytes())),
        tile(if l.power == Power::Paused { icons::PAUSE } else { icons::BOLT }, t!("Ускорение"), if qemu::kvm_available() { t!("KVM") } else { t!("Эмуляция") }, String::new()),
    ];
    boxed(
        Column::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Grid::new(if narrow { 1 } else { 2 }).gap(12.0).children(charts))
            .child(Grid::new(3).gap(12.0).children(tiles)),
    )
}

fn tile(icon: &str, label: String, value: String, sub: String) -> W {
    let mut c = Column::new()
        .gap(4.0)
        .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(icon).class("tile-icon")).child(Text::new(label).max_lines(1).class("tile-label")))
        .child(Text::new(value).max_lines(1).class("tile-value"));
    if !sub.is_empty() {
        c = c.child(Text::new(sub).max_lines(1).class("muted small"));
    }
    boxed(DecoratedBox::new().child(c).class("tile"))
}

fn info(vm: &Vm) -> W {
    let mut rows: Vec<W> = Vec::new();
    rows.push(parts::kv(t!("Система"), syngui::i18n::t(vm.os.title())));
    let mut fw = syngui::i18n::t(vm.firmware.title());
    if vm.tpm {
        fw.push_str(" · TPM 2.0");
    }
    rows.push(parts::kv(t!("Прошивка"), fw));
    rows.push(parts::kv(t!("Процессор"), parts::cores(vm.cpus)));
    rows.push(parts::kv(t!("Память"), parts::mem_mb(vm.memory_mb)));
    for (i, d) in vm.disks.iter().enumerate() {
        let path = vm.disk_path(d);
        let size = model::disk_virtual_size(&path).map(parts::bytes).unwrap_or_else(|| "—".into());
        rows.push(parts::kv(if i == 0 { t!("Диски") } else { String::new() }, format!("{} · {} · {}", parts::file_name(&d.file), size, syngui::i18n::t(vm.disk_bus.title()))));
    }
    rows.push(parts::kv(t!("Привод"), if vm.cdrom.is_empty() { t!("пусто") } else { parts::file_name(&vm.cdrom) }));
    let net = match vm.net {
        NetMode::Nat => {
            let mut s = format!("NAT · {}", syngui::i18n::t(vm.nic.title()));
            for f in &vm.forwards {
                s.push_str(&format!(" · {}:{}→{}", if f.udp { "udp" } else { "tcp" }, f.host, f.guest));
            }
            s
        }
        NetMode::None => t!("отключена"),
    };
    rows.push(parts::kv(t!("Сеть"), net));
    rows.push(parts::kv(t!("Экран"), syngui::i18n::t(vm.display.title())));
    if !vm.shared_dir.is_empty() {
        rows.push(parts::kv(t!("Общая папка"), vm.shared_dir.clone()));
    }
    rows.push(parts::kv(t!("Создана"), state::chrono_like(vm.created).replace('.', ":")));
    rows.push(parts::kv(t!("Папка"), vm.dir().to_string_lossy().into_owned()));
    parts::card(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(icons::INFO, None, 30.0)).child(Text::new(t!("Сведения")).class("card-title grow")))
            .children(rows),
    )
}

fn actions(st: St, vm: &Vm, l: &Live) -> W {
    let (a, b, c) = (vm.id.clone(), vm.id.clone(), vm.id.clone());
    let dir = vm.dir();
    let on = l.power.on();
    let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
    row = row.child(btn(icons::COPY, t!("Копия"), "soft", move || {
        if st.power(&a).on() {
            state::toast_err(st, t!("Копию можно сделать только с выключенной машины"));
            return;
        }
        let Some(vm) = st.vm(&a) else { return };
        let all = st.vms.get_untracked();
        state::background(st, &a, t!("Копирование…"), move || {
            let c = model::clone(&vm, &all)?;
            let name = c.name.clone();
            run_on_main_thread(move || state::reload(st));
            Ok(Some(t!("Готова копия «{n}»", n = name)))
        });
    }));
    row = row.child(btn(icons::FOLDER_OPEN, t!("Папка машины"), "soft", move || {
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }));
    let sc = st.show_cmd;
    row = row.child(btn(icons::TERMINAL, t!("Командная строка"), "soft", move || sc.set(!sc.get_untracked())));
    if !on {
        row = row.child(btn(icons::DELETE, t!("Удалить"), "danger-soft", move || {
            st.confirm.set(Some(Confirm::Delete(b.clone())));
            super::sync_grab(st);
        }));
    }
    let mut col = Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(parts::badge(icons::TUNE, None, 30.0)).child(Text::new(t!("Действия")).class("card-title grow")))
        .child(row);
    let vm2 = vm.clone();
    col = col.child(Reactive::new(move || -> Vec<W> {
        if !sc.get() {
            return vec![];
        }
        let cmd = qemu::command_line(&vm2);
        let cmd2 = cmd.clone();
        vec![boxed(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(DecoratedBox::new().child(Text::new(cmd).selectable(true).class("mono")).class("code"))
                .child(Row::new().child(DecoratedBox::new().class("grow")).child(btn(icons::COPY, t!("Скопировать"), "ghost", move || {
                    syngui::clipboard::copy(&cmd2);
                    state::toast(st, t!("Командная строка скопирована"));
                }))),
        )]
    }));
    let _ = c;
    parts::card(col)
}
