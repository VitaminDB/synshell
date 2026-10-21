//! Чтение архивов через libarchive (та же библиотека, что у `bsdtar` и
//! pacman): zip, 7z, rar, tar.*, iso, cpio, cab, одиночные .gz/.xz/.zst…

use std::ffi::{c_char, c_int, c_long, c_void, CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[allow(non_camel_case_types)]
type la_ssize_t = isize;

#[link(name = "archive")]
unsafe extern "C" {
    fn archive_read_new() -> *mut c_void;
    fn archive_read_support_filter_all(a: *mut c_void) -> c_int;
    fn archive_read_support_format_all(a: *mut c_void) -> c_int;
    fn archive_read_support_format_raw(a: *mut c_void) -> c_int;
    fn archive_read_set_options(a: *mut c_void, opts: *const c_char) -> c_int;
    fn archive_read_open_filename(a: *mut c_void, name: *const c_char, block: usize) -> c_int;
    fn archive_read_next_header(a: *mut c_void, entry: *mut *mut c_void) -> c_int;
    fn archive_read_data(a: *mut c_void, buf: *mut c_void, len: usize) -> la_ssize_t;
    fn archive_read_free(a: *mut c_void) -> c_int;
    fn archive_error_string(a: *mut c_void) -> *const c_char;
    fn archive_format(a: *mut c_void) -> c_int;
    fn archive_entry_pathname(e: *mut c_void) -> *const c_char;
    fn archive_entry_pathname_utf8(e: *mut c_void) -> *const c_char;
    fn archive_entry_symlink(e: *mut c_void) -> *const c_char;
    fn archive_entry_hardlink(e: *mut c_void) -> *const c_char;
    fn archive_entry_filetype(e: *mut c_void) -> u32;
    fn archive_entry_perm(e: *mut c_void) -> u32;
    fn archive_entry_size(e: *mut c_void) -> i64;
    fn archive_entry_size_is_set(e: *mut c_void) -> c_int;
    fn archive_entry_mtime(e: *mut c_void) -> c_long;
    fn archive_entry_is_encrypted(e: *mut c_void) -> c_int;
}

const OK: c_int = 0;
const EOF: c_int = 1;
const WARN: c_int = -20;
/// `ARCHIVE_FORMAT_RAW` — не архив, а один сжатый файл.
const FORMAT_RAW: c_int = 0x90000;

pub const IFMT: u32 = 0o170000;
pub const IFDIR: u32 = 0o040000;
pub const IFLNK: u32 = 0o120000;
pub const IFREG: u32 = 0o100000;

/// Запись архива (заголовок).
#[derive(Debug, Clone)]
pub struct Header {
    /// Путь внутри архива (UTF-8).
    pub path: String,
    pub kind: u32,
    pub perm: u32,
    pub size: Option<u64>,
    pub mtime: i64,
    pub symlink: Option<String>,
    pub hardlink: Option<String>,
    pub encrypted: bool,
}

pub struct Reader {
    a: *mut c_void,
    entry: *mut c_void,
    /// Номер прочитанного заголовка (с 0); `usize::MAX` — ещё ни одного.
    pub index: usize,
}

// Читатель используется одним потоком за раз (под замком).
unsafe impl Send for Reader {}

impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { archive_read_free(self.a) };
    }
}

impl Reader {
    pub fn open(path: &Path) -> Result<Reader, String> {
        // В локали «C» libarchive не перекодирует имена (zip с флагом UTF-8
        // давал пустые русские имена) — читать в UTF-8.
        static LOCALE: std::sync::Once = std::sync::Once::new();
        LOCALE.call_once(|| unsafe {
            if libc::setlocale(libc::LC_CTYPE, c"C.UTF-8".as_ptr()).is_null() {
                libc::setlocale(libc::LC_CTYPE, c"en_US.UTF-8".as_ptr());
            }
        });
        let c = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        unsafe {
            let a = archive_read_new();
            if a.is_null() {
                return Err("archive_read_new".into());
            }
            archive_read_support_filter_all(a);
            archive_read_support_format_all(a);
            // Одиночный сжатый файл (`отчёт.txt.gz`) — архив из одного файла.
            archive_read_support_format_raw(a);
            // Имена в zip без флага UTF-8 — в кодировке OEM той Windows, где
            // архив сделан; у кириллических — CP866, а не CP437.
            if cyrillic_locale() {
                archive_read_set_options(a, c"zip:hdrcharset=CP866".as_ptr());
            }
            let r = Reader { a, entry: std::ptr::null_mut(), index: usize::MAX };
            if archive_read_open_filename(a, c.as_ptr(), 1 << 16) != OK {
                return Err(r.error());
            }
            Ok(r)
        }
    }

    fn error(&self) -> String {
        unsafe {
            let s = archive_error_string(self.a);
            if s.is_null() {
                "libarchive".into()
            } else {
                CStr::from_ptr(s).to_string_lossy().into_owned()
            }
        }
    }

    /// Один сжатый файл, а не архив (известно после первого заголовка).
    pub fn is_raw(&self) -> bool {
        unsafe { archive_format(self.a) == FORMAT_RAW }
    }

    /// Следующий заголовок; `None` — конец архива.
    pub fn next(&mut self) -> Result<Option<Header>, String> {
        let r = unsafe { archive_read_next_header(self.a, &mut self.entry) };
        match r {
            EOF => return Ok(None),
            OK | WARN => {}
            _ => return Err(self.error()),
        }
        self.index = self.index.wrapping_add(1);
        let e = self.entry;
        unsafe {
            let utf8 = archive_entry_pathname_utf8(e);
            let path = if !utf8.is_null() { CStr::from_ptr(utf8).to_bytes().to_vec() } else { cbytes(archive_entry_pathname(e)).unwrap_or_default() };
            let size = (archive_entry_size_is_set(e) != 0).then(|| archive_entry_size(e).max(0) as u64);
            Ok(Some(Header {
                path: decode_name(&path),
                kind: archive_entry_filetype(e) & IFMT,
                perm: archive_entry_perm(e) & 0o7777,
                size,
                mtime: archive_entry_mtime(e) as i64,
                symlink: cbytes(archive_entry_symlink(e)).map(|b| decode_name(&b)),
                hardlink: cbytes(archive_entry_hardlink(e)).map(|b| decode_name(&b)),
                encrypted: archive_entry_is_encrypted(e) != 0,
            }))
        }
    }

    /// Прочитать данные текущей записи; 0 — конец.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let n = unsafe { archive_read_data(self.a, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(self.error());
        }
        Ok(n as usize)
    }
}

fn cyrillic_locale() -> bool {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty())).unwrap_or_default();
    ["ru", "uk", "be", "kk", "bg", "sr", "mk"].iter().any(|p| lang.starts_with(p))
}

unsafe fn cbytes(p: *const c_char) -> Option<Vec<u8>> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_bytes().to_vec())
    }
}

/// Имя из архива: UTF-8 или, если это не UTF-8, — CP866 (zip из Windows
/// с русскими именами без флага UTF-8).
pub fn decode_name(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    b.iter().map(|&c| if c < 0x80 { c as char } else { CP866[(c - 0x80) as usize] }).collect()
}

const CP866: [char; 128] = [
    'А', 'Б', 'В', 'Г', 'Д', 'Е', 'Ж', 'З', 'И', 'Й', 'К', 'Л', 'М', 'Н', 'О', 'П', 'Р', 'С', 'Т', 'У', 'Ф', 'Х', 'Ц', 'Ч', 'Ш', 'Щ', 'Ъ', 'Ы',
    'Ь', 'Э', 'Ю', 'Я', 'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'й', 'к', 'л', 'м', 'н', 'о', 'п', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖',
    '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙',
    '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'р', 'с', 'т', 'у', 'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'ъ', 'ы', 'ь', 'э', 'ю', 'я',
    'Ё', 'ё', 'Є', 'є', 'Ї', 'ї', 'Ў', 'ў', '°', '∙', '·', '√', '№', '¤', '■', '\u{a0}',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp866_names() {
        assert_eq!(decode_name(&[0x8f, 0xe0, 0xa8, 0xa2, 0xa5, 0xe2, b'.', b't', b'x', b't']), "Привет.txt");
        assert_eq!(decode_name("уже utf8".as_bytes()), "уже utf8");
    }
}
