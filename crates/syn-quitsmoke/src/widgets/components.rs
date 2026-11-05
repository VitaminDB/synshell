//! Переиспользуемые визуальные компоненты приложения.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::*;

use crate::app_state::{AppState, Screen};

/// Заголовок секции.
pub fn section_title(text: &str) -> impl Widget {
    Text::new(text).class("section-title")
}

/// Крупный заголовок экрана.
pub fn screen_title(text: &str) -> impl Widget {
    Text::new(text).class("screen-title")
}

/// Универсальная карточка (стилизуется через MSS-класс).
pub fn card(class: &str, child: impl Widget + 'static) -> impl Widget {
    DecoratedBox::new().child(child).clip(true).class(format!("card {class}"))
}

/// Карточка-показатель: иконка, значение, подпись.
pub fn stat_card(icon: &str, value: String, label: &str, tint: &str) -> impl Widget {
    let inner = mgui! {
        Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Start) => [
            Text::new(icon).class("stat-icon"),
            Text::new(value).class("stat-value"),
            Text::new(label).class("stat-label"),
        ]
    };
    DecoratedBox::new()
        .child(inner)
        .clip(true)
        .class(format!("card stat-card {tint}"))
}

/// Кольцо прогресса с текстом в центре.
pub fn ring(
    progress: f32,
    top: &str,
    big: &str,
    big_class: &str,
    bottom: &str,
    size: f32,
    ring_class: &str,
) -> impl Widget {
    let center = mgui! {
        Column::new()
            .width(size)
            .height(size)
            .gap(2.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center) => [
            Text::new(top).class("ring-top"),
            Text::new(big).class(big_class),
            Text::new(bottom).class("ring-bottom"),
        ]
    };
    let stack = Stack::new()
        .child(
            CircularProgress::with_value(progress.clamp(0.0, 1.0))
                .size(size)
                .stroke_width(14.0)
                .class(format!("hero-ring {ring_class}")),
        )
        .child(center);
    // Центрируем кольцо по горизонтали внутри растянутого контейнера.
    Row::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(stack)
}

/// Основная кнопка-действие.
pub fn primary_button(label: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new(label).on_click(on_click).class("btn-primary")
}

/// Мягкая вторичная кнопка.
pub fn soft_button(label: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new(label).on_click(on_click).class("btn-soft")
}

/// Нижняя панель навигации.
pub fn nav_bar(state: AppState) -> impl Widget {
    use crate::icons;
    let current = state.screen.get();
    let items = [
        (icons::HOME, "nav.home", Screen::Home),
        (icons::STATS, "nav.stats", Screen::Stats),
        (icons::HEALTH, "nav.health", Screen::Health),
        (icons::AWARDS, "nav.achievements", Screen::Achievements),
        (icons::SETTINGS, "nav.settings", Screen::Settings),
    ];

    let mut row = Row::new()
        .main_axis_alignment(MainAxisAlignment::SpaceAround)
        .cross_axis_alignment(CrossAxisAlignment::Center);

    for (icon, key, screen) in items {
        let active = current == screen;
        let label = state.tr(key);
        // Ширина — по контенту; горизонтальные поля задаёт .nav-item, чтобы
        // подсветка активного пункта имела отступы вокруг текста.
        let col = Column::new()
            .height(54.0)
            .gap(2.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(icon).class("nav-icon"))
            .child(Text::new(label).class("nav-label"));
        let cls = if active { "nav-item nav-item-active" } else { "nav-item" };
        // DecoratedBox (фон + паддинг + min-width) — прямой ребёнок Row, чтобы
        // размер корректно измерялся; клик — на внутреннем GestureDetector.
        let cell = DecoratedBox::new()
            .child(GestureDetector::new().child(col).on_click(move || state.go(screen)))
            .clip(true)
            .class(cls);
        row = row.child(cell);
    }

    DecoratedBox::new().child(row).class("nav-bar")
}

/// Обёртка основного экрана: прокручиваемый контент + нижняя навигация.
pub fn scaffold(state: AppState, content: impl Widget + 'static) -> impl Widget {
    let root = mgui! {
        Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("screen-root") => [
            ScrollView::new()
                .vertical()
                .child(Padding::symmetric(16.0, 16.0).child(content))
                .class("grow"),
            nav_bar(state),
        ]
    };
    // Фон красит DecoratedBox (Column свой background не рисует). Обёртка
    // растягивается вместе с контентом и переключается с темой (var(--bg)).
    DecoratedBox::new().child(root).class("app-bg")
}
