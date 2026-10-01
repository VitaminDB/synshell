//! syndroid — Android-приложения в synshell: контейнер LineageOS (образы из OTA-каналов Waydroid) со своим
//! рантаймом на Rust. Демон `syndroidd` (root) — образы, контейнер, сеть; `syndroid` — управление.
//! Устройство и этапы — `docs/ANDROID.md`.

pub mod android;
pub mod api;
pub mod bridge;
pub mod config;
pub mod container;
pub mod daemon;
pub mod images;
pub mod net;
pub mod paths;
pub mod props;
pub mod sys;
