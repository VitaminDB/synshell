//! SYNAGENT.COM — гостевой агент syn-vm для DOS.
//!
//! `SYNAGENT` (или `SYNAGENT RUN`) — режим «удалённая работа»: агент на переднем плане слушает COM2 и
//! выполняет запросы хоста — команды (через COMMAND.COM, вывод — файлом), файлы туда и обратно с SHA-256,
//! список каталога, mkdir, часы. Esc — выйти. DOS однозадачна: пока идёт игра, агент не отвечает.
//!
//! `SYNAGENT INSTALL` — копия в C:\SYNVM, строка `SYNAGENT SETUP` в автозапуске (FDAUTO.BAT FreeDOS или
//! AUTOEXEC.BAT) и шаги INSTALL.TXT каталога системы на диске (драйверы, утилиты) с продолжением после
//! перезагрузок. `SYNAGENT SETUP` — выполнить оставшиеся шаги и выйти.

#![no_std]
#![no_main]

mod dos;
mod json;
mod sha256;

use dos::*;

core::arch::global_asm!(
    r#"
    .section .text.entry, "ax"
    .code16
    .global _start
_start:
    cld
    movzwl %sp, %esp
    movw $__bss_start, %di
    movw $__bss_end, %cx
    subw %di, %cx
    xorb %al, %al
    rep stosb
    calll main_rs
    movw $0x4C00, %ax
    int $0x21
"#,
    options(att_syntax)
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    exit(3)
}

/// Порт COM2.
const COM: u16 = 0x2F8;
/// Кусок данных файла в кадре: весь агент живёт в 64 КиБ.
const CHUNK: usize = 2048;
const MAGIC: &[u8; 4] = b"SVA1";
/// Версия протокола (syn_vm_agent::VERSION).
const VERSION: u64 = 1;
/// Предел принимаемого кадра (тело).
const RX_MAX: usize = CHUNK + 1024;

static mut RX: [u8; RX_MAX + 8] = [0; RX_MAX + 8];
static mut TX: [u8; 1024] = [0; 1024];
static mut BIG: [u8; 8192] = [0; 8192];
static mut IO: [u8; CHUNK] = [0; CHUNK];
static mut SAVED_SP: u32 = 0;

#[allow(static_mut_refs)]
fn rx() -> &'static mut [u8; RX_MAX + 8] {
    unsafe { &mut RX }
}
#[allow(static_mut_refs)]
fn tx() -> &'static mut [u8; 1024] {
    unsafe { &mut TX }
}
#[allow(static_mut_refs)]
fn big() -> &'static mut [u8; 8192] {
    unsafe { &mut BIG }
}
#[allow(static_mut_refs)]
fn io() -> &'static mut [u8; CHUNK] {
    unsafe { &mut IO }
}

// ─── экран ──────────────────────────────────────────────────────────────

fn print(s: &[u8]) {
    for &c in s {
        let mut r = R { ax: 0x0200, dx: c as u16, ..R::default() };
        int21(&mut r);
    }
}

fn println(s: &[u8]) {
    print(s);
    print(b"\r\n");
}

// ─── UART ───────────────────────────────────────────────────────────────

fn uart_init() {
    outb(COM + 1, 0);
    outb(COM + 3, 0x80);
    outb(COM, 1); // 115200
    outb(COM + 1, 0);
    outb(COM + 3, 0x03); // 8N1
    outb(COM + 2, 0xC7); // FIFO
    outb(COM + 4, 0x03); // DTR, RTS
}

fn getc() -> Option<u8> {
    (inb(COM + 5) & 1 != 0).then(|| inb(COM))
}

fn putc(b: u8) {
    while inb(COM + 5) & 0x20 == 0 {}
    outb(COM, b);
}

/// Кадр: метка, длина, длина JSON, JSON, данные.
fn send(msg: &[u8], data: &[u8]) {
    let len = (4 + msg.len() + data.len()) as u32;
    for &b in MAGIC.iter().chain(&len.to_le_bytes()).chain(&(msg.len() as u32).to_le_bytes()).chain(msg).chain(data) {
        putc(b);
    }
}

/// Сообщение без данных из построителя.
fn send_json(f: impl FnOnce(&mut json::Out)) {
    let mut o = json::Out::new(tx());
    f(&mut o);
    let n = o.n;
    send(&tx()[..n], &[]);
}

/// Ждать кадр; `true` — пришёл (тело в RX[8..8+len]), `false` — нажат Esc.
fn recv(len_out: &mut usize) -> bool {
    let buf = rx();
    let mut have = 0usize;
    loop {
        // Клавиатура: Esc — выйти из удалённой работы.
        let mut k = R { ax: 0x0100, ..R::default() };
        int16(&mut k);
        if k.flags & 0x40 == 0 {
            let (ch, _) = key();
            if ch == 27 {
                return false;
            }
        }
        while let Some(b) = getc() {
            if have < 4 {
                // поиск метки
                if b == MAGIC[have] {
                    buf[have] = b;
                    have += 1;
                } else {
                    have = if b == MAGIC[0] { 1 } else { 0 };
                }
                continue;
            }
            buf[have] = b;
            have += 1;
            if have >= 8 {
                let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
                if !(4..=RX_MAX).contains(&len) {
                    have = 0;
                    continue;
                }
                if have == 8 + len {
                    *len_out = len;
                    return true;
                }
            }
        }
    }
}

// ─── DOS: память, окружение, запуск ─────────────────────────────────────

fn psp() -> u16 {
    let mut r = R { ax: 0x6200, ..R::default() };
    int21(&mut r);
    r.bx
}

/// Отдать DOS память за своим сегментом (иначе запуск программ не найдёт места).
fn shrink() {
    let mut r = R { ax: 0x4A00, bx: 0x1000, es: psp(), ..R::default() };
    int21(&mut r);
}

/// Переменная окружения (сегмент окружения — PSP:2Ch).
fn env(name: &[u8], out: &mut Path) -> bool {
    let seg = peek16(psp(), 0x2C);
    let mut off = 0u16;
    loop {
        if peek8(seg, off) == 0 {
            return false;
        }
        let mut i = 0;
        while i < name.len() && peek8(seg, off + i as u16) == name[i] {
            i += 1;
        }
        let matched = i == name.len() && peek8(seg, off + i as u16) == b'=';
        let mut p = off;
        while peek8(seg, p) != 0 {
            p += 1;
        }
        if matched {
            out.truncate(0);
            let mut q = off + i as u16 + 1;
            while q < p {
                out.push(&[peek8(seg, q)]);
                q += 1;
            }
            return true;
        }
        off = p + 1;
        if off > 0x7FF0 {
            return false;
        }
    }
}

/// Путь своей программы (за окружением: двойной ноль, счётчик, путь).
fn self_path(out: &mut Path) {
    let seg = peek16(psp(), 0x2C);
    let mut off = 0u16;
    while !(peek8(seg, off) == 0 && peek8(seg, off + 1) == 0) && off < 0x7FF0 {
        off += 1;
    }
    off += 4;
    out.truncate(0);
    while peek8(seg, off) != 0 && out.n < 120 {
        out.push(&[peek8(seg, off)]);
        off += 1;
    }
}

/// Запустить программу (int 21h 4B00h) с хвостом команды; код возврата или None.
fn exec(prog: &Path, tail: &[u8]) -> Option<u8> {
    let cmd = io();
    let n = tail.len().min(125);
    cmd[0] = n as u8;
    cmd[1..1 + n].copy_from_slice(&tail[..n]);
    cmd[1 + n] = 0x0D;
    let seg = ds();
    // Блок параметров: окружение (0 — своё), хвост, FCB1, FCB2 (дальние указатели).
    let fcb = 0x5Cu16;
    let block: [u16; 7] = [0, cmd.as_ptr() as usize as u16, seg, fcb, psp(), 0x6C, psp()];
    let (ax32, cf): (u32, u8);
    unsafe {
        core::arch::asm!(
            "push ebp",
            "push ebx",
            "push esi",
            "push edi",
            "push ds",
            "push es",
            "mov dword ptr cs:[{sp}], esp",
            "push ds",
            "pop es",
            "mov ax, 0x4B00",
            "int 0x21",
            "setc cl",
            "cli",
            "mov dx, cs",
            "mov ss, dx",
            "mov ds, dx",
            "mov esp, dword ptr cs:[{sp}]",
            "sti",
            "pop es",
            "pop ds",
            "pop edi",
            "pop esi",
            "pop ebx",
            "pop ebp",
            sp = sym SAVED_SP,
            inout("edx") prog.ptr() as u32 => _,
            inout("eax") 0u32 => ax32,
            in("ebx") block.as_ptr() as usize as u32,
            out("cl") cf,
        );
    }
    let _ = ax32;
    if cf != 0 {
        return None;
    }
    let mut r = R { ax: 0x4D00, ..R::default() };
    int21(&mut r);
    Some(r.ax as u8)
}

/// Команда через COMMAND.COM /C, вывод — в C:\SYNVM\EXEC.OUT. Код.
fn shell(cmdline: &[u8], capture: bool) -> Option<u8> {
    let mut comspec = Path::new();
    if !env(b"COMSPEC", &mut comspec) {
        comspec = Path::from(b"C:\\COMMAND.COM");
    }
    let t = big();
    let mut o = json::Out::new(&mut t[..]);
    o.raw(b" /C ").raw(cmdline);
    if capture {
        o.raw(b" > C:\\SYNVM\\EXEC.OUT");
    }
    let n = o.n;
    let mut tail = [0u8; 126];
    let m = n.min(126);
    tail[..m].copy_from_slice(&t[..m]);
    exec(&comspec, &tail[..m])
}

/// Сменить диск и каталог.
fn chdir(p: &[u8]) {
    if p.len() >= 2 && p[1] == b':' {
        let mut r = R { ax: 0x0E00, dx: (p[0].to_ascii_uppercase() - b'A') as u16, ..R::default() };
        int21(&mut r);
    }
    let path = Path::from(p);
    let mut r = R { ax: 0x3B00, dx: path.ptr(), ..R::default() };
    int21(&mut r);
}

/// Создать каталог с родителями.
fn mkdirs(p: &[u8]) -> bool {
    let mut i = 3;
    while i <= p.len() {
        if i == p.len() || p[i] == b'\\' {
            let part = Path::from(&p[..i]);
            if !exists(&part) {
                mkdir(&part);
            }
        }
        i += 1;
    }
    exists(&Path::from(p))
}

fn parent(p: &[u8]) -> &[u8] {
    match p.iter().rposition(|&c| c == b'\\') {
        Some(i) => &p[..i],
        None => p,
    }
}

// ─── обработка запросов ─────────────────────────────────────────────────

fn hello() {
    let mut r = R { ax: 0x3000, ..R::default() };
    int21(&mut r);
    let (major, minor) = (r.ax & 0xFF, r.ax >> 8);
    let free_dos = (r.bx >> 8) == 0xFD;
    let mut t = R { ax: 0, ..R::default() };
    int1a(&mut t);
    let ticks = ((t.cx as u64) << 16) | t.dx as u64;
    send_json(|o| {
        o.raw(b"{\"t\":\"hello\",\"version\":").num(VERSION);
        o.raw(b",\"agent\":\"syn-vm-agent-dos 0.2.0\",\"os\":\"");
        o.raw(if free_dos { b"FreeDOS " } else { b"DOS " }).num(major as u64).raw(b".").num(minor as u64);
        o.raw(b"\",\"host\":\"DOS\",\"boot\":\"dos\",\"instance\":").num(ticks + 1);
        o.raw(b",\"user\":null,\"max_chunk\":").num(CHUNK as u64).raw(b"}");
    });
}

fn int1a(r: &mut R) {
    // int 1Ah AH=0: тики с полуночи в CX:DX.
    let (cx, dx): (u16, u16);
    unsafe { core::arch::asm!("push ebx", "xor ax, ax", "int 0x1A", "pop ebx", out("cx") cx, out("dx") dx, out("eax") _) };
    r.cx = cx;
    r.dx = dx;
}

fn simple(kind: &[u8], id: u64) {
    send_json(|o| {
        o.raw(b"{\"t\":\"").raw(kind).raw(b"\",\"id\":").num(id).raw(b"}");
    });
}

fn error(id: u64, msg: &[u8], what: &[u8]) {
    send_json(|o| {
        o.raw(b"{\"t\":\"error\",\"id\":").num(id).raw(b",\"msg\":");
        let mut both = [0u8; 200];
        let mut n = 0;
        for &c in what.iter().chain(b": ").chain(msg) {
            if n < both.len() {
                both[n] = c;
                n += 1;
            }
        }
        o.str(&both[..n]).raw(b"}");
    });
}

fn done(id: u64, size: u64, sha: &[u8]) {
    send_json(|o| {
        o.raw(b"{\"t\":\"done\",\"id\":").num(id).raw(b",\"size\":").num(size).raw(b",\"sha256\":\"").raw(sha).raw(b"\"}");
    });
}

struct Upload {
    id: u64,
    h: u16,
    size: u64,
    sha: sha256::Sha256,
}

fn handle(j: &[u8], data: &[u8], up: &mut Option<Upload>) -> bool {
    let mut t = [0u8; 16];
    let tn = json::get_str(j, b"t", &mut t).unwrap_or(0);
    let t = &t[..tn];
    let id = json::get_num(j, b"id").unwrap_or(0);
    let mut path = [0u8; 128];
    let pn = json::get_str(j, b"path", &mut path).unwrap_or(0);
    let path = &path[..pn];
    match t {
        b"hi" => hello(),
        b"ping" => simple(b"pong", id),
        b"put" => {
            mkdirs(parent(path));
            match create(&Path::from(path)) {
                Some(h) => *up = Some(Upload { id, h, size: 0, sha: sha256::Sha256::new() }),
                None => error(id, b"not writable", path),
            }
        }
        b"data" => {
            if let Some(u) = up.as_mut().filter(|u| u.id == id) {
                write(u.h, data);
                u.size += data.len() as u64;
                u.sha.update(data);
            }
        }
        b"put-end" => {
            if let Some(u) = up.take() {
                close(u.h);
                let sha = u.sha.hex();
                done(u.id, u.size, &sha);
            }
        }
        b"mkdir" => {
            if mkdirs(path) {
                done(id, 0, b"");
            } else {
                error(id, b"not created", path);
            }
        }
        b"get" => get(id, path),
        b"list" => list(id, path),
        b"exec" => {
            let mut cmd = [0u8; 120];
            let n = json::get_argv(j, b"argv", &mut cmd);
            let mut cwd = [0u8; 100];
            if let Some(cn) = json::get_str(j, b"cwd", &mut cwd) {
                chdir(&cwd[..cn]);
            }
            send_json(|o| {
                o.raw(b"{\"t\":\"started\",\"id\":").num(id).raw(b",\"pid\":0}");
            });
            mkdirs(b"C:\\SYNVM");
            let code = shell(&cmd[..n], true);
            if let Some(h) = open(&Path::from(b"C:\\SYNVM\\EXEC.OUT"), 0) {
                while let Some(k) = read(h, io()).filter(|&k| k > 0) {
                    let mut o = json::Out::new(tx());
                    o.raw(b"{\"t\":\"out\",\"id\":").num(id).raw(b",\"fd\":1}");
                    let m = o.n;
                    send(&tx()[..m], &io()[..k]);
                }
                close(h);
            }
            send_json(|o| {
                o.raw(b"{\"t\":\"exit\",\"id\":").num(id).raw(b",\"code\":").int(code.map_or(-1, |c| c as i32)).raw(b"}");
            });
        }
        b"set-time" => {
            if let Some(ms) = json::get_num(j, b"unix_ms") {
                set_time(ms / 1000);
            }
        }
        _ => {}
    }
    true
}

fn get(id: u64, path: &[u8]) {
    let Some(h) = open(&Path::from(path), 0) else {
        error(id, b"not readable", path);
        return;
    };
    // Размер: lseek в конец и обратно.
    let mut r = R { ax: 0x4202, bx: h, ..R::default() };
    int21(&mut r);
    let size = ((r.dx as u64) << 16) | r.ax as u64;
    let mut z = R { ax: 0x4200, bx: h, ..R::default() };
    int21(&mut z);
    send_json(|o| {
        o.raw(b"{\"t\":\"stat\",\"id\":").num(id).raw(b",\"size\":").num(size).raw(b"}");
    });
    let mut sha = sha256::Sha256::new();
    while let Some(k) = read(h, io()).filter(|&k| k > 0) {
        sha.update(&io()[..k]);
        let mut o = json::Out::new(tx());
        o.raw(b"{\"t\":\"data\",\"id\":").num(id).raw(b"}");
        let m = o.n;
        send(&tx()[..m], &io()[..k]);
    }
    close(h);
    let hex = sha.hex();
    send_json(|o| {
        o.raw(b"{\"t\":\"end\",\"id\":").num(id).raw(b",\"sha256\":\"").raw(&hex).raw(b"\"}");
    });
}

/// Рекурсивный список каталога в BIG (сколько поместится).
fn list(id: u64, path: &[u8]) {
    // Файл, а не каталог: одна запись с пустым путём (как у остальных агентов).
    let mut attr = R { ax: 0x4300, dx: Path::from(path).ptr(), ..R::default() };
    let p = Path::from(path);
    attr.dx = p.ptr();
    int21(&mut attr);
    if attr.cf() {
        error(id, b"not found", path);
        return;
    }
    if attr.cx & 0x10 == 0 {
        let size = open(&p, 0).map_or(0, |h| {
            let mut r = R { ax: 0x4202, bx: h, ..R::default() };
            int21(&mut r);
            close(h);
            ((r.dx as u64) << 16) | r.ax as u64
        });
        send_json(|o| {
            o.raw(b"{\"t\":\"entries\",\"id\":").num(id).raw(b",\"entries\":[{\"path\":\"\",\"dir\":false,\"size\":").num(size).raw(b"}]}");
        });
        return;
    }
    let b = big();
    let mut o = json::Out::new(&mut b[..]);
    o.raw(b"{\"t\":\"entries\",\"id\":").num(id).raw(b",\"entries\":[");
    let mut first = true;
    walk(path, b"", &mut o, &mut first, 0);
    o.raw(b"]}");
    let n = o.n;
    send(&b[..n], &[]);
}

fn walk(dir: &[u8], rel: &[u8], o: &mut json::Out, first: &mut bool, depth: u8) {
    if depth > 6 {
        return;
    }
    let mut dta = Dta::new();
    set_dta(&mut dta);
    let mut mask = Path::from(dir);
    mask.push(b"\\*.*");
    if !find_first(&mask, 0x10) {
        return;
    }
    loop {
        let name = Path::from(dta.name());
        if name.as_bytes() != b"." && name.as_bytes() != b".." {
            let is_dir = dta.attr() & 0x10 != 0;
            let mut r = Path::from(rel);
            if !rel.is_empty() {
                r.push(b"/");
            }
            r.push(name.as_bytes());
            if !*first {
                o.raw(b",");
            }
            *first = false;
            o.raw(b"{\"path\":").str(r.as_bytes()).raw(b",\"dir\":").raw(if is_dir { b"true" } else { b"false" }).raw(b",\"size\":").num(if is_dir { 0 } else { dta.size() as u64 }).raw(b"}");
            if is_dir {
                let mut sub = Path::from(dir);
                sub.push(b"\\");
                sub.push(name.as_bytes());
                // Поиск во вложенном каталоге портит DTA — после него продолжить с того же имени.
                walk(sub.as_bytes(), r.as_bytes(), o, first, depth + 1);
                set_dta(&mut dta);
                if !resume(&mask, name.as_bytes()) {
                    return;
                }
            }
        }
        if !find_next() {
            return;
        }
    }
}

/// Повторить поиск до имени `after` (DTA уже установлена).
fn resume(mask: &Path, after: &[u8]) -> bool {
    if !find_first(mask, 0x10) {
        return false;
    }
    // Имя прочитать из текущей DTA через dos::Dta нельзя без ссылки — сравнить побайтно по смещению 30.
    loop {
        let mut r = R { ax: 0x2F00, ..R::default() };
        int21(&mut r); // ES:BX — DTA (наш сегмент)
        let mut name = [0u8; 13];
        for (i, c) in name.iter_mut().enumerate() {
            *c = peek8(r.es, r.bx + 30 + i as u16);
        }
        let n = name.iter().position(|&c| c == 0).unwrap_or(13);
        if &name[..n] == after {
            return true;
        }
        if !find_next() {
            return false;
        }
    }
}

/// Часы DOS — по UTC хоста (машины DOS у syn-vm с RTC в UTC).
fn set_time(secs: u64) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let mut r = R { ax: 0x2B00, cx: y as u16, dx: ((m as u16) << 8) | d as u16, ..R::default() };
    int21(&mut r);
    let mut t = R { ax: 0x2D00, cx: (((rem / 3600) as u16) << 8) | ((rem % 3600 / 60) as u16), dx: ((rem % 60) as u16) << 8, ..R::default() };
    int21(&mut t);
}

// ─── установка ──────────────────────────────────────────────────────────

/// Скопировать каталог целиком.
fn copy_tree(from: &[u8], to: &[u8], depth: u8) {
    if depth > 6 {
        return;
    }
    mkdirs(to);
    let mut dta = Dta::new();
    set_dta(&mut dta);
    let mut mask = Path::from(from);
    mask.push(b"\\*.*");
    if !find_first(&mask, 0x10) {
        return;
    }
    loop {
        let name = Path::from(dta.name());
        if name.as_bytes() != b"." && name.as_bytes() != b".." {
            let mut s = Path::from(from);
            s.push(b"\\");
            s.push(name.as_bytes());
            let mut d = Path::from(to);
            d.push(b"\\");
            d.push(name.as_bytes());
            if dta.attr() & 0x10 != 0 {
                copy_tree(s.as_bytes(), d.as_bytes(), depth + 1);
                set_dta(&mut dta);
                if !resume(&mask, name.as_bytes()) {
                    return;
                }
            } else {
                copy(&s, &d, io());
            }
        }
        if !find_next() {
            return;
        }
    }
}

fn read_all<'a>(path: &[u8], buf: &'a mut [u8]) -> Option<&'a [u8]> {
    let h = open(&Path::from(path), 0)?;
    let n = read(h, buf).unwrap_or(0);
    close(h);
    Some(&buf[..n])
}

fn write_all(path: &[u8], data: &[u8]) {
    if let Some(h) = create(&Path::from(path)) {
        write(h, data);
        close(h);
    }
}

fn install() {
    mkdirs(b"C:\\SYNVM");
    let mut me = Path::new();
    self_path(&mut me);
    let dst = Path::from(b"C:\\SYNVM\\SYNAGENT.COM");
    if !me.as_bytes().eq_ignore_ascii_case(dst.as_bytes()) && !copy(&me, &dst, io()) {
        println(b"SYNAGENT: copy to C:\\SYNVM failed");
        return;
    }
    // Каталог системы на диске: ...\DOS\AGENT\SYNAGENT.COM -> ...\DOS
    let sys = Path::from(parent(parent(me.as_bytes())));
    let mut manifest = Path::from(sys.as_bytes());
    manifest.push(b"\\INSTALL.TXT");
    if exists(&manifest) {
        copy_tree(sys.as_bytes(), b"C:\\SYNVM\\SETUP", 0);
        write_all(b"C:\\SYNVM\\STEP.TXT", b"0");
    }
    // Автозапуск: FreeDOS — FDAUTO.BAT, иначе AUTOEXEC.BAT.
    let auto: &[u8] = if exists(&Path::from(b"C:\\FDAUTO.BAT")) { b"C:\\FDAUTO.BAT" } else { b"C:\\AUTOEXEC.BAT" };
    let line: &[u8] = b"C:\\SYNVM\\SYNAGENT.COM SETUP";
    let have = read_all(auto, big()).is_some_and(|t| t.windows(line.len()).any(|w| w.eq_ignore_ascii_case(line)));
    if !have {
        let h = open(&Path::from(auto), 2).or_else(|| create(&Path::from(auto)));
        if let Some(h) = h {
            let mut r = R { ax: 0x4202, bx: h, ..R::default() };
            int21(&mut r);
            write(h, b"\r\n");
            write(h, line);
            write(h, b"\r\n");
            close(h);
        }
    }
    println(b"SYNAGENT: installed to C:\\SYNVM");
    setup();
}

/// Шаги C:\SYNVM\SETUP\INSTALL.TXT с места, где остановились.
fn setup() {
    let mut sb = [0u8; 16];
    let Some(s) = read_all(b"C:\\SYNVM\\STEP.TXT", &mut sb) else { return };
    let mut n = 0usize;
    for &c in s {
        if c.is_ascii_digit() {
            n = n * 10 + (c - b'0') as usize;
        } else {
            return; // failed N — ждать человека
        }
    }
    let text_buf = big();
    let Some(text) = read_all(b"C:\\SYNVM\\SETUP\\INSTALL.TXT", &mut text_buf[..]) else { return };
    let len = text.len();
    let mut i = 0usize;
    let mut idx = 0usize;
    let mut pos = 0usize;
    while pos < len {
        let end = big()[pos..len].iter().position(|&c| c == b'\n').map_or(len, |k| pos + k);
        let mut line = [0u8; 128];
        let raw = &big()[pos..end];
        let raw = trim(raw);
        let ln = raw.len().min(127);
        line[..ln].copy_from_slice(&raw[..ln]);
        pos = end + 1;
        if ln == 0 || line[0] == b'#' {
            continue;
        }
        if idx < n {
            idx += 1;
            continue;
        }
        let mut cmd = &line[..ln];
        let mut reboot = false;
        let mut skip = false;
        loop {
            if cmd.len() > 7 && cmd[..7].eq_ignore_ascii_case(b"reboot ") {
                reboot = true;
                cmd = trim(&cmd[7..]);
            } else if cmd.len() > 3 && cmd[..3].eq_ignore_ascii_case(b"if=") {
                let sp = cmd.iter().position(|&c| c == b' ').unwrap_or(cmd.len());
                let cond = &cmd[3..sp];
                skip |= !cond.split(|&c| c == b',').any(|c| c.eq_ignore_ascii_case(b"dos"));
                cmd = trim(&cmd[sp..]);
            } else {
                break;
            }
        }
        print(if skip { b"SYNAGENT: skip " } else { b"SYNAGENT: " });
        println(cmd);
        if !skip {
            chdir(b"C:\\SYNVM\\SETUP");
            let code = shell(cmd, false);
            if code != Some(0) {
                println(b"SYNAGENT: step failed, setup stopped");
                write_all(b"C:\\SYNVM\\STEP.TXT", b"failed");
                return;
            }
        }
        idx += 1;
        let mut o = [0u8; 8];
        let mut w = json::Out::new(&mut o);
        w.num(idx as u64);
        let wn = w.n;
        write_all(b"C:\\SYNVM\\STEP.TXT", &o[..wn]);
        if reboot && !skip {
            println(b"SYNAGENT: reboot");
            outb(0x64, 0xFE);
            loop {}
        }
        i += 1;
    }
    let _ = i;
    delete(&Path::from(b"C:\\SYNVM\\STEP.TXT"));
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let b = s.iter().rposition(|c| !c.is_ascii_whitespace()).map_or(a, |k| k + 1);
    &s[a..b]
}

/// Режим «удалённая работа».
fn remote() {
    uart_init();
    println(b"SYNAGENT: syn-vm agent on COM2 - Esc to exit");
    hello();
    let mut up: Option<Upload> = None;
    loop {
        let mut len = 0;
        if !recv(&mut len) {
            break;
        }
        let body = &rx()[8..8 + len];
        let jl = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
        if jl + 4 > len {
            continue;
        }
        let (j, data) = body[4..].split_at(jl);
        handle(j, data, &mut up);
    }
    if let Some(u) = up {
        close(u.h);
    }
    println(b"SYNAGENT: bye");
}

#[unsafe(no_mangle)]
extern "C" fn main_rs() {
    shrink();
    // Хвост командной строки PSP:81h.
    let p = psp();
    let n = peek8(p, 0x80) as usize;
    let mut tail = [0u8; 127];
    for (i, c) in tail.iter_mut().enumerate().take(n.min(127)) {
        *c = peek8(p, 0x81 + i as u16).to_ascii_uppercase();
    }
    let arg = trim(&tail[..n.min(127)]);
    match arg {
        b"INSTALL" => install(),
        b"SETUP" => setup(),
        _ => remote(),
    }
}
