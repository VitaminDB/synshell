//! Окно: «не хватает компонентов — поставить?» (вызов из Wine или `run`) и список всех компонентов.

mod ask;
mod list;

use std::sync::Arc;

use syngui::prelude::*;
use synshell_common::Config;

use crate::db::Db;
use crate::launch::Failed;
use crate::pe::Arch;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn icon(g: &str) -> syngui::widget::StyledWidget<Icon> {
    Icon::new(g).class("ic")
}

/// Кодовые точки Material Icons.
pub mod icons {
    pub const EXTENSION: &str = "\u{E87B}";
    pub const WARNING: &str = "\u{E002}";
    pub const DOWNLOAD: &str = "\u{E2C4}";
    pub const CHECK: &str = "\u{E86C}";
    pub const ERROR: &str = "\u{E000}";
    pub const DELETE: &str = "\u{E872}";
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Список компонентов.
    List,
    /// Программа не запустилась (вызов из Wine): поставить и запустить снова.
    Ask,
    /// `run`: перед запуском — поставить или запустить как есть.
    AskBeforeRun,
}

#[derive(Clone, PartialEq)]
pub enum Phase {
    Idle,
    Busy(String, Option<f32>),
    Done(String),
    Error(String),
}

/// Что предложить: известные пакеты (id, архитектура) и DLL, которых в базе нет.
#[derive(Clone, Default)]
pub struct Plan {
    pub items: Vec<(String, Arch)>,
    pub unknown: Vec<String>,
    /// Уже есть в общем каталоге, а Wine их не нашёл (Wine без патча syn-wine-helper).
    pub present: Vec<String>,
}

impl Plan {
    pub fn new(db: &Db, f: &Failed) -> Plan {
        let mut p = Plan::default();
        let Some(arch) = f.arch() else {
            p.unknown = f.dlls.clone();
            return p;
        };
        for d in &f.dlls {
            if crate::store::find_dll(arch, d).is_some() {
                p.present.push(d.clone());
                continue;
            }
            match db.for_dll(d, arch) {
                Some(pkg) => {
                    if !p.items.iter().any(|(id, _)| *id == pkg.id) {
                        p.items.push((pkg.id.clone(), arch));
                    }
                }
                None => p.unknown.push(d.clone()),
            }
        }
        p
    }

    /// Сколько скачать (источники не в кэше).
    pub fn download_size(&self, db: &Db) -> u64 {
        let mut seen = Vec::new();
        let mut n = 0;
        for (id, arch) in &self.items {
            let Some(src) = db.package(id).and_then(|p| p.pick(*arch)).and_then(|pk| db.source(&pk.source)) else { continue };
            if !seen.contains(&src.id) && !crate::fetch::cached(src).is_file() {
                seen.push(src.id.clone());
                n += src.size;
            }
        }
        n
    }
}

#[derive(Clone, Copy)]
pub struct St {
    pub mode: Mode,
    pub phase: RwSignal<Phase>,
    /// Счётчик изменений общего каталога — список перечитывает состояние.
    pub changed: RwSignal<u64>,
    pub failed: RwSignal<Option<Arc<Failed>>>,
    pub plan: RwSignal<Arc<Plan>>,
}

thread_local! {
    static DB: Arc<Db> = Arc::new(Db::load());
}

pub fn db() -> Arc<Db> {
    DB.with(Arc::clone)
}

impl St {
    pub fn new(mode: Mode, failed: Option<Failed>) -> St {
        let plan = failed.as_ref().map(|f| Plan::new(&db(), f)).unwrap_or_default();
        St { mode, phase: use_signal(Phase::Idle), changed: use_signal(0), failed: use_signal(failed.map(Arc::new)), plan: use_signal(Arc::new(plan)) }
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase.get_untracked(), Phase::Busy(..))
    }

    /// Установка в фоне; `then` — на главном потоке после успеха.
    pub fn install(&self, items: Vec<(String, Arch)>, then: impl FnOnce(St) + Send + 'static) {
        if self.busy() || items.is_empty() {
            return;
        }
        self.phase.set(Phase::Busy(t!("Подготовка…"), None));
        let st = *self;
        let db: Db = (*db()).clone();
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
            let r = crate::install::install(&db, &items, &mut |p| {
                // не чаще 10 раз в секунду
                if p.frac.is_some_and(|f| f > 0.0 && f < 1.0) && last.elapsed() < std::time::Duration::from_millis(100) {
                    return;
                }
                last = std::time::Instant::now();
                run_on_main_thread(move || st.phase.set(Phase::Busy(p.text, p.frac)));
            });
            run_on_main_thread(move || {
                st.changed.update(|n| *n += 1);
                match r {
                    Ok(_) => {
                        st.phase.set(Phase::Idle);
                        then(st);
                    }
                    Err(e) => st.phase.set(Phase::Error(format!("{e:#}"))),
                }
            });
        });
    }
}

pub fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../../styles/syn-wine-helper.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

/// «13,3 МБ»
pub fn size_mb(n: u64) -> String {
    t!("{n} МБ", n = synshell_common::decimal(n as f64 / 1048576.0, 1))
}

/// Ход установки или ошибка.
pub fn progress(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        match st.phase.get() {
            Phase::Idle => vec![],
            Phase::Busy(text, frac) => {
                let bar = match frac {
                    Some(f) => ProgressBar::new().value(f.clamp(0.0, 1.0)),
                    None => ProgressBar::new().indeterminate(),
                };
                vec![boxed(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(bar.class("bar")).child(Text::new(text).max_lines(2).class("muted small")))]
            }
            Phase::Done(text) => vec![boxed(
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(icons::CHECK).class("ic ok-ic")).child(Text::new(text).max_lines(4).class("grow")),
            )],
            Phase::Error(e) => vec![boxed(
                Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Start).child(icon(icons::ERROR).class("ic err-ic")).child(Text::new(t!("Не получилось: {e}", e = e)).max_lines(8).class("grow err-text")),
            )],
        }
    }))
}

fn root(st: St) -> W {
    let page = match st.mode {
        Mode::List => list::page(st),
        Mode::Ask | Mode::AskBeforeRun => ask::page(st),
    };
    boxed(DecoratedBox::new().class("root").child(page))
}

/// Окно; `false` — пользователь отказался (для `run`).
pub fn run(mode: Mode, failed: Option<Failed>) -> bool {
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let (title, size) = match mode {
        Mode::List => (t!("Компоненты Wine"), (520, 760)),
        _ => (t!("Нужны компоненты Wine"), (480, 560)),
    };
    App::new()
        .title(title)
        .app_id("syn-wine-helper")
        .size(size.0, size.1)
        .min_size(340, 420)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| root(St::new(mode, failed.clone())));
    ask::ACCEPTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// `--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page ask|ask-unknown|busy|error|list]`
pub fn screenshot(args: Vec<String>) {
    let mut it = args.into_iter();
    let Some(out) = it.next() else { return };
    let mut size = (480u32, 560u32);
    let mut scale = 2.0f64;
    let mut page = String::from("ask");
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = it.next().unwrap_or(page),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    let r = syngui::shot::render_png(
        move || {
            let demo = Failed {
                machine: 0x14c,
                exe: "\\??\\C:\\Games\\Fable\\Fable.exe".into(),
                dlls: if page == "ask-unknown" { vec!["mfc42.dll".into(), "binkw32.dll".into()] } else { vec!["mfc42.dll".into(), "msvbvm60.dll".into()] },
                ..Default::default()
            };
            let mode = if page == "list" { Mode::List } else { Mode::Ask };
            let st = St::new(mode, (mode != Mode::List).then_some(demo));
            match page.as_str() {
                "busy" => st.phase.set(Phase::Busy("MFC 4.2: скачивание 0,8 из 1,8 МБ".into(), Some(0.45))),
                "error" => st.phase.set(Phase::Error("не удалось скачать VC6RedistSetup_deu.exe: http status: 404".into())),
                _ => {}
            }
            root(st)
        },
        &mss,
        size,
        scale,
        &out,
    );
    match r {
        Ok((w, h)) => println!("{out}: {w}x{h}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
