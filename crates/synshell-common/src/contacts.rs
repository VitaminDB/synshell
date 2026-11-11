//! Контакты synshell — общие для «Телефона», SMS и оболочки (входящий звонок, экран блокировки, уведомления).
//!
//! Хранилище: `$XDG_DATA_HOME/synshell/contacts/<uid>.vcf` (vCard 3.0, по файлу на контакт — правка и синхронизация
//! без общей базы), фото — рядом, `<uid>.jpg`. Импорт понимает несколько карточек в одном файле и встроенное фото
//! (`PHOTO;ENCODING=b`), экспорт встраивает фото обратно — файл можно отдать любой адресной книге.
//!
//! Поиск по номеру сравнивает цифры: полностью или по последним 10 (`+7 701 …` = `8 701 …`).

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Phone {
    pub number: String,
    /// Тип из vCard: `CELL`, `HOME`, `WORK`… (пусто — без типа).
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub uid: String,
    pub name: String,
    pub phones: Vec<Phone>,
    pub emails: Vec<String>,
    pub note: String,
    pub favorite: bool,
}

impl Contact {
    pub fn new(name: &str, number: &str) -> Contact {
        let mut c = Contact { uid: new_uid(), name: name.trim().to_string(), ..Default::default() };
        if !number.trim().is_empty() {
            c.phones.push(Phone { number: number.trim().to_string(), kind: "CELL".into() });
        }
        c
    }

    /// Фото (если есть).
    pub fn photo(&self) -> Option<PathBuf> {
        let p = photo_path(&self.uid);
        p.is_file().then_some(p)
    }

    /// Первая буква имени (для кружка без фото).
    pub fn initial(&self) -> String {
        self.name.trim().chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
    }

    /// Показываемое имя: имя или первый номер.
    pub fn title(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.trim().to_string()
        } else {
            self.phones.first().map(|p| p.number.clone()).unwrap_or_default()
        }
    }

    pub fn has_number(&self, number: &str) -> bool {
        self.phones.iter().any(|p| same_number(&p.number, number))
    }
}

/// Каталог контактов (создаётся при записи).
pub fn dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SYNSHELL_CONTACTS_DIR") {
        return PathBuf::from(p);
    }
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".local/share"));
    data.join("synshell/contacts")
}

pub fn photo_path(uid: &str) -> PathBuf {
    dir().join(format!("{}.jpg", safe(uid)))
}

fn card_path(uid: &str) -> PathBuf {
    dir().join(format!("{}.vcf", safe(uid)))
}

fn safe(uid: &str) -> String {
    uid.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn new_uid() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // SAFETY: getpid без побочных эффектов
    let pid = unsafe { libc::getpid() } as u128;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128;
    format!("syn-{:x}", t ^ (pid << 96) ^ (n << 80))
}

/// Все контакты, по имени.
pub fn list() -> Vec<Contact> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir()) else { return v };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("vcf") {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Some(mut c) = parse(&s).into_iter().next() {
                // имя файла — главный uid (в карточке мог быть другой)
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    c.0.uid = stem.to_string();
                }
                v.push(c.0);
            }
        }
    }
    v.sort_by(|a, b| a.title().to_lowercase().cmp(&b.title().to_lowercase()));
    v
}

pub fn load(uid: &str) -> Option<Contact> {
    let s = std::fs::read_to_string(card_path(uid)).ok()?;
    let mut c = parse(&s).into_iter().next()?.0;
    c.uid = safe(uid);
    Some(c)
}

/// Записать контакт (атомарно; пустой uid — новый).
pub fn save(c: &mut Contact) -> io::Result<()> {
    if c.uid.is_empty() {
        c.uid = new_uid();
    }
    c.uid = safe(&c.uid);
    std::fs::create_dir_all(dir())?;
    let p = card_path(&c.uid);
    let tmp = p.with_extension("vcf.tmp");
    std::fs::write(&tmp, to_vcard(c, None))?;
    std::fs::rename(tmp, p)
}

pub fn delete(uid: &str) -> io::Result<()> {
    let _ = std::fs::remove_file(photo_path(uid));
    std::fs::remove_file(card_path(uid))
}

/// Фото контакта: JPEG (уже уменьшенный вызывающим) или `None` — убрать.
pub fn set_photo(uid: &str, jpeg: Option<&[u8]>) -> io::Result<()> {
    let p = photo_path(uid);
    match jpeg {
        Some(d) => {
            std::fs::create_dir_all(dir())?;
            std::fs::write(p, d)
        }
        None => match std::fs::remove_file(p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Только цифры номера (и ведущий `+` отбрасывается).
pub fn digits(number: &str) -> String {
    number.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Один ли это номер: цифры совпадают целиком или по последним 10 (не короче 7 цифр).
pub fn same_number(a: &str, b: &str) -> bool {
    let (a, b) = (digits(a), digits(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let n = a.len().min(b.len()).min(10);
    n >= 7 && a[a.len() - n..] == b[b.len() - n..]
}

/// Контакт с этим номером.
pub fn find_by_number(number: &str) -> Option<Contact> {
    if digits(number).len() < 3 {
        return None;
    }
    list().into_iter().find(|c| c.has_number(number))
}

/// Имя для номера (или сам номер).
pub fn name_for(number: &str) -> String {
    find_by_number(number).map(|c| c.title()).unwrap_or_else(|| number.to_string())
}

// ─── vCard ──────────────────────────────────────────────────────────────────

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Строки по 75 октетов с продолжением пробелом (RFC 6350 §3.2).
fn fold(line: &str, out: &mut String) {
    let mut n = 0;
    for ch in line.chars() {
        let l = ch.len_utf8();
        if n + l > 75 {
            out.push_str("\r\n ");
            n = 1;
        }
        out.push(ch);
        n += l;
    }
    out.push_str("\r\n");
}

/// Карточка vCard 3.0; `photo` — встроить JPEG (экспорт).
pub fn to_vcard(c: &Contact, photo: Option<&[u8]>) -> String {
    let mut s = String::new();
    fold("BEGIN:VCARD", &mut s);
    fold("VERSION:3.0", &mut s);
    fold(&format!("UID:{}", esc(&c.uid)), &mut s);
    fold(&format!("FN:{}", esc(&c.name)), &mut s);
    fold(&format!("N:{};;;;", esc(&c.name)), &mut s);
    for p in &c.phones {
        let t = if p.kind.is_empty() { String::new() } else { format!(";TYPE={}", p.kind) };
        fold(&format!("TEL{t}:{}", esc(&p.number)), &mut s);
    }
    for e in &c.emails {
        fold(&format!("EMAIL:{}", esc(e)), &mut s);
    }
    if !c.note.is_empty() {
        fold(&format!("NOTE:{}", esc(&c.note)), &mut s);
    }
    if c.favorite {
        fold("X-SYN-FAVORITE:1", &mut s);
    }
    if let Some(d) = photo {
        fold(&format!("PHOTO;ENCODING=b;TYPE=JPEG:{}", b64_encode(d)), &mut s);
    }
    fold("END:VCARD", &mut s);
    s
}

/// Разобрать карточки (одну или несколько); к каждой — встроенное фото, если было.
pub fn parse(text: &str) -> Vec<(Contact, Option<Vec<u8>>)> {
    // развернуть продолжения строк
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut out = Vec::new();
    let mut cur: Option<(Contact, Option<Vec<u8>>, String)> = None;
    for l in lines {
        let Some((head, value)) = l.split_once(':') else { continue };
        let mut parts = head.split(';');
        let prop = parts.next().unwrap_or("").to_ascii_uppercase();
        // группа «item1.TEL»
        let prop = prop.rsplit('.').next().unwrap_or("").to_string();
        let params: Vec<String> = parts.map(|p| p.to_ascii_uppercase()).collect();
        match prop.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => cur = Some((Contact::default(), None, String::new())),
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                if let Some((mut c, ph, n)) = cur.take() {
                    if c.name.trim().is_empty() {
                        c.name = n;
                    }
                    if c.uid.is_empty() {
                        c.uid = new_uid();
                    }
                    c.uid = safe(&c.uid);
                    out.push((c, ph));
                }
            }
            _ => {
                let Some((c, ph, n)) = cur.as_mut() else { continue };
                match prop.as_str() {
                    "UID" => c.uid = unesc(value),
                    "FN" => c.name = unesc(value),
                    "N" => {
                        // Фамилия;Имя;Отчество;… → «Имя Отчество Фамилия»
                        let f: Vec<String> = value.split(';').map(unesc).collect();
                        let order = [1usize, 2, 0];
                        *n = order.iter().filter_map(|&i| f.get(i)).filter(|s| !s.trim().is_empty()).cloned().collect::<Vec<_>>().join(" ");
                    }
                    "TEL" => {
                        let kind = params
                            .iter()
                            .flat_map(|p| p.trim_start_matches("TYPE=").split(','))
                            .map(|t| t.trim().to_string())
                            .find(|t| ["CELL", "HOME", "WORK", "MAIN", "FAX", "PAGER", "VOICE"].contains(&t.as_str()) && t != "VOICE")
                            .unwrap_or_default();
                        let number = unesc(value).trim_start_matches("tel:").to_string();
                        if !number.trim().is_empty() {
                            c.phones.push(Phone { number, kind });
                        }
                    }
                    "EMAIL" => c.emails.push(unesc(value)),
                    "NOTE" => c.note = unesc(value),
                    "X-SYN-FAVORITE" => c.favorite = value.trim() == "1",
                    "PHOTO" => {
                        // 3.0: ENCODING=b; 4.0: data:image/jpeg;base64,…
                        let data = value.split_once("base64,").map(|x| x.1).unwrap_or(value);
                        if params.iter().any(|p| p.contains("ENCODING=B") || p.contains("ENCODING=BASE64")) || value.starts_with("data:") {
                            *ph = b64_decode(data);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// Импорт файла .vcf: новые контакты (одинаковые по имени и номерам пропускаются); ответ — сколько добавлено.
pub fn import(text: &str) -> io::Result<usize> {
    let have = list();
    let mut n = 0;
    for (mut c, photo) in parse(text) {
        if have.iter().any(|h| h.name == c.name && h.phones.iter().map(|p| digits(&p.number)).eq(c.phones.iter().map(|p| digits(&p.number)))) {
            continue;
        }
        if have.iter().any(|h| h.uid == c.uid) {
            c.uid = new_uid();
        }
        save(&mut c)?;
        if let Some(d) = photo {
            set_photo(&c.uid, Some(&d))?;
        }
        n += 1;
    }
    Ok(n)
}

/// Экспорт всех контактов в один .vcf (с фото).
pub fn export(path: &Path) -> io::Result<usize> {
    let all = list();
    let mut s = String::new();
    for c in &all {
        let photo = c.photo().and_then(|p| std::fs::read(p).ok());
        s.push_str(&to_vcard(c, photo.as_deref()));
    }
    std::fs::write(path, s)?;
    Ok(all.len())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert!(same_number("+7 701 234 56 78", "87012345678"));
        assert!(same_number("+1 (555) 0142", "15550142"));
        assert!(!same_number("+7 701 234 56 78", "+7 701 234 56 79"));
        assert!(!same_number("112", "+7 701 000 0112"));
    }

    #[test]
    fn roundtrip() {
        let mut c = Contact::new("Анна Петрова, отдел; продаж", "+7 701 234 56 78");
        c.phones.push(Phone { number: "8 727 000 11 22".into(), kind: "WORK".into() });
        c.emails.push("anna@example.org".into());
        c.note = "две\nстроки".into();
        c.favorite = true;
        let photo = vec![0xFFu8, 0xD8, 1, 2, 3, 4, 5, 0xFF, 0xD9];
        let s = to_vcard(&c, Some(&photo));
        assert!(s.lines().all(|l| l.len() <= 76));
        let v = parse(&s);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, c);
        assert_eq!(v[0].1.as_deref(), Some(&photo[..]));
    }

    #[test]
    fn parse_foreign() {
        let s = "BEGIN:VCARD\r\nVERSION:2.1\r\nN:Иванов;Иван;Иванович;;\r\nTEL;CELL;PREF:+77011112233\r\nitem1.TEL;type=HOME:8 (727) 111-22-33\r\nEND:VCARD\r\nBEGIN:VCARD\nVERSION:4.0\nFN:Bob\nTEL;VALUE=uri;TYPE=work:tel:+1-555-0100\nPHOTO:data:image/jpeg;base64,/9j/AA==\nEND:VCARD\n";
        let v = parse(s);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0.name, "Иван Иванович Иванов");
        assert_eq!(v[0].0.phones.len(), 2);
        assert_eq!(v[0].0.phones[1].kind, "HOME");
        assert_eq!(v[1].0.phones[0].number, "+1-555-0100");
        assert_eq!(v[1].1.as_deref(), Some(&[0xFFu8, 0xD8, 0xFF, 0x00][..]));
    }

    #[test]
    fn store() {
        let d = std::env::temp_dir().join(format!("syn-contacts-test-{}", std::process::id()));
        std::env::set_var("SYNSHELL_CONTACTS_DIR", &d);
        let mut c = Contact::new("Bob", "+1 555 0100");
        save(&mut c).unwrap();
        set_photo(&c.uid, Some(b"jpeg")).unwrap();
        assert_eq!(find_by_number("15550100").map(|x| x.name), Some("Bob".into()));
        assert!(load(&c.uid).unwrap().photo().is_some());
        let exp = std::env::temp_dir().join(format!("syn-contacts-export-{}.vcf", std::process::id()));
        assert_eq!(export(&exp).unwrap(), 1);
        delete(&c.uid).unwrap();
        assert_eq!(import(&std::fs::read_to_string(&exp).unwrap()).unwrap(), 1);
        assert_eq!(list().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_file(&exp);
    }
}
