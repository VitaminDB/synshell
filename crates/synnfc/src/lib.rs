//! NFC телефона без NCI-ядра Linux: свой стек NCI поверх символьного устройства драйвера
//! контроллера (NXP nfc_i2c `/dev/nq-nci` и подобные, synmobile docs/25) — чтение меток
//! (NTAG/Ultralight, Type 4, идентификаторы FeliCa, ISO 15693, MIFARE Classic), запись
//! NDEF, эмуляция метки Type 4 (HCE). Демон `synnfcd` (root) и клиент ([`api`]).

pub mod api;
pub mod daemon;
pub mod emu;
pub mod nci;
pub mod ndef;
pub mod tag;
