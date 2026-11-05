//! Где syndroid хранит своё. Всё состояние — в `/var/lib/syndroid`, временное — в `/run/syndroid`.

use std::path::PathBuf;

pub const STATE: &str = "/var/lib/syndroid";
pub const RUN: &str = "/run/syndroid";
/// Сокет демона (JSON построчно, см. [`crate::api`]).
pub const SOCKET: &str = "/run/syndroid/syndroidd.sock";
/// cgroup контейнера (v2): заморозка — `cgroup.freeze`.
pub const CGROUP: &str = "/sys/fs/cgroup/syndroid";
pub const BINDERFS: &str = "/dev/binderfs";
/// Своя программа для помощников (`__container`, `__exec`, `__bridge`): `/proc/self/exe` работает и после
/// замены файла на диске при обновлении (`current_exe()` тогда указывает на удалённый путь).
pub const SELF_EXE: &str = "/proc/self/exe";

/// Пути внутри контейнера (как у Waydroid — их ждут свойства и HAL образа).
pub const CONTAINER_XDG_RUNTIME_DIR: &str = "/run/xdg";
pub const CONTAINER_WAYLAND_DISPLAY: &str = "wayland-0";

fn state(p: &str) -> PathBuf {
    PathBuf::from(STATE).join(p)
}

pub fn config() -> PathBuf {
    state("config.toml")
}
/// Наборы образов: `images/<имя>/{system.img,vendor.img,info.toml}`.
pub fn images() -> PathBuf {
    state("images")
}
/// `/data` экземпляра Android (у каждого — свой: приложения LineageOS 18 и 20 не смешиваются).
pub fn data(instance: &str) -> PathBuf {
    state("data").join(instance)
}
/// Свои файлы поверх образов (нижний слой overlay): `overlay/{system,vendor}`.
pub fn overlay(part: &str) -> PathBuf {
    state("overlay").join(part)
}
/// Файлы платформы поверх образов (ставит сборка ОС устройства, например turnip под Android):
/// `/usr/share/syndroid/overlay/{system,vendor}` — средний слой overlay.
pub fn platform_overlay(part: &str) -> PathBuf {
    PathBuf::from("/usr/share/syndroid/overlay").join(part)
}
/// Свойства Android от платформы: `/etc/syndroid/*.prop` (поверх них — `[properties]` из config.toml).
pub const PLATFORM_PROPS: &str = "/etc/syndroid";
/// Изменения Android в system/vendor (верхний слой overlay), отдельно для каждого набора образов.
pub fn overlay_rw(set: &str, part: &str) -> PathBuf {
    state("overlay_rw").join(set).join(part)
}
pub fn overlay_work(set: &str, part: &str) -> PathBuf {
    state("overlay_work").join(set).join(part)
}
/// Точки монтирования голых образов (видны только в пространстве имён контейнера).
pub fn image_mnt(part: &str) -> PathBuf {
    state("mnt").join(part)
}
/// Корень контейнера (overlay).
pub fn rootfs() -> PathBuf {
    state("rootfs")
}
pub fn props() -> PathBuf {
    PathBuf::from(RUN).join("waydroid.prop")
}
pub fn container_log() -> PathBuf {
    PathBuf::from(RUN).join("container.log")
}
