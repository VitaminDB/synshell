//! Служебные функции, которые компилятор MSVC-цели зовёт сам (без CRT): 64-битное деление и метка
//! использования плавающей точки. Деление — сдвигами на 32-битных операциях: обычное `/` для u64 здесь
//! снова позвало бы `_aulldiv`.

/// Нужна компоновщику, если в коде есть плавающая точка (у MSVC её даёт CRT).
#[unsafe(no_mangle)]
#[used]
pub static _fltused: i32 = 0x9875;

/// (частное, остаток) беззнакового 64-битного деления.
fn udivmod(n: u64, d: u64) -> (u64, u64) {
    if d == 0 {
        return (u64::MAX, n);
    }
    if d > n {
        return (0, n);
    }
    let (mut q, mut r) = (0u64, 0u64);
    let mut i = 64u32;
    while i > 0 {
        i -= 1;
        r = (r << 1) | ((n >> i) & 1);
        if r >= d {
            r -= d;
            q |= 1u64 << i;
        }
    }
    (q, r)
}

// Имена MSVC (`__aulldiv` и др., без декорации stdcall) — переходами из ассемблера: export_name с «\x01»
// rustc в исполняемом файле всё равно декорирует (`___alldiv@16`).
core::arch::global_asm!(
    ".globl __aulldiv", "__aulldiv: jmp {d}",
    ".globl __aullrem", "__aullrem: jmp {r}",
    ".globl __alldiv", "__alldiv: jmp {sd}",
    ".globl __allrem", "__allrem: jmp {sr}",
    d = sym aulldiv, r = sym aullrem, sd = sym alldiv, sr = sym allrem,
);

/// `_aulldiv`: два u64 в стеке, вызываемый снимает 16 байт, результат в EDX:EAX — это stdcall.
extern "stdcall" fn aulldiv(n: u64, d: u64) -> u64 {
    udivmod(n, d).0
}

extern "stdcall" fn aullrem(n: u64, d: u64) -> u64 {
    udivmod(n, d).1
}

/// Знаковые варианты (`_alldiv`, `_allrem`).
extern "stdcall" fn alldiv(n: i64, d: i64) -> i64 {
    let q = udivmod(n.unsigned_abs(), d.unsigned_abs()).0 as i64;
    if (n < 0) != (d < 0) { q.wrapping_neg() } else { q }
}

extern "stdcall" fn allrem(n: i64, d: i64) -> i64 {
    let r = udivmod(n.unsigned_abs(), d.unsigned_abs()).1 as i64;
    if n < 0 { r.wrapping_neg() } else { r }
}

// `_chkstk` (= `_alloca_probe`): EAX — сколько байт нужно кадру; тронуть каждую страницу сверху вниз (в стеке
// Windows ниже — страница-сторож, её нельзя перепрыгнуть) и вернуться с ESP, опущенным на EAX.
core::arch::global_asm!(
    ".globl __chkstk",
    "__chkstk:",
    "push ecx",
    "lea ecx, [esp + 8]",
    "cmp eax, 0x1000",
    "jb 2f",
    "1:",
    "sub ecx, 0x1000",
    "test [ecx], eax",
    "sub eax, 0x1000",
    "cmp eax, 0x1000",
    "jae 1b",
    "2:",
    "sub ecx, eax",
    "test [ecx], eax",
    "lea eax, [esp + 4]",
    "mov esp, ecx",
    "mov ecx, [eax - 4]",
    "push dword ptr [eax]",
    "ret",
);
