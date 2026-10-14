//! Библиотека: карточки машин с живыми экранами, быстрый запуск, пустое состояние.

use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::LiveView;
use syngui::GestureDetector;
use syngui::CursorIcon;

use super::parts::{self, boxed, btn, icon_btn, spacer, W};
use crate::model::Vm;
use crate::qemu;
use crate::state::{self, icons, Power, St, Tab};

pub fn page(st: St, narrow: bool) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let vms = st.vms.get();
        let live = st.live.get();
        let busy = st.busy.get();
        st.thumb_tick.get();
        let vw = viewport_size().get().width;
        let w = if narrow { vw - 28.0 } else { (vw - 290.0 - 56.0).max(320.0) };
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(head(st, &vms, &live, narrow));
        if let Some(b) = host_banner() {
            col = col.child(b);
        }
        if vms.is_empty() {
            col = col.child(welcome(st));
        } else {
            let gap = 16.0;
            let cols = ((w + gap) / (300.0 + gap)).floor().clamp(1.0, 5.0) as usize;
            let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
            let cards: Vec<W> = vms
                .iter()
                .map(|vm| {
                    let l = live.get(&vm.id).cloned().unwrap_or_default();
                    card(st, vm, &l, busy.get(&vm.id).cloned(), cw)
                })
                .collect();
            col = col.child(Grid::new(cols).gap(gap).children(cards));
        }
        vec![boxed(col.class(if narrow { "page narrow" } else { "page" }))]
    });
    let mut stack = Stack::new().fit(StackFit::Expand).child(ScrollView::new().vertical().child(content));
    if narrow {
        stack = stack.child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Padding::only(0.0, 0.0, 18.0, 22.0).child(GestureDetector::new().on_click(move || super::new_vm(st)).child(DecoratedBox::new().child(Icon::new(icons::ADD).class("fab-icon")).class("fab")))),
        );
    }
    boxed(stack)
}

fn head(st: St, vms: &[Vm], live: &std::collections::HashMap<String, state::Live>, narrow: bool) -> W {
    let running = live.values().filter(|l| l.power.on()).count();
    let sub = if vms.is_empty() {
        t!("Создайте машину из образа системы — Linux, Windows, BSD")
    } else if running > 0 {
        t!("{n} всего · {r} работает", n = vms.len(), r = running)
    } else {
        t!("{n} всего", n = vms.len())
    };
    let mut row = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Column::new().gap(3.0).child(Text::new(t!("Виртуальные машины")).max_lines(1).class("h0")).child(Text::new(sub).max_lines(2).class("muted")).class("grow"));
    if !narrow && !vms.is_empty() {
        row = row.child(btn(icons::ADD, t!("Новая машина"), "primary", move || super::new_vm(st)));
    }
    boxed(row)
}

/// Предупреждение, если нет QEMU или KVM.
fn host_banner() -> Option<W> {
    let (icon, cls, title, text) = if !qemu::qemu_installed() {
        (icons::ERROR, "banner bad", t!("QEMU не установлен"), t!("Установите пакет qemu-desktop (или qemu-full): sudo pacman -S qemu-desktop edk2-ovmf swtpm"))
    } else if !qemu::kvm_available() {
        (
            icons::WARNING,
            "banner warn",
            t!("Аппаратная виртуализация недоступна"),
            t!("/dev/kvm не открывается — машины будут работать в эмуляции, в десятки раз медленнее. Включите VT-x/AMD-V в BIOS и проверьте права на /dev/kvm."),
        )
    } else {
        return None;
    };
    Some(boxed(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(icon).class("banner-icon"))
                    .child(Column::new().gap(2.0).child(Text::new(title).class("row-title")).child(Text::new(text).max_lines(4).selectable(true).class("row-hint")).class("grow")),
            )
            .class(cls),
    ))
}

/// Первый запуск: что умеет менеджер и большая кнопка.
fn welcome(st: St) -> W {
    let feat = |icon: &str, title: String, text: String| -> W {
        boxed(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(8.0)
                        .child(parts::badge(icon, None, 38.0))
                        .child(Text::new(title).max_lines(2).class("row-title"))
                        .child(Text::new(text).max_lines(4).class("row-hint")),
                )
                .class("feature"),
        )
    };
    let vw = viewport_size().get_untracked().width;
    let cols = if vw < 700.0 { 1 } else if vw < 1100.0 { 2 } else { 4 };
    boxed(
        Column::new()
            .gap(22.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                DecoratedBox::new()
                    .child(
                        Column::new()
                            .gap(14.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().child(Icon::new(icons::LAYERS).class("hero-icon")).class("hero-badge"))
                            .child(Text::new(t!("Здесь будут ваши машины")).class("hero-title"))
                            .child(Text::new(t!("Выберите образ установки — менеджер подберёт железо под систему, а экран машины откроется прямо в окне.")).max_lines(3).class("hero-text"))
                            .child(btn(icons::ADD, t!("Создать машину"), "primary big", move || super::new_vm(st))),
                    )
                    .class("hero"),
            )
            .child(Grid::new(cols).gap(12.0).children(vec![
                feat(icons::SPARK, t!("Под систему"), t!("Windows 11 — с TPM и Secure Boot, Linux — с virtio. Ничего не нужно настраивать вручную.")),
                feat(icons::MONITOR, t!("Экран в окне"), t!("Разрешение гостя подстраивается под окно, общий буфер обмена, звук.")),
                feat(icons::HISTORY, t!("Снимки"), t!("Сохраните состояние перед экспериментом и вернитесь к нему за секунды.")),
                feat(icons::FOLDER, t!("Общая папка"), t!("Папка компьютера видна в госте, а порты гостя — снаружи.")),
            ])),
    )
}

fn card(st: St, vm: &Vm, l: &state::Live, busy: Option<String>, width: f32) -> W {
    let id = vm.id.clone();
    let h = (width * 10.0 / 16.0).round();
    let accent = vm.accent();
    let preview: W = match state::thumb(vm) {
        Some(f) => {
            let mut v = DecoratedBox::new().child(LiveView::new(f).fit(ImageFit::Cover).class("thumb-img")).clip(true).class("thumb");
            if !l.power.on() {
                v = v.style("opacity", StyleValue::Number(0.55));
            }
            boxed(v.style("height", StyleValue::px(h)))
        }
        None => boxed(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(parts::os_icon(vm.os)).style("icon-color", accent).style("icon-size", StyleValue::px(h * 0.3))),
                )
                .class("thumb placeholder")
                .style("height", StyleValue::px(h))
                .style("background-color", accent.with_alpha(0.12)),
        ),
    };
    let preview = Stack::new().child(preview).child(Padding::all(10.0).child(Row::new().child(parts::status_chip(l.power, busy.clone()))));
    let sub = match l.power {
        Power::Off => format!("{} · {}", syngui::i18n::t(vm.os.title()), parts::ago(vm.last_run)),
        _ => t!("ЦП {c}% · {m}", c = format!("{:.0}", l.cpu), m = parts::bytes(l.rss)),
    };
    let (a, b) = (id.clone(), id.clone());
    let action: W = if busy.is_some() {
        boxed(DecoratedBox::new().class("spacer-btn"))
    } else if l.power.on() {
        icon_btn(icons::MONITOR, t!("Экран"), move || super::open(st, Some(a.clone()), Some(Tab::Screen)))
    } else {
        boxed(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || state::start(st, &b))
                .child(DecoratedBox::new().child(Icon::new(icons::PLAY).class("play-icon")).class("play")),
        )
    };
    let info = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(parts::badge(parts::os_icon(vm.os), Some(accent), 34.0))
        .child(Column::new().gap(2.0).child(Text::new(vm.name.clone()).max_lines(1).class("card-name")).child(Text::new(sub).max_lines(1).class("muted small")).class("grow"))
        .child(action);
    let col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(preview).child(info);
    let _ = spacer;
    boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(move || super::open(st, Some(id.clone()), None)).child(DecoratedBox::new().child(col).class("vm-card")))
}
