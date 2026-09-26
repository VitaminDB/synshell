//! Страницы настроек и их реестр (навигация, поиск).

use crate::ui::{icons, W};

mod appearance;
mod input;
mod panels;
mod system;
mod windows;

pub use input::capture_hook;

pub struct PageDef {
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub group: &'static str,
    /// Слова для поиска: названия настроек на странице.
    pub keywords: &'static str,
    pub build: fn() -> W,
}

pub const PAGES: &[PageDef] = &[
    PageDef {
        id: "appearance",
        title: "Внешний вид",
        icon: icons::PALETTE,
        group: "Оформление",
        keywords: "тема схема тёмная светлая акцент цвет шрифт размер скругление прозрачность масштаб значки иконки курсор палитра",
        build: appearance::appearance,
    },
    PageDef {
        id: "wallpaper",
        title: "Обои",
        icon: icons::WALLPAPER,
        group: "Оформление",
        keywords: "фон картинка изображение слайд-шоу градиент цвет рабочий стол значки",
        build: appearance::wallpaper,
    },
    PageDef {
        id: "panels",
        title: "Панели",
        icon: icons::PANEL,
        group: "Оформление",
        keywords: "панель задач апплеты виджеты часы трей меню край размер автоскрытие плавающая",
        build: panels::panels,
    },
    PageDef {
        id: "decorations",
        title: "Оформление окон",
        icon: icons::STYLE,
        group: "Оформление",
        keywords: "заголовок рамка кнопки закрыть свернуть развернуть тень скругление цвет",
        build: appearance::decorations,
    },
    PageDef {
        id: "animations",
        title: "Анимации",
        icon: icons::ANIMATION,
        group: "Оформление",
        keywords: "эффекты скорость открытие закрытие сворачивание переключение столов",
        build: appearance::animations,
    },
    PageDef {
        id: "windows",
        title: "Поведение окон",
        icon: icons::WINDOW,
        group: "Рабочее пространство",
        keywords: "фокус размещение рамки модификатор зазоры прилипание snap плитка мозаика заголовок затемнение раскладка мастер",
        build: windows::windows,
    },
    PageDef {
        id: "workspaces",
        title: "Рабочие столы",
        icon: icons::WORKSPACES,
        group: "Рабочее пространство",
        keywords: "виртуальные столы количество имена раскладка",
        build: windows::workspaces,
    },
    PageDef {
        id: "rules",
        title: "Правила окон",
        icon: icons::RULES,
        group: "Рабочее пространство",
        keywords: "app_id заголовок плавающее стол размер положение поверх прозрачность",
        build: windows::rules,
    },
    PageDef {
        id: "launcher",
        title: "Меню запуска",
        icon: icons::APPS,
        group: "Рабочее пространство",
        keywords: "приложения избранное поиск калькулятор категории",
        build: system::launcher,
    },
    PageDef {
        id: "notifications",
        title: "Уведомления",
        icon: icons::NOTIFICATIONS,
        group: "Рабочее пространство",
        keywords: "всплывающие не беспокоить история время",
        build: system::notifications,
    },
    PageDef {
        id: "keyboard",
        title: "Клавиатура",
        icon: icons::KEYBOARD,
        group: "Устройства",
        keywords: "раскладка язык переключение автоповтор numlock xkb",
        build: input::keyboard,
    },
    PageDef {
        id: "shortcuts",
        title: "Комбинации клавиш",
        icon: icons::SHORTCUTS,
        group: "Устройства",
        keywords: "сочетания горячие клавиши действия hotkeys",
        build: input::shortcuts,
    },
    PageDef {
        id: "mouse",
        title: "Мышь и тачпад",
        icon: icons::MOUSE,
        group: "Устройства",
        keywords: "указатель ускорение прокрутка естественная касание тачпад левша",
        build: input::mouse,
    },
    PageDef {
        id: "displays",
        title: "Мониторы",
        icon: icons::DISPLAY,
        group: "Устройства",
        keywords: "экран разрешение частота масштаб поворот положение vrr основной",
        build: system::displays,
    },
    PageDef {
        id: "lock",
        title: "Блокировка и простой",
        icon: icons::LOCK,
        group: "Система",
        keywords: "экран блокировки сон простой dpms гашение",
        build: system::lock,
    },
    PageDef {
        id: "autostart",
        title: "Автозапуск",
        icon: icons::AUTOSTART,
        group: "Система",
        keywords: "запуск при входе программы xdg autostart",
        build: system::autostart,
    },
    PageDef {
        id: "general",
        title: "Сеанс",
        icon: icons::SETTINGS,
        group: "Система",
        keywords: "терминал файловый менеджер браузер оболочка переменные окружения xwayland снимки",
        build: system::general,
    },
    PageDef {
        id: "about",
        title: "О системе",
        icon: icons::INFO,
        group: "Система",
        keywords: "версия ядро процессор память видеокарта",
        build: system::about,
    },
];

pub fn find(id: &str) -> &'static PageDef {
    PAGES.iter().find(|p| p.id == id).unwrap_or(&PAGES[0])
}

/// Страница подходит под строку поиска (все слова запроса встречаются
/// в названии или ключевых словах).
pub fn matches(p: &PageDef, query: &str) -> bool {
    let hay = format!("{} {} {}", p.title, p.keywords, p.group).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}
