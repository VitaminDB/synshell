//! Процессор, графика, видеопамять, нейропроцессор.

use syngui::core::Color;
use syngui::prelude::*;

use super::{header, hero, Cx};
use crate::sampler::{Model, Snap};
use crate::state::{icons, Page};
use crate::ui::parts::{self, boxed, Line, W};

/// Цвета классов ядер: медленные, средние, быстрые, самые быстрые.
const TIERS: [&str; 4] = ["#4fc3f7", "#4f9dff", "#ffb74d", "#ef5350"];

fn tier_color(t: usize) -> Color {
    Color::from_hex(TIERS[t.min(3)])
}

// ─── Процессор ──────────────────────────────────────────────────────────────

pub fn cpu(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Cpu.color();
    let mut out = vec![header(cx, Page::Cpu, s.cpu_model.clone(), true)];
    let online: Vec<&synsystem::cpu::Core> = s.cpu.cores.iter().filter(|c| c.online).collect();
    let freq = online.iter().filter_map(|c| c.cur_mhz).max();
    let mut sub = Vec::new();
    if let Some(f) = freq {
        sub.push(t!("до {f}", f = parts::mhz(f)));
    }
    if let Some(t) = s.cpu_temp {
        sub.push(parts::celsius(t));
    }
    out.push(hero(cx, color, parts::pct(s.cpu.usage), sub.join(" · "), vec![Line::new(s.hist.cpu.tail(cx.span), color)], 100.0, vec![]));

    let mut tiles = vec![
        parts::tile(t!("Загрузка"), parts::pct(s.cpu.usage), s.pressure[0].map(|p| t!("ожидание {p}", p = parts::pct(p)))),
        parts::tile(t!("Ядра"), format!("{}/{}", online.len(), s.cpu.cores.len()), Some(t!("в работе"))),
    ];
    if let Some(f) = freq {
        let max = s.cpu.cores.iter().filter_map(|c| c.max_mhz).max();
        tiles.push(parts::tile(t!("Частота"), parts::mhz(f), max.map(|m| t!("макс. {m}", m = parts::mhz(m)))));
    }
    if let Some(t) = s.cpu_temp {
        tiles.push(parts::tile(t!("Температура"), parts::celsius(t), Some(t!("макс. за 5 мин {t}", t = parts::celsius(s.hist.cpu_temp.max(300))))));
    }
    if let Some((l, tasks)) = s.load {
        tiles.push(parts::tile(t!("Средняя нагрузка"), format!("{:.2}", l[0]), Some(format!("{:.2} · {:.2}", l[1], l[2]))));
        let threads: u32 = s.procs.iter().map(|p| p.threads).sum();
        tiles.push(parts::tile(t!("Процессы"), s.procs.len().to_string(), Some(t!("потоков {n}", n = threads.max(tasks)))));
    }
    tiles.push(parts::tile(t!("Работает"), parts::duration(s.uptime), None));
    out.push(parts::tiles(tiles, cx.w));

    // Ядра: загрузка, частота, мини-график; цвет — класс ядра.
    let cols = ((cx.w / 132.0) as usize).clamp(2, 8);
    let mut cores: Vec<W> = Vec::new();
    for (i, c) in s.cpu.cores.iter().enumerate() {
        let col = tier_color(c.tier);
        let hist = s.hist.cores.get(i).map(|h| h.tail(60)).unwrap_or_default();
        let body = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("CPU{}", c.index)).class("core-name grow"))
                    .child(Text::new(if c.online { parts::pct(c.usage) } else { t!("выкл") }).class("core-value").style("color", col)),
            )
            .child(parts::area_chart(vec![Line::new(hist, col)], 60, 100.0, 34.0))
            .child(Text::new(c.cur_mhz.filter(|_| c.online).map(parts::mhz).unwrap_or_else(|| "—".into())).class("core-mhz"));
        cores.push(boxed(DecoratedBox::new().child(body).class(if c.online { "core" } else { "core off" })));
    }
    if !cores.is_empty() {
        let mut legend = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let tiers = s.cpu.cores.iter().map(|c| c.tier).max().unwrap_or(0);
        if tiers > 0 {
            let names = [t!("энергоэффективные"), t!("производительные"), t!("быстрые"), t!("самые быстрые")];
            for t in 0..=tiers.min(3) {
                legend = legend.child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("dot").style("background-color", tier_color(t)))
                        .child(Text::new(names[t].clone()).class("muted small")),
                );
            }
        }
        out.push(boxed(parts::card(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(parts::card_head(icons::CPU, t!("Ядра"), Some(t!("за минуту")), color))
                .child(Grid::new(cols).gap(8.0).children(cores))
                .child(legend),
        )));
    }

    // Кластеры cpufreq.
    if s.clusters.len() > 1 || s.clusters.iter().any(|c| c.governor.is_some()) {
        let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::HUB, t!("Кластеры"), None, color));
        for c in &s.clusters {
            let range = match (c.min_mhz, c.max_mhz) {
                (Some(a), Some(b)) => format!("{} – {}", parts::mhz(a), parts::mhz(b)),
                _ => "—".into(),
            };
            let cpus = c.cpus.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
            let tier = c.cpus.first().and_then(|i| s.cpu.cores.get(*i)).map(|k| k.tier).unwrap_or(0);
            col = col.child(parts::legend(tier_color(tier), t!("ядра {c}", c = cpus), format!("{range} · {}", c.governor.clone().unwrap_or_default())));
        }
        out.push(boxed(parts::card(col)));
    }
    out
}

// ─── Графика ────────────────────────────────────────────────────────────────

pub fn gpu(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Gpu.color();
    let names = s.gpus.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(" · ");
    let mut out = vec![header(cx, Page::Gpu, Some(names), true)];
    if s.gpus.is_empty() {
        out.push(parts::empty(t!("Графический процессор не найден: драйвер не даёт сведений в sysfs.")));
        return out;
    }
    for (i, g) in s.gpus.iter().enumerate() {
        let col = if i == 0 { color } else { Color::from_hex("#7bd88f") };
        if g.asleep {
            out.push(boxed(parts::card(
                Column::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(parts::card_head(icons::SLEEP, g.name.clone(), Some(g.driver.clone()), col))
                    .child(Text::new(t!("Видеокарта спит (энергосбережение). Сведения появятся, когда её займёт программа — опрос её бы разбудил.")).max_lines(3).class("muted")),
            )));
            continue;
        }
        let hist = s.hist.gpu.get(i).map(|h| h.tail(cx.span)).unwrap_or_default();
        let mut sub = vec![g.name.clone()];
        if let Some(m) = g.cur_mhz {
            sub.push(parts::mhz(m));
        }
        let big = g.busy_percent.map(parts::pct).unwrap_or_else(|| g.cur_mhz.map(parts::mhz).unwrap_or_else(|| "—".into()));
        if g.busy_percent.is_some() {
            out.push(hero(cx, col, big, sub.join(" · "), vec![Line::new(hist, col)], 100.0, vec![]));
        } else {
            out.push(boxed(parts::card(
                Column::new()
                    .gap(6.0)
                    .child(parts::card_head(icons::GPU, g.name.clone(), Some(g.driver.clone()), col))
                    .child(Text::new(big).class("hero-value").style("color", col))
                    .child(Text::new(t!("Драйвер не сообщает загрузку — только частоту.")).class("muted small")),
            )));
        }
        let mut tiles = Vec::new();
        if let Some(b) = g.busy_percent {
            tiles.push(parts::tile(t!("Загрузка"), parts::pct(b), Some(t!("пик за минуту {p}", p = parts::pct(s.hist.gpu.get(i).map(|h| h.max(60)).unwrap_or(0.0))))));
        }
        if let Some(m) = g.cur_mhz {
            tiles.push(parts::tile(t!("Частота"), parts::mhz(m), g.max_mhz.map(|x| t!("макс. {m}", m = parts::mhz(x)))));
        }
        if let Some(t) = g.temp_c.or(if i == 0 { s.gpu_temp } else { None }) {
            tiles.push(parts::tile(t!("Температура"), parts::celsius(t), None));
        }
        if let Some(p) = g.power_w {
            tiles.push(parts::tile(t!("Мощность"), format!("{p:.1} Вт"), None));
        }
        if let Some(u) = g.mem_used {
            let sub = match g.mem_total {
                Some(t) => t!("из {t}", t = parts::bytes(t)),
                None => t!("из общей памяти"),
            };
            tiles.push(parts::tile(t!("Видеопамять"), parts::bytes(u), Some(sub)));
        }
        tiles.push(parts::tile(t!("Драйвер"), g.driver.clone(), None));
        out.push(parts::tiles(tiles, cx.w));
    }
    if let Some(models) = &s.models {
        let on: Vec<&Model> = models.iter().filter(|m| matches!(m.device.as_str(), "gpu" | "cuda") && m.state != synaptixd::LoadState::Unloaded).collect();
        if !on.is_empty() {
            out.push(models_card(&t!("Нейросети на GPU"), &on, color));
        }
    }
    out
}

// ─── Видеопамять ────────────────────────────────────────────────────────────

pub fn vram(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Vram.color();
    let mut out = vec![header(cx, Page::Vram, None, true)];
    let mut any = false;
    for (i, g) in s.gpus.iter().enumerate() {
        let Some(used) = g.mem_used else {
            if g.asleep {
                out.push(boxed(parts::card(
                    Column::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(parts::card_head(icons::SLEEP, g.name.clone(), None, color))
                        .child(Text::new(t!("Видеокарта спит — её память свободна.")).class("muted")),
                )));
                any = true;
            }
            continue;
        };
        any = true;
        let total = g.mem_total;
        let hist = s.hist.vram.get(i).map(|h| h.tail(cx.span)).unwrap_or_default();
        // Общая память: шкала по пику истории (вся ОЗУ сделала бы график плоским).
        let max = match total {
            Some(t) => t as f32,
            None => parts::nice_max(hist.iter().copied().fold(used as f32, f32::max) * 1.2, 64.0 * 1048576.0),
        };
        let sub = match total {
            Some(t) => t!("{n} · из {t} ({p})", n = g.name.clone(), t = parts::bytes(t), p = parts::pct(used as f32 / t.max(1) as f32 * 100.0)),
            None => t!("{n} · память общая с ОЗУ", n = g.name.clone()),
        };
        let mut legend = Vec::new();
        if let Some(t) = total {
            legend.push(boxed(parts::meter(used as f32 / t.max(1) as f32 * 100.0, color, 10.0)));
            legend.push(parts::legend(color, t!("Занято"), parts::bytes(used)));
            legend.push(parts::legend(Color::from_hex("#ffffff30"), t!("Свободно"), parts::bytes(t.saturating_sub(used))));
        } else {
            legend.push(parts::legend(color, t!("Выделено драйвером"), parts::bytes(used)));
            legend.push(parts::legend(Page::Memory.color(), t!("Доля ОЗУ"), parts::pct(used as f32 / (s.mem.total.max(1) * 1024) as f32 * 100.0)));
        }
        out.push(hero(cx, color, parts::bytes(used), sub, vec![Line::new(hist, color)], max, legend));
    }
    if !any {
        out.push(parts::empty(t!("Драйвер графики не сообщает занятость видеопамяти.")));
    }
    if s.gpus.iter().any(|g| g.shared_memory && g.mem_used.is_some()) {
        out.push(parts::empty(t!("У встроенной графики своей памяти нет: GPU берёт буферы из системной памяти по мере надобности, поэтому они входят и в «Память».")));
    }

    // По процессам.
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::LIST, t!("По процессам"), None, color));
    if s.gpu_procs.is_empty() {
        col = col.child(Text::new(t!("Собираю сведения…")).class("muted"));
    }
    let top = s.gpu_procs.first().map(|p| p.2).unwrap_or(1).max(1);
    for (pid, name, b) in s.gpu_procs.iter().take(16) {
        col = col.child(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(10.0)
                        .child(Text::new(name.clone()).max_lines(1).class("grow"))
                        .child(Text::new(pid.to_string()).class("muted small"))
                        .child(Text::new(parts::bytes(*b)).class("num")),
                )
                .child(parts::meter(*b as f32 / top as f32 * 100.0, color, 6.0)),
        );
    }
    out.push(boxed(parts::card(col)));
    if let Some(models) = &s.models {
        let on: Vec<&Model> = models.iter().filter(|m| m.vram_bytes > 0).collect();
        if !on.is_empty() {
            out.push(models_card(&t!("Нейросети в видеопамяти"), &on, color));
        }
    }
    out
}

// ─── Нейропроцессор ─────────────────────────────────────────────────────────

pub fn npu(cx: &Cx, s: &Snap) -> Vec<W> {
    let color = Page::Npu.color();
    let Some(n) = &s.npu else {
        return vec![
            header(cx, Page::Npu, None, false),
            parts::empty(t!("Нейропроцессор не найден. Поддерживаются Qualcomm Hexagon (CDSP) и NPU на драйверах accel (Intel и др.).")),
        ];
    };
    let mut out = vec![header(cx, Page::Npu, Some(n.name.clone()), n.busy_percent.is_some())];
    let status = if n.online { t!("включён") } else { t!("выключен") };
    let state = n.state.clone().unwrap_or_default();
    if let Some(b) = n.busy_percent {
        let sub = match n.cur_mhz {
            Some(m) if m > 0 => format!("{status} · {}", parts::mhz(m)),
            _ => status.clone(),
        };
        out.push(hero(cx, color, parts::pct(b), sub, vec![Line::new(s.hist.npu.tail(cx.span), color)], 100.0, vec![]));
    } else {
        let busy_models = s.models.as_ref().map(|m| m.iter().filter(|m| m.device == "npu" && m.busy).count()).unwrap_or(0);
        let active = n.daemon.as_ref().map(|d| d.workers.len()).unwrap_or(0);
        let (big, line) = if busy_models > 0 {
            (t!("считает"), t!("Сейчас работают модели: {n}", n = busy_models))
        } else if active > 0 {
            (t!("готов"), t!("Открыто сеансов: {n} — графы загружены, ждут данных", n = active))
        } else if n.online {
            (t!("простаивает"), t!("Сопроцессор включён, программ нет"))
        } else {
            (t!("выключен"), t!("Сопроцессор не запущен"))
        };
        out.push(boxed(parts::card(
            Row::new()
                .gap(16.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(parts::badge_icon(icons::NPU, color, 64.0))
                .child(
                    Column::new()
                        .gap(4.0)
                        .child(Text::new(big).class("hero-value").style("color", color))
                        .child(Text::new(line).max_lines(2).class("muted"))
                        .child(Text::new(t!("Загрузку HTP ядро не сообщает — видно состояние, сеансы и модели.")).max_lines(2).class("muted small"))
                        .class("grow"),
                ),
        )));
    }
    let mut tiles = vec![parts::tile(t!("Состояние"), status, (!state.is_empty()).then_some(state))];
    if let Some(m) = n.cur_mhz {
        tiles.push(parts::tile(t!("Частота"), if m > 0 { parts::mhz(m) } else { t!("простой") }, n.max_mhz.map(|x| t!("макс. {m}", m = parts::mhz(x)))));
    }
    if let Some(b) = n.mem_bytes {
        tiles.push(parts::tile(t!("Память"), parts::bytes(b), Some(if n.vendor == "qualcomm" { t!("процессы демона") } else { t!("задачи NPU") })));
    }
    if let Some(t) = n.temp_c {
        tiles.push(parts::tile(t!("Температура"), parts::celsius(t), Some(t!("блок NSP"))));
    }
    if let Some(d) = &n.daemon {
        tiles.push(parts::tile(t!("Сеансы"), d.workers.len().to_string(), Some(t!("по PD на CDSP"))));
    }
    if let Some(c) = s.npu_cache {
        tiles.push(parts::tile(t!("Кэш графов"), parts::bytes(c), Some("/var/cache/synnpu".into())));
    }
    out.push(parts::tiles(tiles, cx.w));
    if let Some(d) = &n.daemon {
        out.push(boxed(parts::card(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(parts::card_head(icons::HUB, t!("Служба synnpu-qnn"), Some(t!("pid {p}", p = d.pid)), color))
                .child(parts::kv_list(vec![
                    (t!("Рабочих процессов"), d.workers.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ")),
                    (t!("Память"), parts::kb(d.rss_kb)),
                ])),
        )));
    }
    match &s.models {
        Some(models) => {
            let on: Vec<&Model> = models.iter().filter(|m| m.device == "npu").collect();
            if on.is_empty() {
                out.push(parts::empty(t!("Моделей synaptixd на NPU сейчас нет.")));
            } else {
                out.push(models_card(&t!("Нейросети на NPU"), &on, color));
            }
        }
        None => out.push(parts::empty(t!("Служба нейросетей synaptixd не отвечает."))),
    }
    out
}

fn models_card(title: &str, models: &[&Model], color: Color) -> W {
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(parts::card_head(icons::NPU, title.to_string(), None, color));
    for m in models {
        let state = match m.state {
            synaptixd::LoadState::Loaded if m.busy => t!("считает"),
            synaptixd::LoadState::Loaded => t!("загружена"),
            synaptixd::LoadState::Loading => t!("загружается"),
            synaptixd::LoadState::Failed => t!("ошибка"),
            synaptixd::LoadState::Unloaded => t!("выгружена"),
        };
        let mem = if m.vram_bytes > 0 { format!("{} + {}", parts::bytes(m.mem_bytes), parts::bytes(m.vram_bytes)) } else { parts::bytes(m.mem_bytes) };
        col = col.child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Column::new().gap(1.0).child(Text::new(m.title.clone()).max_lines(1)).child(Text::new(format!("{state} · {}", t!("клиентов {n}", n = m.clients))).class("muted small")).class("grow"))
                .child(Text::new(mem).class("num")),
        );
    }
    boxed(parts::card(col))
}
