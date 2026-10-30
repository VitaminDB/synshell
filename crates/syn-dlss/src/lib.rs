//! Общая часть DLSS 5 synshell: общая память помощника (`shm`), пути и config.ini, обмен кадрами видео
//! с помощником без Vulkan-слоя (`video`) — для окна syn-dlss и видеоплеера.

pub mod config;
pub mod paths;
pub mod shm;
pub mod video;
