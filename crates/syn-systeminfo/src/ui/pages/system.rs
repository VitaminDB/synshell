//! Память, сеть, накопители, датчики, питание.

use syngui::core::Color;
use syngui::prelude::*;

use super::{header, hero, Cx};
use crate::sampler::Snap;
use crate::state::{icons, Page};
use crate::ui::parts::{self, boxed, Line, W};

// ─── Память ─────────────────────────────────────────────────────────────────

pub fn memory(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Memory.color();
    let m = &s.mem;
    let mut out = vec![header(cx, Page::Memory, Some(t!("Всего {t}", t = parts::kb(m.total))), true)];
    let swap_c = Color::from_hex("#c792ea");
    let mut lines = vec![Line::new(s.hist.mem.tail(cx.span), color)];
    if m.swap_total > 0 {
        lines.push(Line::new(s.hist.swap.tail(cx.span), swap_c).no_fill());
    }
    // Состав: приложения, общая, ядро, кэш, свободно.
    let t = m.total.max(1) as f32;
    let apps = m.used().saturating_sub(m.kernel() + m.shmem);
    let parts_v: Vec<(String, u64, Color)> = vec![
        (t!("Приложения"), apps, color),
        (t!("Общая (shmem)"), m.shmem, Color::from_hex("#ffd54f")),
        (t!("Ядро"), m.kernel(), Color::from_hex("#ef5350")),
        (t!("Кэш файлов"), m.cache(), Color::from_hex("#4f9dff")),
        (t!("Свободно"), m.free, Color::from_hex("#00000000")),
    ];
    let mut legend: Vec<W> = vec![boxed(parts::stacked(parts_v.iter().map(|(_, v, c)| (*v as f32 / t, *c)).collect(), 14.0))];
    for (name, v, c) in &parts_v {
        let c = if c.a == 0.0 { Color::from_hex("#ffffff30") } else { *c };
        legend.push(parts::legend(c, name.clone(), parts::kb(*v)));
    }
    if m.swap_total > 0 {
        legend.push(parts::legend(swap_c, t!("Подкачка (линия)"), format!("{} / {}", parts::kb(m.swap_used()), parts::kb(m.swap_total))));
    }
    out.push(hero(
        cx,
        color,
        parts::pct(s.hist.mem.last()),
        t!("занято {u} из {t}", u = parts::kb(m.used()), t = parts::kb(m.total)),
        lines,
        100.0,
        legend,
    ));
    let mut tiles = vec![
        parts::tile(t!("Занято"), parts::kb(m.used()), Some(parts::pct(s.hist.mem.last()))),
        parts::tile(t!("Доступно"), parts::kb(m.available), Some(t!("с кэшем"))),
        parts::tile(t!("Кэш"), parts::kb(m.cache()), Some(t!("освобождается"))),
    ];
    if m.swap_total > 0 {
        tiles.push(parts::tile(t!("Подкачка"), parts::kb(m.swap_used()), Some(t!("из {t}", t = parts::kb(m.swap_total)))));
    }
    tiles.push(parts::tile(t!("Ждёт записи"), parts::kb(m.dirty + m.writeback), None));
    if let Some(p) = s.pressure[1] {
        tiles.push(parts::tile(t!("Нехватка"), parts::pct(p), Some(t!("PSI за 10 с"))));
    }
    if m.cma_total > 0 {
        tiles.push(parts::tile(t!("CMA"), parts::kb(m.cma_total - m.cma_free.min(m.cma_total)), Some(t!("из {t}", t = parts::kb(m.cma_total)))));
    }
    out.push(parts::tiles(tiles, cx.w));
    if !s.swaps.is_empty() {
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::STORAGE, t!("Подкачка"), None, swap_c));
        for (dev, kind, size, used) in &s.swaps {
            let kind = if dev.contains("zram") { t!("zram, сжатие в памяти") } else if kind == "file" { t!("файл") } else { t!("раздел") };
            col = col.child(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Row::new().gap(10.0).child(Text::new(dev.clone()).max_lines(1).class("grow")).child(Text::new(format!("{} / {}", parts::kb(*used), parts::kb(*size))).class("num")))
                    .child(Text::new(kind).class("muted small"))
                    .child(parts::meter(*used as f32 / (*size).max(1) as f32 * 100.0, swap_c, 6.0)),
            );
        }
        out.push(boxed(parts::card(col)));
    }
    // Больше всего памяти.
    let mut top: Vec<&synsystem::procs::Process> = s.procs.iter().filter(|p| p.rss_kb > 0).collect();
    top.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb));
    let max = top.first().map(|p| p.rss_kb).unwrap_or(1).max(1);
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::LIST, t!("Больше всего памяти"), Some(t!("RSS")), color));
    for p in top.into_iter().take(10) {
        col = col.child(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(10.0).child(Text::new(p.name.clone()).max_lines(1).class("grow")).child(Text::new(parts::kb(p.rss_kb)).class("num")))
                .child(parts::meter(p.rss_kb as f32 / max as f32 * 100.0, color, 6.0)),
        );
    }
    out.push(boxed(parts::card(col)));
    out
}

// ─── Сеть ───────────────────────────────────────────────────────────────────

const RX: &str = "#22c3e6";
const TX: &str = "#ffca28";

fn iface_icon(kind: &str) -> &'static str {
    match kind {
        "wifi" => icons::WIFI,
        "mobile" => icons::CELL,
        "usb" => icons::USB,
        "vpn" => icons::VPN,
        "ethernet" => icons::ETHERNET,
        _ => icons::HUB,
    }
}

fn iface_kind(kind: &str) -> String {
    match kind {
        "wifi" => t!("Wi-Fi"),
        "mobile" => t!("Мобильная сеть"),
        "usb" => t!("USB"),
        "vpn" => t!("VPN"),
        "ethernet" => t!("Ethernet"),
        "bridge" => t!("Мост"),
        _ => t!("Виртуальный"),
    }
}

pub fn network(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Network.color();
    let (rx_c, tx_c) = (Color::from_hex(RX), Color::from_hex(TX));
    let mut out = vec![header(cx, Page::Network, None, true)];
    let rx = s.hist.rx.tail(cx.span);
    let tx = s.hist.tx.tail(cx.span);
    let max = parts::nice_max(rx.iter().chain(tx.iter()).copied().fold(0.0, f32::max), 64.0 * 1024.0);
    let (total_rx, total_tx) = s.net.iter().fold((0u64, 0u64), |a, i| (a.0 + i.info.rx_bytes, a.1 + i.info.tx_bytes));
    out.push(hero(
        cx,
        color,
        format!("↓ {}", parts::rate(s.rx())),
        format!("↑ {} · {}", parts::rate(s.tx()), t!("шкала {m}", m = parts::rate(max as f64))),
        vec![Line::new(rx, rx_c), Line::new(tx, tx_c)],
        max,
        vec![parts::legend(rx_c, t!("Принято с загрузки"), parts::bytes(total_rx)), parts::legend(tx_c, t!("Передано с загрузки"), parts::bytes(total_tx))],
    ));
    let mut up: Vec<&crate::sampler::Iface> = s.net.iter().filter(|i| i.info.up && i.info.kind != "virtual").collect();
    up.sort_by(|a, b| (b.info.rx_bytes + b.info.tx_bytes).cmp(&(a.info.rx_bytes + a.info.tx_bytes)));
    if up.is_empty() {
        out.push(parts::empty(t!("Нет подключений.")));
    }
    for i in up {
        let mut rows = vec![(t!("Скорость"), format!("↓ {}  ↑ {}", parts::rate(i.rx_bps), parts::rate(i.tx_bps))), (t!("Всего"), format!("↓ {}  ↑ {}", parts::bytes(i.info.rx_bytes), parts::bytes(i.info.tx_bytes)))];
        if !i.addrs.is_empty() {
            rows.push((t!("Адреса"), i.addrs.join("\n")));
        }
        if let Some(m) = &i.info.mac {
            rows.push((t!("MAC"), m.clone()));
        }
        if let Some(sp) = i.info.speed_mbps {
            rows.push((t!("Линия"), t!("{n} Мбит/с", n = sp)));
        }
        out.push(boxed(parts::card(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(parts::card_head(iface_icon(&i.info.kind), i.info.name.clone(), Some(iface_kind(&i.info.kind)), color))
                .child(parts::kv_list(rows)),
        )));
    }
    let down: Vec<String> = s.net.iter().filter(|i| !i.info.up && i.info.kind != "virtual").map(|i| i.info.name.clone()).collect();
    if !down.is_empty() {
        out.push(parts::empty(t!("Отключены: {l}", l = down.join(", "))));
    }
    out
}

// ─── Накопители ─────────────────────────────────────────────────────────────

pub fn disk(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Disk.color();
    let (r_c, w_c) = (Color::from_hex("#a3d65c"), Color::from_hex("#ff8a65"));
    let mut out = vec![header(cx, Page::Disk, None, true)];
    let r = s.hist.disk_read.tail(cx.span);
    let w = s.hist.disk_write.tail(cx.span);
    let max = parts::nice_max(r.iter().chain(w.iter()).copied().fold(0.0, f32::max), 1024.0 * 1024.0);
    out.push(hero(
        cx,
        color,
        parts::rate(s.disk_read() + s.disk_write()),
        format!("{} · {}", t!("чтение {r}", r = parts::rate(s.disk_read())), t!("запись {w}", w = parts::rate(s.disk_write()))),
        vec![Line::new(r, r_c), Line::new(w, w_c)],
        max,
        vec![parts::legend(r_c, t!("Чтение"), parts::rate(s.disk_read())), parts::legend(w_c, t!("Запись"), parts::rate(s.disk_write()))],
    ));
    for d in &s.disks {
        out.push(boxed(parts::card(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(parts::card_head(icons::STORAGE, d.model.clone().unwrap_or_else(|| d.name.clone()), Some(format!("{} · {}", d.name, parts::bytes(d.size))), color))
                .child(parts::kv_list(vec![
                    (t!("Чтение"), parts::rate(d.read_bps)),
                    (t!("Запись"), parts::rate(d.write_bps)),
                    (t!("Занят"), parts::pct(d.busy_percent)),
                    (t!("Тип"), if d.rotational { t!("жёсткий диск") } else { t!("флеш-память") }),
                ]))
                .child(parts::meter(d.busy_percent, color, 6.0)),
        )));
    }
    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::STORAGE, t!("Разделы"), None, color));
    for m in &s.mounts {
        let p = m.used as f32 / m.total.max(1) as f32 * 100.0;
        let bar = if p >= 90.0 { Color::from_hex("#ef5350") } else if p >= 75.0 { Color::from_hex("#ff9f43") } else { color };
        col = col.child(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(10.0)
                        .child(Text::new(m.path.clone()).max_lines(1).class("grow"))
                        .child(Text::new(t!("{u} из {t}", u = parts::bytes(m.used), t = parts::bytes(m.total))).class("num")),
                )
                .child(parts::meter(p, bar, 8.0))
                .child(Text::new(format!("{} · {}{} · {}", m.device, m.fstype, if m.read_only { t!(", только чтение") } else { String::new() }, t!("свободно {f}", f = parts::bytes(m.total - m.used)))).max_lines(1).class("muted small")),
        );
    }
    if s.mounts.is_empty() {
        col = col.child(Text::new(t!("Нет смонтированных разделов.")).class("muted"));
    }
    out.push(boxed(parts::card(col)));
    out
}

// ─── Датчики ────────────────────────────────────────────────────────────────

/// Зоны одного узла (`cpu-1-2-usr`, `cpu-0-0-usr` → `cpu`) — одной группой.
fn group_name(name: &str) -> String {
    let base = name.strip_prefix("hwmon:").unwrap_or(name);
    let cut = base.char_indices().find(|(i, c)| c.is_ascii_digit() && *i > 0 && base[..*i].ends_with(['-', '_', ' '])).map(|(i, _)| i).unwrap_or(base.len());
    let n = base[..cut].trim_end_matches(['-', '_', ' ']);
    if n.is_empty() { base.to_string() } else { n.to_string() }
}

pub fn sensors(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Sensors.color();
    let mut out = vec![header(cx, Page::Sensors, Some(t!("Датчиков: {n}", n = s.temps.len())), true)];
    let gpu_c = Page::Gpu.color();
    let mut lines = vec![Line::new(s.hist.cpu_temp.tail(cx.span), color)];
    let mut legend = Vec::new();
    if let Some(t) = s.cpu_temp {
        legend.push(parts::legend(color, t!("Процессор"), parts::celsius(t)));
    }
    if let Some(t) = s.gpu_temp {
        lines.push(Line::new(s.hist.gpu_temp.tail(cx.span), gpu_c).no_fill());
        legend.push(parts::legend(gpu_c, t!("Графика"), parts::celsius(t)));
    }
    let hottest = s.temps.first();
    if let Some(h) = hottest {
        out.push(hero(cx, parts::temp_color(h.celsius), parts::celsius(h.celsius), t!("горячее всего: {n}", n = group_name(&h.name)), lines, 100.0, legend));
    } else {
        out.push(parts::empty(t!("Датчиков температуры нет.")));
        return out;
    }
    // Группы: максимум, число зон.
    let mut groups: Vec<(String, f32, usize, f32)> = Vec::new();
    for t in &s.temps {
        let g = group_name(&t.name);
        match groups.iter_mut().find(|x| x.0 == g) {
            Some(x) => {
                x.1 = x.1.max(t.celsius);
                x.2 += 1;
                x.3 = x.3.min(t.celsius);
            }
            None => groups.push((g, t.celsius, 1, t.celsius)),
        }
    }
    groups.sort_by(|a, b| b.1.total_cmp(&a.1));
    let cols = if cx.w >= 640.0 { 2 } else { 1 };
    let mut items: Vec<W> = Vec::new();
    for (name, max, n, min) in groups {
        let sub = if n > 1 { t!("зон {n} · от {min}", n = n, min = parts::celsius(min)) } else { String::new() };
        items.push(boxed(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Column::new().gap(0.0).child(Text::new(name).max_lines(1)).child(Text::new(sub).max_lines(1).class("muted small")).class("grow"))
                                .child(Text::new(parts::celsius(max)).class("temp-value").style("color", parts::temp_color(max))),
                        )
                        .child(parts::meter(max.clamp(0.0, 110.0) / 1.1, parts::temp_color(max), 6.0)),
                )
                .class("sensor"),
        ));
    }
    out.push(boxed(Grid::new(cols).gap(8.0).children(items)));
    out
}

// ─── Питание ────────────────────────────────────────────────────────────────

fn status_text(s: &str) -> String {
    match s {
        "Charging" => t!("заряжается"),
        "Discharging" => t!("разряжается"),
        "Full" => t!("заряжена"),
        "Not charging" => t!("не заряжается"),
        "" => "—".into(),
        other => other.to_string(),
    }
}

pub fn battery(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Battery.color();
    let mut out = vec![header(cx, Page::Battery, s.batteries.first().and_then(|b| b.model.clone()), true)];
    let Some(b) = &s.battery else {
        out.push(parts::empty(match s.adc_volt {
            Some(v) => t!("Драйвер батареи не поднят; напряжение по АЦП — {v} В.", v = format!("{v:.2}")),
            None => t!("Батареи нет — питание от сети."),
        }));
        return out;
    };
    let pw_c = Color::from_hex("#ffd54f");
    let state = if b.charging { t!("заряжается") } else if b.full { t!("заряжена") } else { t!("от батареи") };
    let left = b.minutes.filter(|m| *m > 0 && !b.full).map(|m| if b.charging { t!("до полной {d}", d = parts::duration(m as u64 * 60)) } else { t!("осталось {d}", d = parts::duration(m as u64 * 60)) });
    let ring_size = if cx.narrow { 120.0 } else { 150.0 };
    let mut info = Column::new()
        .gap(4.0)
        .child(Text::new(state).class("hero-value").style("color", color))
        .child(Text::new(left.unwrap_or_default()).class("muted"));
    if let Some(p) = b.power_w {
        info = info.child(Text::new(t!("{p} Вт", p = format!("{p:.1}"))).class("muted"));
    }
    out.push(boxed(parts::card(
        Row::new()
            .gap(20.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(parts::ring(b.percent as f32, ring_size, color, format!("{}%", b.percent), if b.charging { "⚡".into() } else { String::new() }))
            .child(info.class("grow")),
    )));
    let pmax = parts::nice_max(s.hist.power.max(cx.span), 1.0);
    out.push(hero(
        cx,
        color,
        format!("{}%", b.percent),
        t!("заряд и мощность (шкала {m} Вт)", m = format!("{pmax:.0}")),
        vec![Line::new(s.hist.battery.tail(cx.span), color), Line::new(s.hist.power.tail(cx.span).into_iter().map(|p| p / pmax * 100.0).collect(), pw_c).no_fill()],
        100.0,
        vec![parts::legend(color, t!("Заряд"), format!("{}%", b.percent)), parts::legend(pw_c, t!("Мощность"), b.power_w.map(|p| format!("{p:.1} Вт")).unwrap_or_else(|| "—".into()))],
    ));
    for bi in &s.batteries {
        let mut tiles = vec![parts::tile(t!("Состояние"), status_text(&bi.status), bi.health.clone())];
        if let Some(v) = bi.voltage_v {
            tiles.push(parts::tile(t!("Напряжение"), format!("{v:.2} В"), None));
        }
        if let Some(a) = bi.current_a {
            tiles.push(parts::tile(t!("Ток"), format!("{:.2} А", a.abs()), None));
        }
        if let Some(t) = bi.temp_c {
            tiles.push(parts::tile(t!("Температура"), parts::celsius(t), None));
        }
        if let Some(w) = bi.wear_percent() {
            tiles.push(parts::tile(t!("Ёмкость"), format!("{w}%"), Some(t!("от паспортной"))));
        }
        if let Some(c) = bi.cycles.filter(|c| *c > 0) {
            tiles.push(parts::tile(t!("Циклы"), c.to_string(), None));
        }
        if let Some(t) = &bi.technology {
            tiles.push(parts::tile(t!("Технология"), t.clone(), bi.manufacturer.clone()));
        }
        out.push(parts::tiles(tiles, cx.w));
    }
    out
}
