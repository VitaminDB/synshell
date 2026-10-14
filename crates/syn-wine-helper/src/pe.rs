//! Что за PE-файл: архитектура, имя в таблице экспорта, импортируемые DLL.

use object::pe;
use object::read::pe::{ImageNtHeaders, PeFile};
use object::LittleEndian as LE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub const ALL: [Arch; 2] = [Arch::X86, Arch::X64];

    pub fn from_machine(m: u16) -> Option<Arch> {
        match m {
            pe::IMAGE_FILE_MACHINE_I386 => Some(Arch::X86),
            pe::IMAGE_FILE_MACHINE_AMD64 => Some(Arch::X64),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
        }
    }

    pub fn parse(s: &str) -> Option<Arch> {
        match s {
            "x86" | "i386" | "win32" | "32" => Some(Arch::X86),
            "x64" | "amd64" | "x86_64" | "win64" | "64" => Some(Arch::X64),
            _ => None,
        }
    }

    /// Каталог в общем хранилище и в Wine (`<arch>-windows`).
    pub fn dir(self) -> &'static str {
        match self {
            Arch::X86 => "i386-windows",
            Arch::X64 => "x86_64-windows",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeInfo {
    pub machine: u16,
    /// Имя DLL из таблицы экспорта (как её собрали), строчными.
    pub export_name: Option<String>,
    /// Обязательные импорты (без них загрузчик не запустит модуль), строчными.
    pub imports: Vec<String>,
    pub is_dll: bool,
}

impl PeInfo {
    pub fn arch(&self) -> Option<Arch> {
        Arch::from_machine(self.machine)
    }
}

/// Быстрая проверка заголовка без разбора: MZ и PE\0\0.
pub fn looks_like_pe(data: &[u8]) -> bool {
    if data.len() < 0x40 || &data[..2] != b"MZ" {
        return false;
    }
    let off = u32::from_le_bytes([data[0x3c], data[0x3d], data[0x3e], data[0x3f]]) as usize;
    data.len() > off + 4 && &data[off..off + 4] == b"PE\0\0"
}

pub fn parse(data: &[u8]) -> Option<PeInfo> {
    if !looks_like_pe(data) {
        return None;
    }
    parse_as::<pe::ImageNtHeaders32>(data).or_else(|| parse_as::<pe::ImageNtHeaders64>(data))
}

fn parse_as<Pe: ImageNtHeaders>(data: &[u8]) -> Option<PeInfo> {
    let file = PeFile::<Pe>::parse(data).ok()?;
    let fh = file.nt_headers().file_header();
    let machine = fh.machine.get(LE);
    let is_dll = fh.characteristics.get(LE) & pe::IMAGE_FILE_DLL != 0;
    let export_name = file.export_table().ok().flatten().and_then(|t| {
        let rva = t.directory().name.get(LE);
        t.name_from_pointer(rva).ok().map(|n| String::from_utf8_lossy(n).to_ascii_lowercase())
    });
    let mut imports = Vec::new();
    if let Ok(Some(table)) = file.import_table() {
        if let Ok(mut it) = table.descriptors() {
            while let Ok(Some(d)) = it.next() {
                if let Ok(n) = table.name(d.name.get(LE)) {
                    let n = String::from_utf8_lossy(n).to_ascii_lowercase();
                    if !imports.contains(&n) {
                        imports.push(n);
                    }
                }
            }
        }
    }
    Some(PeInfo { machine, export_name, imports, is_dll })
}

