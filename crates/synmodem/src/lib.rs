//! Модем телефона: свой демон `synmodemd` (root) говорит с модемом Qualcomm по QMI поверх QRTR и отдаёт
//! оболочке состояние сети, SIM, SMS и звонки через unix-сокет [`api`].

pub mod pdu;
pub mod qmi;
pub mod qrtr;
pub mod api;
pub mod daemon;
pub mod store;
pub mod time;
