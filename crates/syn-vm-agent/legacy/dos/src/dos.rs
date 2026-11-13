#![allow(dead_code)]
//! Вызовы DOS и BIOS из .COM (модель tiny: DS = ES = SS = CS). Регистры — через структуру в памяти:
//! так не важно, какие регистры компилятор держит за собой (ebx/esi/ebp сохраняются вручную).
use core::arch::asm;

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct R {
    pub ax: u16,
    pub bx: u16,
    pub cx: u16,
    pub dx: u16,
    pub si: u16,
    pub di: u16,
    pub es: u16,
    pub flags: u16,
}

impl R {
    pub fn cf(&self) -> bool {
        self.flags & 1 != 0
    }
}

macro_rules! int_fn {
    ($name:ident, $n:literal) => {
        #[inline(never)]
        pub fn $name(r: &mut R) {
            unsafe {
                asm!(
                    "push ebp",
                    "push ebx",
                    "push esi",
                    "push edi",
                    "mov ebp, eax",
                    "push ebp",
                    "push es",
                    "mov ax, [ebp + 12]",
                    "mov es, ax",
                    "mov ax, [ebp]",
                    "mov bx, [ebp + 2]",
                    "mov cx, [ebp + 4]",
                    "mov dx, [ebp + 6]",
                    "mov si, [ebp + 8]",
                    "mov di, [ebp + 10]",
                    concat!("int ", $n),
                    "pushf",
                    "mov ebp, [esp + 4]",
                    "mov [ebp], ax",
                    "mov [ebp + 2], bx",
                    "mov [ebp + 4], cx",
                    "mov [ebp + 6], dx",
                    "mov [ebp + 8], si",
                    "mov [ebp + 10], di",
                    "mov ax, es",
                    "mov [ebp + 12], ax",
                    "pop ax",
                    "mov [ebp + 14], ax",
                    "pop es",
                    "pop ebp",
                    "pop edi",
                    "pop esi",
                    "pop ebx",
                    "pop ebp",
                    inout("eax") r as *mut R => _,
                    out("ecx") _,
                    out("edx") _,
                );
            }
        }
    };
}

int_fn!(int21, "0x21");
int_fn!(int10, "0x10");
int_fn!(int16, "0x16");
int_fn!(int2f, "0x2F");
int_fn!(int33, "0x33");

pub fn outb(port: u16, v: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack)) }
}
pub fn inb(port: u16) -> u8 {
    let v: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") v, options(nomem, nostack)) }
    v
}
/// Создать каталог (int 21h 39h); true — создан или уже есть.
pub fn mkdir(p: &Path) -> bool {
    let mut r = R { ax: 0x3900, dx: p.ptr(), ..R::default() };
    int21(&mut r);
    !r.cf() || exists(p)
}

pub fn outw(port: u16, v: u16) {
    unsafe { asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack)) }
}
pub fn inw(port: u16) -> u16 {
    let v: u16;
    unsafe { asm!("in ax, dx", in("dx") port, out("ax") v, options(nomem, nostack)) }
    v
}

pub fn outl(port: u16, v: u32) {
    unsafe { asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack)) }
}
pub fn inl(port: u16) -> u32 {
    let v: u32;
    unsafe { asm!("in eax, dx", in("dx") port, out("eax") v, options(nomem, nostack)) }
    v
}

/// Запись в видеопамять текста (B800:смещение).
pub fn poke16(seg: u16, off: u16, v: u16) {
    unsafe {
        asm!("push es", "mov es, {s:x}", "mov word ptr es:[{o:e}], {v:x}", "pop es",
             s = in(reg) seg, o = in(reg) off as u32, v = in(reg) v)
    }
}

pub fn peek16(seg: u16, off: u16) -> u16 {
    let v: u16;
    unsafe {
        asm!("push es", "mov es, {s:x}", "mov {v:x}, word ptr es:[{o:e}]", "pop es",
             s = in(reg) seg, o = in(reg) off as u32, v = out(reg) v)
    }
    v
}

pub fn peek8(seg: u16, off: u16) -> u8 {
    let v: u8;
    unsafe {
        asm!("push es", "mov es, {s:x}", "mov {v}, byte ptr es:[{o:e}]", "pop es",
             s = in(reg) seg, o = in(reg) off as u32, v = out(reg_byte) v)
    }
    v
}

pub fn ds() -> u16 {
    let v: u16;
    unsafe { asm!("mov {:x}, ds", out(reg) v, options(nomem, nostack)) }
    v
}

// ─── Файлы ───

/// Строка C в буфере (имя файла DOS).
pub struct Path {
    pub b: [u8; 128],
    pub n: usize,
}

impl Path {
    pub const fn new() -> Path {
        Path { b: [0; 128], n: 0 }
    }
    pub fn from(s: &[u8]) -> Path {
        let mut p = Path::new();
        p.push(s);
        p
    }
    pub fn push(&mut self, s: &[u8]) {
        for &c in s {
            if c == 0 || self.n + 1 >= self.b.len() {
                break;
            }
            self.b[self.n] = c;
            self.n += 1;
        }
        self.b[self.n] = 0;
    }
    pub fn truncate(&mut self, n: usize) {
        self.n = n.min(self.n);
        self.b[self.n] = 0;
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.b[..self.n]
    }
    pub fn ptr(&self) -> u16 {
        self.b.as_ptr() as usize as u16
    }
}

pub fn open(p: &Path, mode: u8) -> Option<u16> {
    let mut r = R { ax: 0x3D00 | mode as u16, dx: p.ptr(), ..R::default() };
    int21(&mut r);
    (!r.cf()).then_some(r.ax)
}

pub fn create(p: &Path) -> Option<u16> {
    let mut r = R { ax: 0x3C00, cx: 0, dx: p.ptr(), ..R::default() };
    int21(&mut r);
    (!r.cf()).then_some(r.ax)
}

pub fn read(h: u16, buf: &mut [u8]) -> Option<usize> {
    let mut r = R { ax: 0x3F00, bx: h, cx: buf.len() as u16, dx: buf.as_mut_ptr() as usize as u16, ..R::default() };
    int21(&mut r);
    (!r.cf()).then_some(r.ax as usize)
}

pub fn write(h: u16, buf: &[u8]) -> Option<usize> {
    let mut r = R { ax: 0x4000, bx: h, cx: buf.len() as u16, dx: buf.as_ptr() as usize as u16, ..R::default() };
    int21(&mut r);
    (!r.cf()).then_some(r.ax as usize)
}

pub fn close(h: u16) {
    let mut r = R { ax: 0x3E00, bx: h, ..R::default() };
    int21(&mut r);
}

pub fn rename(from: &Path, to: &Path) -> bool {
    let mut r = R { ax: 0x5600, dx: from.ptr(), di: to.ptr(), es: ds(), ..R::default() };
    int21(&mut r);
    !r.cf()
}

pub fn delete(p: &Path) -> bool {
    let mut r = R { ax: 0x4100, dx: p.ptr(), ..R::default() };
    int21(&mut r);
    !r.cf()
}

pub fn exists(p: &Path) -> bool {
    let mut r = R { ax: 0x4300, dx: p.ptr(), ..R::default() };
    int21(&mut r);
    !r.cf()
}

/// Копировать файл (буфер — с вызывающего).
pub fn copy(from: &Path, to: &Path, buf: &mut [u8]) -> bool {
    let Some(src) = open(from, 0) else { return false };
    let Some(dst) = create(to) else {
        close(src);
        return false;
    };
    let mut ok = true;
    loop {
        match read(src, buf) {
            Some(0) => break,
            Some(n) => {
                if write(dst, &buf[..n]) != Some(n) {
                    ok = false;
                    break;
                }
            }
            None => {
                ok = false;
                break;
            }
        }
    }
    close(src);
    close(dst);
    ok
}

/// DTA для поиска файлов (43 байта): атрибут — 21, размер — 26, имя — 30.
#[repr(C)]
pub struct Dta {
    pub raw: [u8; 43],
}

impl Dta {
    pub const fn new() -> Dta {
        Dta { raw: [0; 43] }
    }
    pub fn attr(&self) -> u8 {
        self.raw[21]
    }
    #[allow(dead_code)]
    pub fn size(&self) -> u32 {
        u32::from_le_bytes([self.raw[26], self.raw[27], self.raw[28], self.raw[29]])
    }
    pub fn name(&self) -> &[u8] {
        let n = &self.raw[30..43];
        &n[..n.iter().position(|&c| c == 0).unwrap_or(13)]
    }
}

pub fn set_dta(d: &mut Dta) {
    let mut r = R { ax: 0x1A00, dx: d.raw.as_mut_ptr() as usize as u16, ..R::default() };
    int21(&mut r);
}

/// Найти первый: маска, атрибуты (0x10 — и каталоги).
pub fn find_first(mask: &Path, attr: u16) -> bool {
    let mut r = R { ax: 0x4E00, cx: attr, dx: mask.ptr(), ..R::default() };
    int21(&mut r);
    !r.cf()
}

pub fn find_next() -> bool {
    let mut r = R { ax: 0x4F00, ..R::default() };
    int21(&mut r);
    !r.cf()
}

/// Диск существует и не съёмный (жёсткий): IOCTL 4408h (0 — съёмный, 1 — нет), CD-ROM — через MSCDEX.
pub fn fixed_drive(n: u8) -> bool {
    let mut r = R { ax: 0x4408, bx: n as u16, ..R::default() };
    int21(&mut r);
    if r.cf() || r.ax != 1 {
        return false;
    }
    // CD-ROM (MSCDEX/SHSUCDX): int 2Fh AX=150Bh, CX=диск (0 — A:) → BX=ADADh и AX≠0.
    let mut c = R { ax: 0x150B, cx: (n - 1) as u16, ..R::default() };
    int2f(&mut c);
    !(c.bx == 0xADAD && c.ax != 0)
}

/// Клавиша: (ASCII, скан-код).
pub fn key() -> (u8, u8) {
    let mut r = R::default();
    int16(&mut r);
    (r.ax as u8, (r.ax >> 8) as u8)
}

pub fn exit(code: u8) -> ! {
    let mut r = R { ax: 0x4C00 | code as u16, ..R::default() };
    int21(&mut r);
    loop {}
}
