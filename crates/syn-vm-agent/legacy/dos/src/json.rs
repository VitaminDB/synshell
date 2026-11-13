//! Маленький JSON для сообщений агента: чтение полей плоского объекта и запись в буфер.

/// Буфер вывода фиксированного размера (лишнее отбрасывается).
pub struct Out<'a> {
    pub b: &'a mut [u8],
    pub n: usize,
}

impl<'a> Out<'a> {
    pub fn new(b: &'a mut [u8]) -> Self {
        Out { b, n: 0 }
    }
    pub fn raw(&mut self, s: &[u8]) -> &mut Self {
        for &c in s {
            if self.n < self.b.len() {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        self
    }
    /// Строка в кавычках (байты ≥ 0x80 из CP866 — кириллица в UTF-8, прочее — «?»).
    pub fn str(&mut self, s: &[u8]) -> &mut Self {
        self.raw(b"\"");
        for &c in s {
            match c {
                b'"' => self.raw(b"\\\""),
                b'\\' => self.raw(b"\\\\"),
                0..=0x1F => self.raw(b" "),
                0x80.. => {
                    let mut u = [0u8; 3];
                    let n = cp866_to_utf8(c, &mut u);
                    self.raw(&u[..n])
                }
                _ => self.raw(&[c]),
            };
        }
        self.raw(b"\"")
    }
    pub fn num(&mut self, v: u64) -> &mut Self {
        let mut d = [0u8; 20];
        let mut i = d.len();
        let mut v = v;
        loop {
            i -= 1;
            d[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let tmp = d;
        self.raw(&tmp[i..])
    }
    pub fn int(&mut self, v: i32) -> &mut Self {
        if v < 0 {
            self.raw(b"-").num(v.unsigned_abs() as u64)
        } else {
            self.num(v as u64)
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

/// CP866 → UTF-8 (кириллица; остальное — «?»).
fn cp866_to_utf8(c: u8, out: &mut [u8; 3]) -> usize {
    let u: u32 = match c {
        0x80..=0xAF => 0x0410 + (c - 0x80) as u32,
        0xE0..=0xEF => 0x0440 + (c - 0xE0) as u32,
        0xF0 => 0x0401,
        0xF1 => 0x0451,
        _ => {
            out[0] = b'?';
            return 1;
        }
    };
    out[0] = 0xC0 | (u >> 6) as u8;
    out[1] = 0x80 | (u & 0x3F) as u8;
    2
}

/// UTF-8 → CP866 (для путей и команд от хоста).
fn utf8_to_cp866(u: u32) -> u8 {
    match u {
        0..=0x7F => u as u8,
        0x0410..=0x043F => (u - 0x0410) as u8 + 0x80,
        0x0440..=0x044F => (u - 0x0440) as u8 + 0xE0,
        0x0401 => 0xF0,
        0x0451 => 0xF1,
        _ => b'?',
    }
}

/// Найти значение ключа верхнего уровня: срез, начинающийся со значения.
fn find<'a>(j: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    let mut depth = 0i32;
    while i < j.len() {
        match j[i] {
            b'"' => {
                let start = i + 1;
                let end = skip_str(j, i)?;
                if depth == 1 && &j[start..end - 1] == key {
                    let mut k = end;
                    while k < j.len() && (j[k] == b' ' || j[k] == b':') {
                        k += 1;
                    }
                    return Some(&j[k..]);
                }
                i = end;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Позиция после закрывающей кавычки строки, начинающейся в `i`.
fn skip_str(j: &[u8], i: usize) -> Option<usize> {
    let mut k = i + 1;
    while k < j.len() {
        match j[k] {
            b'\\' => k += 2,
            b'"' => return Some(k + 1),
            _ => k += 1,
        }
    }
    None
}

/// Прочитать строку JSON (`v` начинается с кавычки) в CP866; длина или None.
fn read_str(v: &[u8], out: &mut [u8]) -> Option<usize> {
    if v.first() != Some(&b'"') {
        return None;
    }
    let mut n = 0;
    let mut k = 1;
    let mut push = |c: u8, n: &mut usize| {
        if *n + 1 < out.len() {
            out[*n] = c;
            *n += 1;
        }
    };
    while k < v.len() {
        let c = v[k];
        match c {
            b'"' => return Some(n),
            b'\\' => {
                let e = *v.get(k + 1)?;
                k += 2;
                match e {
                    b'n' => push(b'\n', &mut n),
                    b't' => push(b'\t', &mut n),
                    b'r' => push(b'\r', &mut n),
                    b'u' => {
                        let h = v.get(k..k + 4)?;
                        let mut u = 0u32;
                        for &d in h {
                            u = u * 16 + (d as char).to_digit(16)?;
                        }
                        k += 4;
                        push(utf8_to_cp866(u), &mut n);
                    }
                    other => push(other, &mut n),
                }
                continue;
            }
            0xC0..=0xDF if k + 1 < v.len() => {
                let u = ((c as u32 & 0x1F) << 6) | (v[k + 1] as u32 & 0x3F);
                push(utf8_to_cp866(u), &mut n);
                k += 2;
                continue;
            }
            0xE0..=0xFF => {
                push(b'?', &mut n);
                k += if c >= 0xF0 { 4 } else { 3 };
                continue;
            }
            _ => push(c, &mut n),
        }
        k += 1;
    }
    None
}

/// Поле-строка.
pub fn get_str(j: &[u8], key: &[u8], out: &mut [u8]) -> Option<usize> {
    read_str(find(j, key)?, out)
}

/// Поле-число.
pub fn get_num(j: &[u8], key: &[u8]) -> Option<u64> {
    let v = find(j, key)?;
    let mut n = 0u64;
    let mut any = false;
    for &c in v {
        if c.is_ascii_digit() {
            n = n * 10 + (c - b'0') as u64;
            any = true;
        } else {
            break;
        }
    }
    any.then_some(n)
}

/// Поле-массив строк: склеить через пробел (аргументы с пробелами — в кавычках).
pub fn get_argv(j: &[u8], key: &[u8], out: &mut [u8]) -> usize {
    let Some(v) = find(j, key) else { return 0 };
    if v.first() != Some(&b'[') {
        return 0;
    }
    let mut n = 0;
    let mut k = 1;
    let mut one = [0u8; 128];
    while k < v.len() {
        match v[k] {
            b'"' => {
                let Some(len) = read_str(&v[k..], &mut one) else { break };
                let quote = one[..len].contains(&b' ');
                if n > 0 && n < out.len() {
                    out[n] = b' ';
                    n += 1;
                }
                let q: &[u8] = if quote { b"\"" } else { b"" };
                for part in [q, &one[..len], q] {
                    for &c in part {
                        if n < out.len() {
                            out[n] = c;
                            n += 1;
                        }
                    }
                }
                k = skip_str(v, k).unwrap_or(v.len());
            }
            b']' => break,
            _ => k += 1,
        }
    }
    n
}
