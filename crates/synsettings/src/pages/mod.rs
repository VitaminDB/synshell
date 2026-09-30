//! Страницы настроек и их реестр (навигация, поиск).

use crate::ui::{icons, W};

mod appearance;
mod hardware;
mod input;
mod mobile;
mod net;
mod panels;
mod power;
mod system;
mod themes;
mod wallpaper;
mod windows;

pub use input::capture_hook;
pub use themes::gallery_mss as themes_gallery_mss;
pub use wallpaper::{close_editor as close_wallpaper_editor, open_editor as open_wallpaper_editor};

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
        id: "themes",
        title: "Темы",
        icon: icons::THEMES,
        group: "Оформление",
        keywords: "тема оформление стиль палитра nord catppuccin gruvbox dracula tokyo night synthwave неон стекло",
        build: themes::themes,
    },
    PageDef {
        id: "appearance",
        title: "Внешний вид",
        icon: icons::PALETTE,
        group: "Оформление",
        keywords: "тема схема тёмная светлая акцент цвет шрифт размер скругление прозрачность масштаб значки иконки курсор мышь скрыть курсор палитра",
        build: appearance::appearance,
    },
    PageDef {
        id: "wallpaper",
        title: "Обои",
        icon: icons::WALLPAPER,
        group: "Оформление",
        keywords: "фон картинка изображение фото слайд-шоу градиент цвет рабочий стол столы значки кадр масштаб панорама параллакс галерея каталог",
        build: wallpaper::build,
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
        keywords: "экран разрешение частота масштаб поворот положение vrr основной яркость ниты подсветка",
        build: system::displays,
    },
    PageDef {
        id: "wifi",
        title: "Wi-Fi",
        icon: icons::WIFI,
        group: "Устройства",
        keywords: "беспроводная сеть интернет пароль подключение iwd networkmanager точка доступа",
        build: net::wifi,
    },
    PageDef {
        id: "bluetooth",
        title: "Bluetooth",
        icon: icons::BLUETOOTH,
        group: "Устройства",
        keywords: "наушники колонка сопряжение устройство клавиатура мышь bluez",
        build: net::bluetooth,
    },
    PageDef {
        id: "phone",
        title: "Телефон",
        icon: icons::PHONE,
        group: "Устройства",
        keywords: "режим окон страницы свободный стол ручка назад домашний экран ресурсы мобильный",
        build: mobile::phone,
    },
    PageDef {
        id: "gestures",
        title: "Жесты",
        icon: icons::GESTURE,
        group: "Устройства",
        keywords: "свайп край назад домой шторка недавние удержание тачпад пальцы",
        build: mobile::gestures,
    },
    PageDef {
        id: "vibration",
        title: "Вибрация",
        icon: icons::VIBRATION,
        group: "Устройства",
        keywords: "вибро вибромотор виброотклик тактильный отклик haptics сила клавиатура уведомления",
        build: mobile::vibration,
    },
    PageDef {
        id: "hardware",
        title: "Оборудование",
        icon: icons::HARDWARE,
        group: "Устройства",
        keywords: "процессор ядра частота память аккумулятор батарея датчики сенсоры температура диск usb камера звук видеокарта gpu",
        build: hardware::hardware,
    },
    PageDef {
        id: "power",
        title: "Питание",
        icon: icons::BATTERY,
        group: "Система",
        keywords: "батарея аккумулятор заряд регулятор частота governor сон гашение экрана",
        build: power::power,
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
