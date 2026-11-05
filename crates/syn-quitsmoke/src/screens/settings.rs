//! Экран настроек.

use syngui::mgui;
use syngui::prelude::*;
use syngui::GestureDetector;

use crate::app_state::AppState;
use crate::icons;
use crate::model::profile::VapeProfile;
use crate::model::settings::{Locale, ReminderStyle};
use crate::widgets::components::{screen_title, section_title};

pub fn build(state: AppState) -> impl Widget {
    let confirm = use_signal(false);
    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            move || body(state, confirm),
            // Диалог подтверждения — вне реактивного body, чтобы сохранять состояние.
            // Тема и подписи кнопок берутся из виджета syngui (класс .dialog +
            // локализованные метки, заданные через syngui::set_dialog_labels).
            ConfirmDialog::new(
                state.tr("settings.reset"),
                state.tr("settings.reset_confirm"),
                confirm,
                move |ok| { if ok { state.reset_all(); } },
            ),
        ]
    }
}

/// Карточка-секция: колонка строк со скруглением, рамкой и разделителями.
fn card(child: impl Widget + 'static) -> impl Widget {
    DecoratedBox::new().child(child).clip(true).class("set-card")
}

/// Тонкий разделитель между строками внутри карточки.
fn divider() -> impl Widget {
    DecoratedBox::new().class("set-divider")
}


/// Заголовок секции (маленький, приглушённый).
fn section(title: impl Widget + 'static, body: impl Widget + 'static) -> impl Widget {
    mgui! {
        Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            title,
            body,
        ]
    }
}

/// Строка настройки: иконка + подпись слева, контрол справа.
fn row(icon: &str, label: &str, control: impl Widget + 'static) -> impl Widget {
    let left = mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(icon).class("set-ico"),
            Text::new(label).class("setting-label"),
        ]
    };
    mgui! {
        Row::new()
            .main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("set-row") => [
            left,
            control,
        ]
    }
}

/// Строка с контролом во всю ширину под подписью (для широких сегментов).
fn row_stacked(icon: &str, label: &str, control: impl Widget + 'static) -> impl Widget {
    let header = mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(icon).class("set-ico"),
            Text::new(label).class("setting-label"),
        ]
    };
    mgui! {
        Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("set-row") => [
            header,
            control,
        ]
    }
}

/// Кликабельная строка-действие (нейтральная или деструктивная).
fn action(
    icon: &str,
    label: &str,
    danger: bool,
    on_click: impl FnMut() + Send + 'static,
) -> impl Widget {
    let cls = if danger { "set-action set-action-danger" } else { "set-action" };
    let inner = mgui! {
        Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(icon).class("set-ico"),
            Text::new(label).class("setting-label"),
        ]
    };
    DecoratedBox::new()
        .child(GestureDetector::new().child(inner).on_click(on_click))
        .class(cls)
}

fn body(state: AppState, confirm: RwSignal<bool>) -> impl Widget {
    let st = state.settings.get();

    let dark = Toggle::new()
        .on(st.dark_mode)
        .on_change(move |v| state.set_dark(v))
        .class("set-toggle");

    // Язык: «Как в системе» и все языки каталогов (как в остальных программах synshell).
    let system = state.tr("settings.language.system").to_string();
    let langs: Vec<(String, String)> =
        syngui::i18n::languages().into_iter().map(|l| (l.tag.tag().to_string(), l.name)).collect();
    let current = if st.locale.is_auto() {
        system.clone()
    } else {
        langs.iter().find(|(t, _)| t == st.locale.tag()).map(|(_, n)| n.clone()).unwrap_or_else(|| system.clone())
    };
    let mut lang = Dropdown::new().item(DropdownItem::simple(system.clone()));
    for (_, name) in &langs {
        lang = lang.item(DropdownItem::simple(name.clone()));
    }
    let lang = lang
        .selected(current)
        .on_change(move |v| {
            let tag = langs.iter().find(|(_, n)| n == v).map(|(t, _)| t.as_str()).unwrap_or("auto");
            state.set_locale(Locale::new(tag));
        })
        .class("set-dropdown");

    let currency = Dropdown::new()
        .item(DropdownItem::simple("₸"))
        .item(DropdownItem::simple("₽"))
        .item(DropdownItem::simple("$"))
        .item(DropdownItem::simple("€"))
        .selected(st.currency_symbol.clone())
        .width(96.0)
        .on_change(move |v| state.set_currency(v.to_string()))
        .class("set-dropdown");

    let notif = Toggle::new()
        .on(st.notifications_enabled)
        .on_change(move |v| state.set_notifications(v))
        .class("set-toggle");

    let reminder = SegmentedButton::new(vec![
        state.tr("settings.reminder.chrono"),
        state.tr("settings.reminder.bar"),
        state.tr("settings.reminder.both"),
    ])
    .selected(match st.reminder_style {
        ReminderStyle::Chronometer => 0,
        ReminderStyle::ProgressBar => 1,
        ReminderStyle::Both => 2,
    })
    .on_change(move |i| {
        let style = match i {
            0 => ReminderStyle::Chronometer,
            1 => ReminderStyle::ProgressBar,
            _ => ReminderStyle::Both,
        };
        state.set_reminder_style(style);
    })
    .class("set-seg");

    let appearance = card(mgui! {
        Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            row(icons::MOON, state.tr("settings.dark"), dark),
            divider(),
            row(icons::TRANSLATE, state.tr("settings.language"), lang),
        ]
    });

    let general = card(mgui! {
        Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            row(icons::DOLLAR, state.tr("settings.currency"), currency),
            divider(),
            row(icons::BELL, state.tr("settings.notifications"), notif),
            divider(),
            row_stacked(icons::TUNE, state.tr("settings.reminder_style"), reminder),
        ]
    });

    let data = card(mgui! {
        Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            action(icons::TRASH, state.tr("settings.reset"), true, move || confirm.set(true)),
            divider(),
            action(icons::RESTART, state.tr("settings.restart_onboarding"), false,
                move || state.restart_onboarding()),
        ]
    });

    // ── Vape-секция ──────────────────────────────────────────────────────────
    let vp = state.profile_vape.get();
    let vape_on = state.vape_enabled.get();

    let vape_toggle = Toggle::new()
        .on(vape_on)
        .on_change(move |v| state.set_vape_enabled(v))
        .class("set-toggle");

    let vapes_day = number_field(state.tr("vape.vapes_per_day"), vp.vapes_per_day as f64, 1.0, 100.0, 1.0, {
        let s = state;
        move |v| s.profile_vape.update(|p| p.vapes_per_day = v.round().max(1.0) as u32)
    });

    let price_pod = number_field(state.tr("vape.price_per_pod"), vp.price_per_pod, 1.0, 10000.0, 10.0, {
        let s = state;
        move |v| s.profile_vape.update(|p| p.price_per_pod = v)
    });

    let puffs_pod = number_field(state.tr("vape.puffs_per_pod"), vp.puffs_per_pod as f64, 1.0, 5000.0, 10.0, {
        let s = state;
        move |v| s.profile_vape.update(|p| p.puffs_per_pod = v.round().max(1.0) as u32)
    });

    let years_vape = number_field(state.tr("vape.years_vaping"), vp.years_vaping as f64, 0.0, 70.0, 1.0, {
        let s = state;
        move |v| s.profile_vape.update(|p| p.years_vaping = v as f32)
    });

    let vape_section_desc = Text::new(state.tr("vape.section.desc")).class("card-body");

    let vape_card = card(mgui! {
        Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            row(icons::VAPE, state.tr("vape.enabled"), vape_toggle),
            divider(),
            vapes_day,
            divider(),
            price_pod,
            divider(),
            puffs_pod,
            divider(),
            years_vape,
        ]
    });

    mgui! {
        Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            screen_title(state.tr("settings.title")),
            section(section_title(state.tr("settings.section.appearance")), appearance),
            section(section_title(state.tr("settings.section.app")), general),
            section(section_title(state.tr("vape.title")), mgui! {
                Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
                    vape_section_desc,
                    vape_card,
                ]
            }),
            section(section_title(state.tr("settings.section.data")), data),
        ]
    }
}

/// Числовое поле для настроек (label + input + clamp + callback).
fn number_field(
    label: &str,
    value: f64,
    _min: f64,
    _max: f64,
    _step: f64,
    on_change: impl Fn(f64) + Send + 'static,
) -> impl Widget {
    let text = if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    };
    mgui! {
        Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("field") => [
            Text::new(label).class("field-label"),
            TextField::with_text(text).on_change(move |s: &str| {
                let cleaned: String = s.trim().replace(',', ".");
                if let Ok(v) = cleaned.parse::<f64>() {
                    on_change(v);
                }
            }).class("onb-input"),
        ]
    }
}
