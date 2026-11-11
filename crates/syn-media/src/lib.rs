//! Камера и запись видео для программ synshell.
//!
//! - [`capture`] — источник кадров: служба syncamd телефона ([`syncamd`]), сырой
//!   Bayer через media controller с программным ISP ([`mcam`], [`isp`]; mainline-ядра) или
//!   камера V4L2 компьютера ([`v4l2`]); [`capture::cameras`] и
//!   [`capture::front_camera`] находят камеры той машины, где запущена программа.
//! - [`engine::Engine`] — сеанс камеры в фоне: превью для `syngui::widgets::LiveView`
//!   (`LiveFrame`, RGBA с поворотом и зеркалом), управление, запись.
//! - [`recorder::Recorder`] — запись MP4: видео — аппаратный кодер V4L2 M2M
//!   телефона ([`encoder`], `venc.c`) или ffmpeg компьютера (NVENC, VAAPI,
//!   libx264); звук — микрофон и внешние дорожки ([`audio::Track`], голос
//!   собеседника) смешиваются в AAC; [`recorder::Recorder::position`] — время
//!   записи для синхронизации транскрипта.
//! - [`media::Library`] — где лежат снимки и видео программы (каталог, префикс имён, миниатюры).
//! - [`convert`] — NV12/NV21/YUYV/RGB: превью и вход кодера.

pub mod audio;
pub mod capture;
pub mod convert;
pub mod encoder;
pub mod engine;
pub mod isp;
pub mod mcam;
pub mod media;
mod mux;
pub mod recorder;
pub mod syncamd;
pub mod v4l2;

/// Каталог переводов строк библиотеки (сообщения камеры и записи); его
/// подключают программы, которые их показывают: `i18n::init(&[…, syn_media::CATALOG])`.
pub const CATALOGS: &[&str] = &synshell_tr::catalogs!("../i18n");

pub use capture::{cameras, front_camera, Cam};
pub use engine::{Engine, Note, OpenSpec};
pub use recorder::{Recorder, Settings};
