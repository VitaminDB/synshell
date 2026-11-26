//! Раздел «Настройки»: Hugging Face (токен, кэш, загрузки, форматы) и
//! каталог моделей. Строки и вид — как были в настройках synthos.

use syngui::prelude::*;
use syngui::widgets::{SpinBox, Toggle, ToolButton};

use crate::icons::{MI_CLOUD_DOWNLOAD, MI_FILTER_ALT, MI_FOLDER, MI_FOLDER_OPEN, MI_FOLDER_ZIP, MI_INVENTORY_2, MI_LOCK, MI_SPEED, MI_TUNE};
use crate::pages::huggingface::HuggingFaceCtx;
use crate::pages::syn_explorer::SynExplorerCtx;

type W = Box<dyn Widget>;

pub fn view() -> impl Widget {
    let hf = use_context::<HuggingFaceCtx>();
    let syn = use_context::<SynExplorerCtx>();
    let models_dir = use_signal(syn.models_dir.get_untracked().display().to_string());
    create_effect(move || {
        let v = models_dir.get();
        let p = crate::config::resolve_models_dir(&v);
        if syn.models_dir.get_untracked() != p {
            syn.models_dir.set(p);
        }
    });
    let body = Column::new()
        .gap(28.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("models-settings")
        .child(Text::new(tr!("models.page.settings")).class("settings-page-title"))
        .child(section(
            "Hugging Face".to_string(),
            vec![
                secret_row(MI_LOCK, tr!("settings.general.hf_token"), tr!("settings.general.hf_token.desc"), hf.token),
                folder_row(MI_FOLDER, tr!("settings.general.hf_cache_dir"), tr!("settings.general.hf_cache_dir.desc"), hf.cache_dir, "~/.local/share/synthos/hf"),
                count_row(MI_CLOUD_DOWNLOAD, tr!("settings.general.hf_concurrent"), tr!("settings.general.hf_concurrent.desc"), hf.concurrent_limit, 1, 8),
                count_row(MI_TUNE, tr!("settings.general.hf_segments"), tr!("settings.general.hf_segments.desc"), hf.segments_per_file, 1, 16),
                count_row(MI_SPEED, tr!("settings.general.hf_speed_limit"), tr!("settings.general.hf_speed_limit.desc"), hf.speed_limit_mbps, 0, 2000),
                switch_row(MI_FILTER_ALT, tr!("settings.general.hf_skip_formats"), tr!("settings.general.hf_skip_formats.desc"), hf.skip_unwanted_formats),
                switch_row(MI_FOLDER_ZIP, tr!("settings.general.hf_gguf"), tr!("settings.general.hf_gguf.desc"), hf.gguf_support),
            ],
        ))
        .child(section(
            tr!("models.page.library"),
            vec![folder_row(
                MI_INVENTORY_2,
                tr!("models.settings.models_dir"),
                tr!("models.settings.models_dir.desc"),
                models_dir,
                "~/Storage/syn_models",
            )],
        ));
    ScrollView::new().child(body)
}

fn section(title: String, rows: Vec<W>) -> impl Widget {
    let card = DecoratedBox::new()
        .class("settings-card")
        .child(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows));
    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(title).class("settings-section-title"))
        .child(card)
}

fn row_frame(icon: &'static str, title: String, desc: String, control: W) -> W {
    let inner = Row::new()
        .gap(16.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("settings-row-icon-wrap").child(Center::new().child(Icon::new(icon).class("settings-row-icon"))))
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .child(Text::new(title).class("settings-row-title"))
                    .child(Text::new(desc).class("settings-row-desc")),
            ),
        )
        .children(vec![control]);
    Box::new(DecoratedBox::new().class("settings-row").child(Padding::symmetric(24.0, 18.0).child(inner)))
}

/// Поле секрета (токен): символы скрыты.
fn secret_row(icon: &'static str, title: String, desc: String, value: RwSignal<String>) -> W {
    let control: W = Box::new(
        TextField::with_text(value.get_untracked()).obscure(true).placeholder("hf_…").on_change(move |s| value.set(s.to_string())).width(360.0),
    );
    row_frame(icon, title, desc, control)
}

/// Поле с кнопкой «обзор» (выбор папки).
fn folder_row(icon: &'static str, title: String, desc: String, value: RwSignal<String>, placeholder: &'static str) -> W {
    // Поле перестраивается только по выбору через «обзор»: при наборе
    // сигнал меняет само поле.
    let picked = use_signal(0u64);
    let field = Reactive::new(move || -> Vec<W> {
        picked.get();
        vec![Box::new(
            TextField::with_text(value.get_untracked()).placeholder(placeholder).on_change(move |s| value.set(s.to_string())).width(320.0),
        )]
    });
    let browse = ToolButton::new(MI_FOLDER_OPEN).on_click(move || {
        if let Some(path) = rfd::FileDialog::new().pick_folder() {
            value.set(path.display().to_string());
            picked.update(|n| *n += 1);
        }
    });
    let control: W = Box::new(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(vec![Box::new(field) as W, Box::new(browse) as W]),
    );
    row_frame(icon, title, desc, control)
}

fn switch_row(icon: &'static str, title: String, desc: String, value: RwSignal<bool>) -> W {
    let control: W = Box::new(Toggle::with_state(value.get_untracked()).on_change(move |v| value.set(v)));
    row_frame(icon, title, desc, control)
}

fn count_row(icon: &'static str, title: String, desc: String, value: RwSignal<u32>, min: u32, max: u32) -> W {
    let control: W = Box::new(
        SpinBox::new()
            .value(value.get_untracked() as f64)
            .range(min as f64, max as f64)
            .step(1.0)
            .decimal_places(0)
            .on_change(move |v| value.set(v.clamp(min as f64, max as f64) as u32))
            .width(140.0),
    );
    row_frame(icon, title, desc, control)
}
