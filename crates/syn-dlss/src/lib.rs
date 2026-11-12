//! Общая часть DLSS 5 synshell: общая память помощника (`shm`), пути и config.ini, обмен кадрами с
//! помощником без Vulkan-слоя (`video`) и фильтр кадров (`filter`) — для окна syn-dlss, видеоплеера и syn-vm.

pub mod config;
pub mod filter;
pub mod paths;
pub mod shm;
pub mod video;
