//! Двойная загрузка Android + Linux (synmobile): можно ли перезагрузиться в Android. Одна проверка для экрана входа
//! и меню питания оболочки — на всех устройствах одинаково:
//! - меню в boot.img (Xiaomi): каталог synmobile на /data Android;
//! - Linux в recovery с выбором через BCB в misc (Samsung с lk2nd, BOOX): `/etc/synmobile/boot-bcb`.
//! Сама перезагрузка — `syn-boot android -r` (способ устройства); из сеанса — служба `syn-reboot-android.service`
//! (правило polkit synmobile для wheel).

pub fn android_boot_available() -> bool {
    std::path::Path::new("/android/data/local/tmp/syn").is_dir() || std::path::Path::new("/etc/synmobile/boot-bcb").is_file()
}

/// Команда для сеанса пользователя (без root).
pub const REBOOT_ANDROID_CMD: &str = "systemctl start syn-reboot-android.service";
