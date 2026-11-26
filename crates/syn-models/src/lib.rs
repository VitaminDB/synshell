//! syn-models — «Модели» synshell: поиск и загрузка моделей с Hugging Face,
//! библиотека пакетов `.syn` и перепаковка в них (квантование — в службе
//! нейросетей), обучение адаптеров LoRA (в службе). Перенесено из synthos.

pub mod app;
pub mod components;
pub mod config;
pub mod context;
pub mod fsutil;
pub mod i18n;
pub mod icons;
pub mod pages;
pub mod paths;
pub mod time;
