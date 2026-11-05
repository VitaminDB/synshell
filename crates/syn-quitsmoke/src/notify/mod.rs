//! Слой системных уведомлений. На desktop — no-op, на Android — реальная реализация.

use crate::app_state::AppState;

#[cfg(target_os = "android")]
mod android;

/// Создать каналы, запросить разрешение (после онбординга).
pub fn init(state: &AppState) {
    #[cfg(target_os = "android")]
    android::init(state);
    #[cfg(not(target_os = "android"))]
    let _ = state;
}

/// Синхронизировать уведомления с состоянием (после мутаций и каждую секунду).
pub fn sync(state: &AppState) {
    #[cfg(target_os = "android")]
    android::sync(state);
    #[cfg(not(target_os = "android"))]
    let _ = state;
}

/// Опросить действия из уведомлений (кнопки).
pub fn poll(state: &AppState) {
    #[cfg(target_os = "android")]
    android::poll(state);
    #[cfg(not(target_os = "android"))]
    let _ = state;
}

/// Убрать все уведомления (сброс прогресса).
pub fn clear() {
    #[cfg(target_os = "android")]
    android::clear();
}

/// Запросить разрешение на уведомления (из онбординга/настроек).
pub fn request_permission() {
    #[cfg(target_os = "android")]
    android::request_permission();
}
