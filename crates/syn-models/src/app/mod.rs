//! Окно «Моделей»: свой заголовок (CSD) с разделами «Hugging Face»,
//! «Библиотека» и «Настройки», под ним — открытый раздел.
//!
//! Один экземпляр: второй запуск поднимает окно первого (очередь загрузок и
//! её файл состояния — у одного процесса).

use syngui::input::Key;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::feedback::NotificationHost;
use syngui::widgets::overlay::PortalAnchor;
use syngui::widgets::{EventHook, KeyReply};
use synshell_common::Config;

use crate::config::AppConfig;
use crate::context::{AppCtx, Page};
use crate::pages::{huggingface, syn_explorer};

pub mod theme;

pub const APP_ID: &str = "syn-models";

type W = Box<dyn Widget>;

/// Состояние окна (развёрнуто, фокус) — для своего заголовка.
#[derive(Clone, Copy)]
pub struct WindowCtx {
    pub state: RwSignal<syngui::window::WindowState>,
}

struct Args {
    page: Option<Page>,
    screenshot: Option<String>,
    size: (u32, u32),
}

fn parse_args() -> Args {
    let mut a = Args { page: None, screenshot: None, size: (1280, 820) };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--page" => a.page = it.next().map(|p| Page::from_config(&p)),
            "--screenshot" => a.screenshot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x').map(|(w, h)| (w.parse().ok(), h.parse().ok()))) {
                    if let (Some(w), Some(h)) = (w, h) {
                        a.size = (w, h);
                    }
                }
            }
            "-h" | "--help" => {
                println!("syn-models [--page huggingface|library|settings] [--screenshot out.png [--size ШxВ]]");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    a
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();
    let args = parse_args();
    synshell_common::i18n::init(&crate::i18n::CATALOGS);
    let (shell_cfg, _) = Config::load();

    if let Some(out) = args.screenshot.clone() {
        let mss = theme::mss(&shell_cfg);
        let page = args.page;
        let r = syngui::shot::render_png(
            move || {
                install_state(false);
                if let Some(p) = page {
                    use_context::<AppCtx>().page.set(p);
                }
                Box::new(frame(false))
            },
            &mss,
            args.size,
            1.0,
            &out,
        );
        match r {
            Ok((w, h)) => println!("{out}: {w}x{h}"),
            Err(e) => {
                eprintln!("снимок: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let theme = use_signal(theme::mss(&shell_cfg));
    install_state(true);
    if let Some(p) = args.page {
        use_context::<AppCtx>().page.set(p);
    }
    watch_shell_config(theme);
    let window = use_context::<WindowCtx>().state;
    App::new()
        .title(syngui::tr!("models.title"))
        .app_id(APP_ID)
        .with_single_instance(APP_ID)
        .size(1280, 820)
        .min_size(720, 480)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(theme)
        .with_window_state(window)
        .run(|_| Box::new(root()));
}

/// Контексты разделов, очередь загрузок и автосохранение настроек — один раз
/// на процесс. `live` — настоящий запуск (в снимке загрузки не продолжаются
/// и настройки не пишутся).
fn install_state(live: bool) {
    let cfg = AppConfig::load();
    let app = AppCtx::new(&cfg);
    provide_context(app.clone());
    provide_context(WindowCtx { state: use_signal(syngui::window::WindowState::default()) });
    provide_context(syn_explorer::SynExplorerCtx::new(&cfg));
    let hf = huggingface::HuggingFaceCtx::new(&cfg);
    provide_context(hf);
    if !live {
        return;
    }
    // Прерванная очередь загрузок: Pending/Active → Pending, Done остаётся.
    let persisted = huggingface::persist::load();
    huggingface::download::resume_persisted(hf, app.notifications.clone(), persisted);
    huggingface::persist::install_autosave(hf);
    install_config_autosave(app, cfg);
    for (file, backup) in crate::fsutil::take_quarantined() {
        notify_quarantined(&file, &backup);
    }
    crate::fsutil::set_notifier(|file, backup| {
        syngui::async_runtime::run_on_main_thread(move || notify_quarantined(&file, &backup));
    });
}

fn notify_quarantined(file: &std::path::Path, backup: &std::path::Path) {
    use_context::<AppCtx>().notifications.error(syngui::tr!(
        "app.file.quarantined",
        file = crate::paths::pretty(file),
        backup = crate::paths::pretty(backup)
    ));
}

/// Настройки разделов — в файл при каждом изменении.
fn install_config_autosave(app: AppCtx, base: AppConfig) {
    let syn = use_context::<syn_explorer::state::SynExplorerCtx>();
    let hf = use_context::<huggingface::HuggingFaceCtx>();
    let last = use_signal(base.clone());
    create_effect(move || {
        let cfg = AppConfig {
            syn_explorer_bookmarks: syn.bookmarks.get().iter().map(|p| p.display().to_string()).collect(),
            syn_explorer_left_split_ratio: syn.left_split_ratio.get(),
            syn_explorer_right_split_ratio: syn.right_split_ratio.get(),
            syn_explorer_expert: syn.wizard.expert.get(),
            syn_explorer_left: app.panels.syn_explorer.0.get(),
            syn_explorer_right: app.panels.syn_explorer.1.get(),
            hf_cache_dir: hf.cache_dir.get(),
            hf_concurrent_downloads: hf.concurrent_limit.get(),
            hf_segments_per_file: hf.segments_per_file.get(),
            hf_speed_limit_mbps: hf.speed_limit_mbps.get(),
            hf_skip_unwanted_formats: hf.skip_unwanted_formats.get(),
            hf_gguf_support: hf.gguf_support.get(),
            hf_files_view: hf.files_view_mode.get().as_config().to_string(),
            hf_dock_expanded: hf.dock_expanded.get(),
            hf_token: hf.token.get(),
            hf_left_split_ratio: app.hf_left_split_ratio.get(),
            hf_right_split_ratio: app.hf_right_split_ratio.get(),
            huggingface_left: app.panels.huggingface.0.get(),
            huggingface_right: app.panels.huggingface.1.get(),
            models_dir: syn.models_dir.get().display().to_string(),
            page: app.page.get().as_config().to_string(),
        };
        if last.get_untracked() != cfg {
            cfg.save();
            last.set(cfg);
        }
    });
}

/// Тема и язык меняются вместе с оформлением оболочки.
fn watch_shell_config(theme: RwSignal<String>) {
    synshell_common::live::spawn(Vec::new(), move |cfg| {
        let mss = theme::mss(&cfg);
        syngui::async_runtime::run_on_main_thread(move || {
            synshell_common::i18n::apply(&cfg);
            if theme.get_untracked() != mss {
                theme.set(mss);
            }
        });
    });
}

fn tab(page: Page, icon: &'static str, label: String) -> W {
    let app = use_context::<AppCtx>();
    let selected = app.page.get() == page;
    Box::new(
        Button::new(label)
            .icon(icon)
            .class(if selected { "tb-tab selected" } else { "tb-tab" })
            .on_click(move || app.page.set(page)),
    )
}

/// Свой заголовок окна: значок и имя, разделы, кнопки окна.
fn titlebar() -> W {
    let window = use_context::<WindowCtx>().state;
    let tabs = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        vec![Box::new(
            Row::new()
                .gap(4.0)
                .class("tb-tabs")
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(tab(Page::HuggingFace, crate::icons::MI_CLOUD_DOWNLOAD, "Hugging Face".into()))
                .child(tab(Page::Library, crate::icons::MI_INVENTORY_2, syngui::tr!("models.page.library")))
                .child(tab(Page::Settings, crate::icons::MI_SETTINGS, syngui::tr!("models.page.settings"))),
        )]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let w = window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let brand = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        vec![Box::new(Text::new(syngui::tr!("models.title")).class("tb-brand"))]
    });
    Box::new(
        DecoratedBox::new().class("titlebar").child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(crate::icons::MI_MEMORY).class("tb-brand-icon"))
                        .child(brand)
                        .child(DecoratedBox::new().class("tb-gap")),
                )
                .child(tabs)
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(WindowDragRegion::new().child(DecoratedBox::new().class("drag-space grow"))),
                )
                .child(DecoratedBox::new().class("window-controls").child(controls)),
        ),
    )
}

/// Открытый раздел.
fn page_view() -> W {
    let app = use_context::<AppCtx>();
    Box::new(DecoratedBox::new().class("grow page-body").child(Stack::new().fit(StackFit::Expand).child(Reactive::new(
        move || -> Vec<W> {
            syngui::i18n::subscribe();
            match app.page.get() {
                Page::HuggingFace => vec![Box::new(huggingface::view())],
                Page::Library => vec![Box::new(syn_explorer::view())],
                Page::Settings => vec![Box::new(crate::pages::settings::view())],
            }
        },
    ))))
}

fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

pub(crate) fn root() -> impl Widget {
    let window = use_context::<WindowCtx>().state;
    Reactive::new(move || -> Vec<W> {
        let fs = window.get().fullscreen;
        vec![Box::new(frame(!fs && !is_phone()))]
    })
}

/// Каркас: заголовок, раздел, тосты; `window` — рамка и изменение размера.
fn frame(window: bool) -> impl Widget {
    let app = use_context::<AppCtx>();
    let col = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(titlebar()).child(page_view());
    let always = use_signal(true);
    let toasts = Portal::new()
        .is_open(always)
        .modal(false)
        .backdrop(false)
        .anchor(PortalAnchor::BottomEnd { margin_bottom: 16.0, margin_right: 16.0 })
        .child(NotificationHost::new(app.notifications.clone()).grow_up(true).class("synthos-notification-host"));
    let stack = Stack::new()
        .clip(false)
        .fit(StackFit::Expand)
        .child(DecoratedBox::new().class("window-backdrop").child(DecoratedBox::new().clip(true).class("shell").child(col)))
        .child(toasts);
    let keys = EventHook::new()
        .on_key_down(move |k, m| {
            // Ctrl+1/2/3 — разделы.
            if m.ctrl {
                let page = match k {
                    Key::Num1 => Some(Page::HuggingFace),
                    Key::Num2 => Some(Page::Library),
                    Key::Num3 => Some(Page::Settings),
                    _ => None,
                };
                if let Some(p) = page {
                    app.page.set(p);
                    return KeyReply::Handled;
                }
            }
            KeyReply::Ignore
        })
        .child(stack);
    WindowResizeRegion::new().over_content(true).inset(5.0).enabled(window).child(
        DecoratedBox::new().class(if window { "window-frame" } else { "window-frame flat" }).clip(true).child(keys),
    )
}
