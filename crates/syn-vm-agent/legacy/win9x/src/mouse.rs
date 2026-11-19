//! Мышь Windows 9x без захвата: координаты курсора хоста из vmmouse (бэкдор VMware в QEMU, порт 5658h — как у
//! SYNMOUSE.COM в DOS) → SetCursorPos, кнопки → mouse_event. У 9x нет абсолютной мыши (планшет USB 95 не знает), а
//! PS/2 даёт только сдвиги — окно syn-vm приходилось захватывать. Абсолютный режим vmmouse делает его активной мышью
//! QEMU, и syn-vm (query-mice) захват снимает сам. Игра заперла курсор (ClipCursor — DirectInput и полноэкранные
//! игры) — vmmouse выключается, снова работает PS/2 со сдвигами, и syn-vm захватывает мышь для игры.
use crate::win::*;
use core::arch::asm;

const VM_MAGIC: u32 = 0x564D_5868;
const VM_PORT: u32 = 0x5658;
const CMD_GETVERSION: u32 = 10;
const CMD_DATA: u32 = 39;
const CMD_STATUS: u32 = 40;
const CMD_COMMAND: u32 = 41;
const READ_ID: u32 = 0x4541_4552;
const REQ_ABSOLUTE: u32 = 0x5342_4152;
const DISABLE: u32 = 0x0000_00f5;

/// Дескриптор SYNGPU.VXD, если бэкдор идёт через него (Windows 95: IN из кольца 3 VMM выполняет сам, без
/// «волшебного» EAX, — QEMU такой запрос не узнаёт; VxD драйвера экрана syn-gpu делает IN из кольца 0).
static mut VXD: HANDLE = 0;
const OP_SYNGPU_BACKDOOR: u32 = 0x5003;

/// in eax, dx с EBX = arg, ECX = cmd → (eax, ebx, ecx, edx).
fn backdoor(cmd: u32, arg: u32) -> (u32, u32, u32, u32) {
    let h = unsafe { VXD };
    if h != 0 {
        let inb = [arg, cmd];
        let mut out = [0u32; 4];
        let mut ret = 0u32;
        unsafe { DeviceIoControl(h, OP_SYNGPU_BACKDOOR, inb.as_ptr().cast(), 8, out.as_mut_ptr().cast(), 16, &mut ret, core::ptr::null_mut()) };
        return (out[0], out[1], out[2], out[3]);
    }
    backdoor_ring3(cmd, arg)
}

fn backdoor_ring3(cmd: u32, arg: u32) -> (u32, u32, u32, u32) {
    let (a, b, c, d): (u32, u32, u32, u32);
    unsafe {
        asm!(
            "xchg ebx, edi",
            "in eax, dx",
            "xchg ebx, edi",
            inout("eax") VM_MAGIC => a,
            inout("edi") arg => b,
            inout("ecx") cmd => c,
            inout("edx") VM_PORT => d,
            options(nostack)
        );
    }
    (a, b, c, d)
}

fn enable() -> bool {
    backdoor(CMD_COMMAND, READ_ID);
    let (st, ..) = backdoor(CMD_STATUS, 0);
    if st & 0xFFFF == 0 || st == 0xFFFF_0000 {
        return false;
    }
    let (id, ..) = backdoor(CMD_DATA, 1);
    if id != 0x3442_554a {
        return false;
    }
    backdoor(CMD_COMMAND, REQ_ABSOLUTE);
    true
}

/// Курсор заперт игрой в прямоугольник меньше экрана.
fn clipped(sw: i32, sh: i32) -> bool {
    let mut r = [0i32; 4];
    let ok = unsafe { GetClipCursor(r.as_mut_ptr()) } != 0;
    ok && (r[0] > 0 || r[1] > 0 || r[2] < sw || r[3] < sh)
}

pub unsafe extern "system" fn thread(_: *mut u8) -> u32 {
    if is_nt() {
        return 0;
    }
    // Sleep(1) у 95 — 55 мс без этого.
    unsafe { timeBeginPeriod(1) };
    let (_, mut b, _, _) = backdoor(CMD_GETVERSION, 0);
    if b != VM_MAGIC {
        // Windows 95: через VxD драйвера экрана syn-gpu.
        let h = unsafe { CreateFileA(b"\\\\.\\SYNGPU.VXD\0".as_ptr(), 0, 0, core::ptr::null(), OPEN_EXISTING, 0x0400_0000, 0) };
        if h != INVALID {
            unsafe { VXD = h };
            b = backdoor(CMD_GETVERSION, 0).1;
            if b != VM_MAGIC {
                unsafe { VXD = 0 };
            } else {
                crate::log("мышь: бэкдор через SYNGPU.VXD");
            }
        }
    }
    if b != VM_MAGIC {
        crate::log("мышь: бэкдора VMware нет — курсор захватывается (PS/2)");
        return 0;
    }
    let mut on = false;
    let mut buttons = 0u32;
    let mut last = (u32::MAX, u32::MAX);
    loop {
        let (sw, sh) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
        let want = !clipped(sw, sh);
        if want && !on {
            on = enable();
            last = (u32::MAX, u32::MAX);
            crate::log(if on { "мышь: vmmouse — курсор хоста без захвата" } else { "мышь: vmmouse не включилась" });
        } else if !want && on {
            backdoor(CMD_COMMAND, DISABLE);
            on = false;
            crate::log("мышь: курсор заперт игрой — PS/2 (захват)");
        }
        if on {
            for _ in 0..32 {
                let (st, ..) = backdoor(CMD_STATUS, 0);
                if st == 0xFFFF_0000 {
                    on = false;
                    break;
                }
                if st & 0xFFFF < 4 {
                    break;
                }
                let (flags, ax, ay, _z) = backdoor(CMD_DATA, 4);
                let (ax, ay) = (ax & 0xFFFF, ay & 0xFFFF);
                // Позиция — на каждом пакете, а не только при смене: на каждое событие vmmouse QEMU шлёт и в PS/2
                // «пустой» пакет со сдвигом dx=1 (i8042 fake event), курсор Windows уползает — без возврата щелчки
                // расходятся дальше допуска двойного щелчка.
                if flags & 0x0001_0000 == 0 {
                    last = (ax, ay);
                }
                if last != (u32::MAX, u32::MAX) {
                    let x = ((last.0 as i64 * sw as i64) >> 16) as i32;
                    let y = ((last.1 as i64 * sh as i64) >> 16) as i32;
                    unsafe { SetCursorPos(x, y) };
                }
                // Кнопки vmmouse: 20h — левая, 10h — правая, 08h — средняя.
                let b = (flags & 0x20 != 0) as u32 | ((flags & 0x10 != 0) as u32) << 1 | ((flags & 0x08 != 0) as u32) << 2;
                let ch = b ^ buttons;
                buttons = b;
                // MOUSEEVENTF_LEFTDOWN/UP 2/4, RIGHTDOWN/UP 8/10h, MIDDLEDOWN/UP 20h/40h.
                for (i, (down, up)) in [(2u32, 4u32), (8, 0x10), (0x20, 0x40)].iter().enumerate() {
                    if ch & (1 << i) != 0 {
                        unsafe { mouse_event(if b & (1 << i) != 0 { *down } else { *up }, 0, 0, 0, 0) };
                    }
                }
            }
        }
        unsafe { Sleep(if on { 2 } else { 100 }) };
    }
}
