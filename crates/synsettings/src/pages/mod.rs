//! Страницы настроек и их реестр (навигация, поиск).

use crate::ui::{icons, W};
use syngui::n_;

mod appearance;
mod audio;
mod camera;
mod datetime;
mod default_apps;
mod devices;
mod hardware;
mod input;
mod language;
mod location;
mod mobile;
mod modem;
mod vpn;
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
        title: n_!("Темы"),
        icon: icons::THEMES,
        group: n_!("Оформление"),
        keywords: n_!("тема оформление стиль палитра nord catppuccin gruvbox dracula tokyo night synthwave неон стекло"),
        build: themes::themes,
    },
    PageDef {
        id: "appearance",
        title: n_!("Внешний вид"),
        icon: icons::PALETTE,
        group: n_!("Оформление"),
        keywords: n_!("тема схема тёмная светлая акцент цвет шрифт размер скругление прозрачность масштаб значки иконки курсор мышь скрыть курсор палитра"),
        build: appearance::appearance,
    },
    PageDef {
        id: "wallpaper",
        title: n_!("Обои"),
        icon: icons::WALLPAPER,
        group: n_!("Оформление"),
        keywords: n_!("фон картинка изображение фото слайд-шоу градиент цвет рабочий стол столы значки кадр масштаб панорама параллакс галерея каталог"),
        build: wallpaper::build,
    },
    PageDef {
        id: "panels",
        title: n_!("Панели"),
        icon: icons::PANEL,
        group: n_!("Оформление"),
        keywords: n_!("панель задач апплеты виджеты часы трей меню край размер автоскрытие плавающая"),
        build: panels::panels,
    },
    PageDef {
        id: "decorations",
        title: n_!("Оформление окон"),
        icon: icons::STYLE,
        group: n_!("Оформление"),
        keywords: n_!("заголовок рамка кнопки закрыть свернуть развернуть тень скругление цвет"),
        build: appearance::decorations,
    },
    PageDef {
        id: "animations",
        title: n_!("Анимации"),
        icon: icons::ANIMATION,
        group: n_!("Оформление"),
        keywords: n_!("эффекты скорость открытие закрытие сворачивание переключение столов"),
        build: appearance::animations,
    },
    PageDef {
        id: "windows",
        title: n_!("Поведение окон"),
        icon: icons::WINDOW,
        group: n_!("Рабочее пространство"),
        keywords: n_!("фокус размещение рамки модификатор зазоры прилипание snap плитка мозаика заголовок затемнение раскладка мастер"),
        build: windows::windows,
    },
    PageDef {
        id: "workspaces",
        title: n_!("Рабочие столы"),
        icon: icons::WORKSPACES,
        group: n_!("Рабочее пространство"),
        keywords: n_!("виртуальные столы количество имена раскладка"),
        build: windows::workspaces,
    },
    PageDef {
        id: "rules",
        title: n_!("Правила окон"),
        icon: icons::RULES,
        group: n_!("Рабочее пространство"),
        keywords: n_!("app_id заголовок плавающее стол размер положение поверх прозрачность"),
        build: windows::rules,
    },
    PageDef {
        id: "launcher",
        title: n_!("Меню запуска"),
        icon: icons::APPS,
        group: n_!("Рабочее пространство"),
        keywords: n_!("приложения избранное поиск калькулятор категории"),
        build: system::launcher,
    },
    PageDef {
        id: "notifications",
        title: n_!("Уведомления"),
        icon: icons::NOTIFICATIONS,
        group: n_!("Рабочее пространство"),
        keywords: n_!("всплывающие не беспокоить история время"),
        build: system::notifications,
    },
    PageDef {
        id: "keyboard",
        title: n_!("Клавиатура"),
        icon: icons::KEYBOARD,
        group: n_!("Устройства"),
        keywords: n_!("раскладка язык переключение автоповтор numlock xkb"),
        build: input::keyboard,
    },
    PageDef {
        id: "shortcuts",
        title: n_!("Комбинации клавиш"),
        icon: icons::SHORTCUTS,
        group: n_!("Устройства"),
        keywords: n_!("сочетания горячие клавиши действия hotkeys"),
        build: input::shortcuts,
    },
    PageDef {
        id: "mouse",
        title: n_!("Мышь и тачпад"),
        icon: icons::MOUSE,
        group: n_!("Устройства"),
        keywords: n_!("указатель ускорение прокрутка естественная касание тачпад левша"),
        build: input::mouse,
    },
    PageDef {
        id: "displays",
        title: n_!("Мониторы"),
        icon: icons::DISPLAY,
        group: n_!("Устройства"),
        keywords: n_!("экран разрешение частота масштаб поворот положение vrr основной яркость ниты подсветка"),
        build: system::displays,
    },
    PageDef {
        id: "wifi",
        title: "Wi-Fi",
        icon: icons::WIFI,
        group: n_!("Устройства"),
        keywords: n_!("беспроводная сеть интернет пароль подключение iwd networkmanager точка доступа"),
        build: net::wifi,
    },
    PageDef {
        id: "audio",
        title: n_!("Звук"),
        icon: "\u{e050}",
        group: n_!("Устройства"),
        keywords: n_!("громкость динамик наушники гарнитура микрофон вывод ввод устройство программы dolby atmos профиль музыка кино pipewire"),
        build: audio::audio,
    },
    PageDef {
        id: "bluetooth",
        title: "Bluetooth",
        icon: icons::BLUETOOTH,
        group: n_!("Устройства"),
        keywords: n_!("наушники колонка сопряжение устройство клавиатура мышь bluez"),
        build: net::bluetooth,
    },
    PageDef {
        id: "mobile-network",
        title: n_!("Мобильная сеть"),
        icon: "\u{E1C8}",
        group: n_!("Устройства"),
        keywords: n_!("сотовая связь sim сим-карта esim есим профиль оператор роуминг мобильный интернет передача данных apn точка доступа 5g 4g lte 3g 2g поиск сети регистрация pin пин puk трафик режим полёта"),
        build: modem::mobile_network,
    },
    PageDef {
        id: "calls-sms",
        title: n_!("Вызовы и SMS"),
        icon: "\u{E0B0}",
        group: n_!("Устройства"),
        keywords: n_!("звонки ожидание вызова переадресация скрыть номер аон clir ussd баланс sms смс центр smsc"),
        build: modem::calls_sms,
    },
    PageDef {
        id: "modem-info",
        title: n_!("О модеме"),
        icon: "\u{E88E}",
        group: n_!("Устройства"),
        keywords: n_!("imei imsi iccid eid прошивка модема baseband диапазоны band сота rsrp rsrq sinr earfcn"),
        build: modem::modem_info,
    },
    PageDef {
        id: "vpn",
        title: "VPN",
        icon: "\u{E897}",
        group: n_!("Устройства"),
        keywords: n_!("впн vpn wireguard openvpn туннель частная сеть networkmanager импорт ключ"),
        build: vpn::vpn_page,
    },
    PageDef {
        id: "devices",
        title: n_!("Связь с устройствами"),
        icon: "\u{e326}",
        group: n_!("Устройства"),
        keywords: n_!("телефон компьютер usb wi-fi synlink спаривание экран трансляция файлы уведомления ssh отладка mcp claude"),
        build: devices::devices,
    },
    PageDef {
        id: "phone",
        title: n_!("Телефон"),
        icon: icons::PHONE,
        group: n_!("Устройства"),
        keywords: n_!("режим окон страницы свободный стол ручка назад домашний экран ресурсы мобильный"),
        build: mobile::phone,
    },
    PageDef {
        id: "gestures",
        title: n_!("Жесты"),
        icon: icons::GESTURE,
        group: n_!("Устройства"),
        keywords: n_!("свайп край назад домой шторка недавние удержание тачпад пальцы"),
        build: mobile::gestures,
    },
    PageDef {
        id: "vibration",
        title: n_!("Вибрация"),
        icon: icons::VIBRATION,
        group: n_!("Устройства"),
        keywords: n_!("вибро вибромотор виброотклик тактильный отклик haptics сила клавиатура уведомления"),
        build: mobile::vibration,
    },
    PageDef {
        id: "hardware",
        title: n_!("Оборудование"),
        icon: icons::HARDWARE,
        group: n_!("Устройства"),
        keywords: n_!("процессор ядра частота память аккумулятор батарея датчики сенсоры температура диск usb камера звук видеокарта gpu"),
        build: hardware::hardware,
    },
    PageDef {
        id: "power",
        title: n_!("Питание"),
        icon: icons::BATTERY,
        group: n_!("Система"),
        keywords: n_!("батарея аккумулятор заряд регулятор частота governor сон гашение экрана"),
        build: power::power,
    },
    PageDef {
        id: "location",
        title: n_!("Местоположение"),
        icon: "\u{e0c8}",
        group: n_!("Система"),
        keywords: n_!("местоположение геолокация gps gnss спутники координаты geoclue карты навигация разрешения конфиденциальность"),
        build: location::location,
    },
    PageDef {
        id: "camera",
        title: n_!("Камера"),
        icon: "\u{e04b}",
        group: n_!("Система"),
        keywords: n_!("камера веб-камера видео видеозвонок портал разрешения конфиденциальность браузер pipewire"),
        build: camera::camera,
    },
    PageDef {
        id: "language",
        title: n_!("Язык"),
        icon: "\u{e894}",
        group: n_!("Система"),
        keywords: n_!("язык интерфейса перевод локаль английский русский language locale english"),
        build: language::language,
    },
    PageDef {
        id: "datetime",
        title: n_!("Дата и время"),
        icon: icons::SCHEDULE,
        group: n_!("Система"),
        keywords: n_!("часы время дата часовой пояс таймзона timezone ntp синхронизация автоматически utc"),
        build: datetime::datetime,
    },
    PageDef {
        id: "lock",
        title: n_!("Блокировка и простой"),
        icon: icons::LOCK,
        group: n_!("Система"),
        keywords: n_!("экран блокировки сон простой dpms гашение"),
        build: system::lock,
    },
    PageDef {
        id: "autostart",
        title: n_!("Автозапуск"),
        icon: icons::AUTOSTART,
        group: n_!("Система"),
        keywords: n_!("запуск при входе программы xdg autostart"),
        build: system::autostart,
    },
    PageDef {
        id: "default-apps",
        title: n_!("Программы по умолчанию"),
        icon: "\u{e89f}",
        group: n_!("Система"),
        keywords: n_!("ассоциации типы файлов открывать с помощью браузер почта терминал проводник файловый менеджер видео музыка плеер изображения картинки текст редактор pdf архивы mime mimeapps"),
        build: default_apps::default_apps,
    },
    PageDef {
        id: "general",
        title: n_!("Сеанс"),
        icon: icons::SETTINGS,
        group: n_!("Система"),
        keywords: n_!("оболочка переменные окружения xwayland x11 разрешение wine игры мелкий интерфейс снимки"),
        build: system::general,
    },
    PageDef {
        id: "about",
        title: n_!("О системе"),
        icon: icons::INFO,
        group: n_!("Система"),
        keywords: n_!("версия ядро процессор память видеокарта"),
        build: system::about,
    },
];

pub fn find(id: &str) -> &'static PageDef {
    PAGES.iter().find(|p| p.id == id).unwrap_or(&PAGES[0])
}

/// Страница подходит под строку поиска (все слова запроса встречаются
/// в названии или ключевых словах).
pub fn matches(p: &PageDef, query: &str) -> bool {
    // Исходные слова и перевод — ищется на любом из двух языков.
    let tl = crate::ui::tl;
    let hay = format!("{} {} {} {} {} {}", p.title, p.keywords, p.group, tl(p.title), tl(p.keywords), tl(p.group)).to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}
