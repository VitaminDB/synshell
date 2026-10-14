//! Схема нод: порты, поля, описание типа ноды.

/// Тип данных порта.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortType {
    Text,
    Number,
    Image,
    Audio,
    Video,
    /// Любой медиафайл (картинка, звук, ролик).
    Media,
    /// Список значений (пакет): порт принимает его целиком, без раскрытия.
    List,
    Any,
    /// Загруженная модель (дескриптор службы).
    Model,
    /// Условие: закодированное описание (и референсы).
    Cond,
    /// Латент: результат сэмплера или закодированная картинка.
    Latent,
}

impl PortType {
    pub fn is_media(self) -> bool {
        matches!(self, PortType::Image | PortType::Audio | PortType::Video | PortType::Media)
    }

    /// Дескриптор объекта службы — не покидает процесс, исполняющий граф.
    pub fn is_handle(self) -> bool {
        matches!(self, PortType::Model | PortType::Cond | PortType::Latent)
    }

    /// Можно ли соединить выход `self` со входом `to`. Медиатипы
    /// совместимы между собой (проверка формата — при исполнении), список
    /// идёт в одиночный вход пакетом (нода выполняется для каждого элемента).
    pub fn connects_to(self, to: PortType) -> bool {
        use PortType::*;
        match (self, to) {
            (_, Any) | (Any, _) => true,
            (a, b) if a == b => true,
            (a, b) if a.is_handle() || b.is_handle() => false,
            (List, _) | (_, List) => true,
            (a, b) if a.is_media() && b.is_media() => true,
            (Number, Text) | (Text, Number) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PortSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub ty: PortType,
}

pub const fn port(name: &'static str, label: &'static str, ty: PortType) -> PortSpec {
    PortSpec { name, label, ty }
}

/// Тип поля ноды.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldType {
    Text,
    /// Многострочный текст (описание, стихи).
    LongText,
    Number,
    Bool,
    /// Выбор из списка; значение — строка-вариант.
    Choice(&'static [&'static str]),
    /// Путь к файлу.
    File,
}

#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub ty: FieldType,
    /// Значение по умолчанию строкой (`""` — пусто); число — `"24"`, флаг — `"true"`.
    pub default: &'static str,
}

pub const fn field(name: &'static str, label: &'static str, ty: FieldType, default: &'static str) -> FieldSpec {
    FieldSpec { name, label, ty, default }
}

/// Описание типа ноды. Поле и входной порт с одним именем — одно значение:
/// подключённый порт перекрывает поле.
#[derive(Debug)]
pub struct NodeSpec {
    pub kind: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// Раздел палитры: «Генерация», «Монтаж», «Данные».
    pub category: &'static str,
    pub inputs: &'static [PortSpec],
    pub outputs: &'static [PortSpec],
    pub fields: &'static [FieldSpec],
    /// Стадия модели: исполняется только службой, результат (дескриптор)
    /// кэшируется — повторный прогон с теми же входами берёт его из кэша.
    pub stage: bool,
}

impl NodeSpec {
    pub fn input(&self, name: &str) -> Option<&PortSpec> {
        self.inputs.iter().find(|p| p.name == name)
    }
    pub fn output(&self, name: &str) -> Option<&PortSpec> {
        self.outputs.iter().find(|p| p.name == name)
    }
    pub fn field(&self, name: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|f| f.name == name)
    }
    /// Главный выход (первый) — результат ноды в отчёте.
    pub fn main_output(&self) -> Option<&PortSpec> {
        self.outputs.first()
    }
}
