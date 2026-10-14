//! Мастер новой машины: источник (образ, готовый диск, пусто) → система → параметры.
//! Система угадывается по имени образа, железо подбирается под неё.

use std::path::PathBuf;

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, spacer, W};
use crate::model::{self, DiskBus, Firmware, OsKind};
use crate::state::{self, icons, Draft, Source, St};

const STEPS: usize = 3;

pub fn layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let Some(step) = st.wiz.get() else { return vec![] };
        let vp = viewport_size().get();
        let narrow = vp.width < super::NARROW_BELOW;
        let w = if narrow { vp.width - 20.0 } else { 720.0f32.min(vp.width - 40.0) };
        let body = match step {
            0 => source(st),
            1 => system(st, narrow),
            _ => params(st),
        };
        let card = DecoratedBox::new()
            .child(
                Column::new()
                    .gap(18.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(head(st, step))
                    .child(ScrollView::new().vertical().child(body).class("wiz-body"))
                    .child(footer(st, step)),
            )
            .class("dialog wizard")
            .style("width", StyleValue::px(w))
            .style("max-height", StyleValue::px(vp.height - 40.0));
        vec![super::modal(boxed(card), move || super::close_wizard(st))]
    }))
}

fn draft(st: St) -> Draft {
    st.draft.get_untracked().unwrap_or_default()
}

fn edit(st: St, f: impl FnOnce(&mut Draft)) {
    let mut d = draft(st);
    f(&mut d);
    st.draft.set(Some(d));
}

fn head(st: St, step: usize) -> W {
    let titles = [t!("Источник"), t!("Система"), t!("Параметры")];
    let narrow = viewport_size().get_untracked().width < 620.0;
    let mut dots = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (i, title) in titles.iter().enumerate() {
        let cls = if i < step { "step done" } else if i == step { "step active" } else { "step" };
        dots = dots.child(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(6.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().child(if i < step { boxed(Icon::new(icons::CHECK).class("step-check")) } else { boxed(Text::new((i + 1).to_string()).class("step-num")) }).class("step-dot"))
                        .child(Text::new(if narrow && i != step { String::new() } else { title.clone() }).class("step-label")),
                )
                .class(cls),
        );
        if i + 1 < STEPS {
            dots = dots.child(DecoratedBox::new().class(if i < step { "step-line done" } else { "step-line" }));
        }
    }
    boxed(
        Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(parts::badge(icons::ADD, None, 38.0))
                    .child(Text::new(t!("Новая машина")).class("dialog-title grow"))
                    .child(parts::icon_btn(icons::CLOSE, String::new(), move || super::close_wizard(st))),
            )
            .child(dots),
    )
}

fn footer(st: St, step: usize) -> W {
    let mut r = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if step > 0 {
        r = r.child(btn(icons::BACK, t!("Назад"), "ghost", move || st.wiz.set(Some(step - 1))));
    }
    r = r.child(spacer());
    if step + 1 < STEPS {
        r = r.child(btn("", t!("Далее"), "primary", move || {
            let d = draft(st);
            if step == 0 && d.source != Source::Empty && d.path.trim().is_empty() {
                state::toast_err(st, if d.source == Source::Iso { t!("Выберите образ установки") } else { t!("Выберите файл диска") });
                return;
            }
            st.wiz.set(Some(step + 1));
        }));
    } else {
        r = r.child(btn(icons::CHECK, t!("Создать"), "primary", move || create(st)));
    }
    boxed(r)
}

// ─── Шаг 1: источник ────────────────────────────────────────────────────

fn source(st: St) -> W {
    let choice = Reactive::new(move || -> Vec<W> {
        let d = st.draft.get().unwrap_or_default();
        let opt = |src: Source, icon: &'static str, title: String, hint: String| -> W {
            let active = d.source == src;
            boxed(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || {
                        edit(st, |d| {
                            if d.source != src {
                                d.source = src;
                                d.path.clear();
                            }
                        })
                    })
                    .child(
                        DecoratedBox::new()
                            .child(
                                Column::new()
                                    .gap(8.0)
                                    .child(parts::badge(icon, None, 36.0))
                                    .child(Text::new(title).max_lines(1).class("row-title"))
                                    .child(Text::new(hint).max_lines(3).class("row-hint")),
                            )
                            .class(if active { "option active" } else { "option" }),
                    ),
            )
        };
        let narrow = viewport_size().get_untracked().width < 640.0;
        vec![boxed(Grid::new(if narrow { 1 } else { 3 }).gap(10.0).children(vec![
            opt(Source::Iso, icons::DISC, t!("Образ установки"), t!("ISO-файл системы: Ubuntu, Fedora, Windows…")),
            opt(Source::Disk, icons::STORAGE, t!("Готовый диск"), t!("qcow2, raw, img — уже установленная система")),
            opt(Source::Empty, icons::ADD, t!("Пустая машина"), t!("Без образа: сетевая загрузка или вставить позже")),
        ]))]
    });
    let file = Reactive::new(move || -> Vec<W> {
        let d = st.draft.get().unwrap_or_default();
        if d.source == Source::Empty {
            return vec![];
        }
        let iso = d.source == Source::Iso;
        let picked: W = if d.path.is_empty() {
            boxed(Text::new(if iso { t!("Образ не выбран") } else { t!("Диск не выбран") }).class("row-hint"))
        } else {
            let size = std::fs::metadata(&d.path).map(|m| parts::bytes(m.len())).unwrap_or_default();
            let guess = OsKind::guess(&d.path).map(|o| t!("похоже на {o}", o = syngui::i18n::t(o.title()))).unwrap_or_default();
            boxed(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(parts::badge(if iso { icons::DISC } else { icons::STORAGE }, None, 34.0))
                    .child(
                        Column::new()
                            .gap(2.0)
                            .child(Text::new(parts::file_name(&d.path)).max_lines(1).class("row-title"))
                            .child(Text::new([size, guess].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")).max_lines(1).class("row-hint"))
                            .class("grow"),
                    ),
            )
        };
        let pick = btn(icons::FOLDER_OPEN, t!("Выбрать файл…"), "soft", move || {
            std::thread::spawn(move || {
                let (title, filter, pats): (String, String, &[&str]) = if iso {
                    (t!("Образ установки"), t!("Образы дисков"), &["*.iso", "*.ISO", "*.img"])
                } else {
                    (t!("Диск машины"), t!("Диски"), &["*.qcow2", "*.img", "*.raw", "*.vmdk", "*.vdi"])
                };
                if let Some(p) = parts::choose_file(&title, &filter, pats, false) {
                    run_on_main_thread(move || set_path(st, p));
                }
            });
        });
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(picked).class("grow")).child(pick)).class("picked"));
        if iso {
            let found = find_isos();
            if !found.is_empty() {
                let mut list = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(t!("Найдены в «Загрузках»")).class("tile-label"));
                for p in found {
                    let s = p.to_string_lossy().into_owned();
                    let active = s == d.path;
                    let size = std::fs::metadata(&p).map(|m| parts::bytes(m.len())).unwrap_or_default();
                    let os = OsKind::guess(&s).unwrap_or(OsKind::Other);
                    list = list.child(
                        GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || set_path(st, s.clone())).child(
                            DecoratedBox::new()
                                .child(
                                    Row::new()
                                        .gap(10.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(Icon::new(parts::os_icon(os)).class("list-icon"))
                                        .child(Text::new(parts::file_name(&p.to_string_lossy())).max_lines(1).class("grow"))
                                        .child(Text::new(size).class("muted small")),
                                )
                                .class(if active { "list-item active" } else { "list-item" }),
                        ),
                    );
                }
                col = col.child(list);
            }
        }
        vec![boxed(col)]
    });
    boxed(Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(choice).child(file))
}

/// Мастер с уже выбранным файлом (открыт из Проводника): образ или диск.
pub fn open_with(st: St, path: String) {
    super::new_vm(st);
    let disk = ["qcow2", "img", "raw", "vmdk", "vdi"].iter().any(|e| path.to_ascii_lowercase().ends_with(e)) && !path.to_ascii_lowercase().ends_with(".iso");
    edit(st, |d| d.source = if disk { Source::Disk } else { Source::Iso });
    set_path(st, path);
}

/// Образ выбран: угадать систему и имя.
fn set_path(st: St, p: String) {
    edit(st, |d| {
        if d.source == Source::Iso {
            if let Some(os) = OsKind::guess(&p) {
                apply_os(d, os);
            }
        }
        if !d.named {
            d.vm.name = suggest_name(&p, d.vm.os);
        }
        d.path = p;
    });
}

fn apply_os(d: &mut Draft, os: OsKind) {
    let name = d.vm.name.clone();
    d.vm = os.preset();
    d.vm.name = name;
}

/// Имя из файла: «ubuntu-24.04.1-desktop-amd64.iso» → «Ubuntu 24.04.1».
fn suggest_name(path: &str, os: OsKind) -> String {
    let stem = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut words: Vec<String> = Vec::new();
    for w in stem.split(['-', '_', ' ', '+']) {
        let lw = w.to_ascii_lowercase();
        if w.is_empty() || ["amd64", "x86", "x64", "x86", "64", "desktop", "live", "dvd", "iso", "netinst", "release", "disc1", "english", "russian", "international", "workstation", "x86_64", "v1", "v2"].contains(&lw.as_str()) {
            continue;
        }
        let mut c = w.chars();
        let cap = match c.next() {
            Some(f) if f.is_ascii_lowercase() => f.to_ascii_uppercase().to_string() + c.as_str(),
            Some(f) => f.to_string() + c.as_str(),
            None => continue,
        };
        words.push(cap);
        if words.len() >= 3 {
            break;
        }
    }
    let name = if words.is_empty() { syngui::i18n::t(os.title()) } else { words.join(" ") };
    let name = if os == OsKind::Windows11 && !name.contains("11") { "Windows 11".to_string() } else { name };
    model::unique_name(&name, &model::load_all())
}

/// Свежие образы в «Загрузках» и «Рабочем столе».
fn find_isos() -> Vec<PathBuf> {
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for kind in ["DOWNLOAD", "DESKTOP"] {
        let Some(dir) = synshell_common::paths::user_dir(kind) else { continue };
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("iso")) {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                out.push((t, p));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.dedup_by(|a, b| a.1 == b.1);
    out.into_iter().take(5).map(|(_, p)| p).collect()
}

// ─── Шаг 2: система ─────────────────────────────────────────────────────

fn system(st: St, narrow: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let d = st.draft.get().unwrap_or_default();
        let mut cards: Vec<W> = Vec::new();
        for os in OsKind::ALL {
            let active = d.vm.os == os;
            cards.push(boxed(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || {
                        edit(st, |d| {
                            apply_os(d, os);
                            if !d.named && !d.path.is_empty() {
                                d.vm.name = suggest_name(&d.path, os);
                            }
                        })
                    })
                    .child(
                        DecoratedBox::new()
                            .child(
                                Row::new()
                                    .gap(12.0)
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .child(parts::badge(parts::os_icon(os), None, 40.0))
                                    .child(Column::new().gap(2.0).child(Text::new(syngui::i18n::t(os.title())).max_lines(1).class("row-title")).child(Text::new(syngui::i18n::t(os.hint())).max_lines(2).class("row-hint")).class("grow"))
                                    .child(if active { boxed(Icon::new(icons::CHECK_CIRCLE).class("option-check")) } else { boxed(DecoratedBox::new()) }),
                            )
                            .class(if active { "option active" } else { "option" }),
                    ),
            ));
        }
        vec![boxed(Grid::new(if narrow { 1 } else { 2 }).gap(10.0).children(cards))]
    }))
}

// ─── Шаг 3: параметры ───────────────────────────────────────────────────

fn params(st: St) -> W {
    let d = draft(st);
    let host_cpu = model::host_cpus();
    let host_mem = model::host_memory_mb();
    let name = TextField::with_text(d.vm.name.clone())
        .placeholder(t!("Имя машины"))
        .on_change(move |s: &str| {
            let s = s.to_string();
            if let Some(mut d) = st.draft.get_untracked() {
                d.vm.name = s;
                d.named = true;
                st.draft.set_always(Some(d));
            }
        })
        .class("field");
    let label = |f: fn(&Draft) -> String| -> W {
        boxed(Reactive::new(move || -> Vec<W> {
            let d = st.draft.get().unwrap_or_default();
            vec![boxed(Text::new(f(&d)).class("value-label"))]
        }))
    };
    let slider_row = |title: String, hint: String, s: Slider, l: W| -> W { parts::row(title, hint, Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(s).child(l)) };
    let cpu = Slider::new().range(1.0, host_cpu as f32).step(1.0).value(d.vm.cpus as f32).width(220.0).on_change(move |v| edit(st, |d| d.vm.cpus = v.round().max(1.0) as u32));
    let mem = Slider::new().range(512.0, host_mem.max(1024) as f32).step(256.0).value(d.vm.memory_mb as f32).width(220.0).on_change(move |v| edit(st, |d| d.vm.memory_mb = ((v / 256.0).round() * 256.0).max(512.0) as u32));
    let free_gb = model::free_space().map(|b| (b >> 30) as f32).unwrap_or(512.0);
    let disk = Slider::new().range(8.0, free_gb.clamp(16.0, 2048.0)).step(4.0).value(d.vm.disk_gb as f32).width(220.0).on_change(move |v| edit(st, |d| d.vm.disk_gb = (v / 4.0).round() as u32 * 4));
    let mut col = Column::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(parts::row_wide(t!("Имя"), String::new(), name))
        .child(slider_row(t!("Ядра"), t!("Из {n}", n = host_cpu), cpu, label(|d| parts::cores(d.vm.cpus))))
        .child(slider_row(t!("Память"), t!("Из {m}", m = parts::mem_mb(host_mem)), mem, label(|d| parts::mem_mb(d.vm.memory_mb))));
    if d.source != Source::Disk {
        col = col.child(slider_row(
            t!("Диск"),
            t!("Растёт по мере записи; свободно {v}", v = model::free_space().map(parts::bytes).unwrap_or_default()),
            disk,
            label(|d| t!("{n} ГБ", n = d.vm.disk_gb)),
        ));
    }
    col = col.child(parts::row(t!("Запустить сразу"), t!("Экран машины откроется здесь же"), Toggle::with_state(d.start).on_change(move |b| edit(st, |d| d.start = b))));
    let adv = Reactive::new(move || -> Vec<W> {
        let d = st.draft.get().unwrap_or_default();
        let chips = [
            syngui::i18n::t(d.vm.firmware.title()),
            if d.vm.tpm { "TPM 2.0".into() } else { String::new() },
            syngui::i18n::t(d.vm.disk_bus.title()),
            syngui::i18n::t(d.vm.nic.title()),
            syngui::i18n::t(d.vm.display.title()),
        ];
        let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(6.0);
        for c in chips.into_iter().filter(|c| !c.is_empty()) {
            row = row.child(DecoratedBox::new().child(Text::new(c).class("tag-label")).class("tag"));
        }
        let toggle = GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || edit(st, |d| d.advanced = !d.advanced)).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Железо")).class("row-title"))
                .child(DecoratedBox::new().child(row).class("grow"))
                .child(Icon::new(if d.advanced { icons::COLLAPSE } else { icons::EXPAND }).class("list-icon")),
        );
        let mut c = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(toggle);
        if d.advanced {
            let mut fw = Dropdown::new().width(200.0);
            for f in Firmware::ALL {
                fw = fw.item(DropdownItem::new(format!("{f:?}"), syngui::i18n::t(f.title())));
            }
            let fw = fw.selected(format!("{:?}", d.vm.firmware)).on_change(move |s: &str| {
                if let Some(f) = Firmware::ALL.iter().find(|f| format!("{f:?}") == s) {
                    edit(st, |d| d.vm.firmware = *f);
                }
            });
            let mut bus = Dropdown::new().width(160.0);
            for b in DiskBus::ALL {
                bus = bus.item(DropdownItem::new(format!("{b:?}"), syngui::i18n::t(b.title())));
            }
            let bus = bus.selected(format!("{:?}", d.vm.disk_bus)).on_change(move |s: &str| {
                if let Some(b) = DiskBus::ALL.iter().find(|b| format!("{b:?}") == s) {
                    edit(st, |d| d.vm.disk_bus = *b);
                }
            });
            c = c
                .child(parts::row(t!("Прошивка"), String::new(), fw))
                .child(parts::row(t!("TPM 2.0"), t!("Для Windows 11"), Toggle::with_state(d.vm.tpm).on_change(move |b| edit(st, |d| d.vm.tpm = b))))
                .child(parts::row(t!("Подключение диска"), String::new(), bus));
        }
        vec![boxed(c.class("row"))]
    });
    col = col.child(adv);
    boxed(col)
}

fn create(st: St) {
    let d = draft(st);
    let mut vm = d.vm.clone();
    if vm.name.trim().is_empty() {
        vm.name = model::unique_name(&syngui::i18n::t(vm.os.title()), &st.vms.get_untracked());
    }
    vm.name = vm.name.trim().to_string();
    let existing = match d.source {
        Source::Iso => {
            vm.cdrom = d.path.clone();
            vm.boot_cdrom = true;
            None
        }
        Source::Disk => Some(PathBuf::from(&d.path)),
        Source::Empty => None,
    };
    super::close_wizard(st);
    let start = d.start;
    state::background(st, "+new", t!("Создание машины…"), move || {
        let vm = model::create(vm, existing)?;
        let id = vm.id.clone();
        let name = vm.name.clone();
        run_on_main_thread(move || {
            state::reload(st);
            super::open(st, Some(id.clone()), Some(crate::state::Tab::Overview));
            if start {
                state::start(st, &id);
            }
        });
        Ok(Some(t!("Машина «{n}» создана", n = name)))
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn names() {
        let n = super::suggest_name("/x/ubuntu-24.04.1-desktop-amd64.iso", crate::model::OsKind::Linux);
        assert!(n.starts_with("Ubuntu 24.04.1"), "{n}");
        let n = super::suggest_name("/x/Win11_24H2_Russian_x64.iso", crate::model::OsKind::Windows11);
        assert!(n.starts_with("Win11") || n.starts_with("Windows 11"), "{n}");
    }
}
