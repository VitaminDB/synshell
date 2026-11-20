//! `syn-wine-inspect.exe <X11-id окна>` — запускается тем же Wine (тот же
//! префикс), что и программа. Находит её окно по X11-идентификатору (Wine
//! хранит его в свойстве `__wine_x11_whole_window`) и печатает JSON: дочерние
//! окна (кнопки, поля, списки — у классических программ Win32 и Delphi это
//! настоящие окна), строку меню и открытые меню (`#32768`). Координаты —
//! экранные Wine, они же координаты X-сервера.

#[cfg(not(windows))]
fn main() {
    eprintln!("syn-wine-inspect — программа Windows: запускается через wine");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    let target: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(0);
    match win::run(target) {
        Ok(json) => println!("{json}"),
        Err(e) => {
            println!("{{\"ok\":false,\"error\":\"{}\"}}", win::esc(&e));
            std::process::exit(1);
        }
    }
}

#[cfg(windows)]
mod win {
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub fn esc(s: &str) -> String {
        let mut o = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn class_of(h: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    /// Текст окна; у полей чужого процесса — через WM_GETTEXT (с тайм-аутом).
    fn text_of(h: HWND) -> String {
        let mut buf = vec![0u16; 4096];
        let mut res: usize = 0;
        let ok = unsafe { SendMessageTimeoutW(h, WM_GETTEXT, buf.len() as WPARAM, buf.as_mut_ptr() as LPARAM, SMTO_ABORTIFHUNG, 300, &mut res) };
        if ok != 0 && res > 0 {
            return String::from_utf16_lossy(&buf[..res.min(buf.len())]);
        }
        let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    fn rect_of(h: HWND) -> RECT {
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        unsafe { GetWindowRect(h, &mut r) };
        r
    }

    /// Роль по классу окна (стандартные элементы, Delphi/VCL, MFC).
    fn role_of(h: HWND, class: &str) -> &'static str {
        let c = class.to_lowercase();
        let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
        if c == "button" || c.ends_with("button") || c == "tbutton" || c == "tbitbtn" || c == "tspeedbutton" {
            return match style & 0xF {
                2 | 3 | 5 | 6 => "checkbox",
                4 | 9 => "radiobutton",
                7 => "group",
                _ => "button",
            };
        }
        if c.contains("check") {
            "checkbox"
        } else if c.contains("radio") {
            "radiobutton"
        } else if c.contains("edit") || c.contains("memo") || c.contains("richedit") {
            "textfield"
        } else if c.contains("combo") {
            "combobox"
        } else if c.contains("listview") || c.contains("listbox") || c == "tlistbox" {
            "listbox"
        } else if c.contains("tree") {
            "tree"
        } else if c.contains("tab") {
            "tablist"
        } else if c.contains("status") {
            "statusbar"
        } else if c.contains("toolbar") {
            "toolbar"
        } else if c == "static" || c.contains("label") {
            "statictext"
        } else if c.contains("scrollbar") {
            "scrollbar"
        } else if c.contains("trackbar") || c.contains("slider") {
            "slider"
        } else if c.contains("progress") {
            "progressbar"
        } else {
            "window"
        }
    }

    struct Node {
        role: String,
        label: String,
        shortcut: String,
        class: String,
        rect: RECT,
        depth: u32,
        state: Vec<&'static str>,
    }

    fn json(n: &Node, id: usize) -> String {
        let r = n.rect;
        let mut s = format!(
            "{{\"id\":{id},\"depth\":{},\"role\":\"{}\",\"screen\":[{},{},{},{}],\"class\":\"{}\"",
            n.depth,
            n.role,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            esc(&n.class)
        );
        if !n.label.is_empty() {
            s.push_str(&format!(",\"label\":\"{}\"", esc(&n.label)));
        }
        if !n.shortcut.is_empty() {
            s.push_str(&format!(",\"shortcut\":\"{}\"", esc(&n.shortcut)));
        }
        if !n.state.is_empty() {
            s.push_str(&format!(",\"state\":\"{}\"", n.state.join(" ")));
        }
        s.push('}');
        s
    }

    /// Пункты меню (строки меню окна или открытого меню).
    fn menu_items(owner: HWND, menu: HMENU, depth: u32, out: &mut Vec<Node>) {
        let count = unsafe { GetMenuItemCount(menu) };
        for i in 0..count.max(0) {
            let mut info: MENUITEMINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
            info.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_SUBMENU;
            unsafe { GetMenuItemInfoW(menu, i as u32, 1, &mut info) };
            if info.fType & MFT_SEPARATOR != 0 {
                continue;
            }
            let mut buf = [0u16; 512];
            let n = unsafe { GetMenuStringW(menu, i as u32, buf.as_mut_ptr(), buf.len() as i32, MF_BYPOSITION) };
            let raw = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            let (label, shortcut) = match raw.split_once('\t') {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (raw.clone(), String::new()),
            };
            let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe { GetMenuItemRect(owner, menu, i as u32, &mut rect) };
            let mut state = Vec::new();
            if info.fState & MFS_DISABLED != 0 {
                state.push("disabled");
            }
            if info.fState & MFS_CHECKED != 0 {
                state.push("checked");
            }
            if info.fState & MFS_HILITE != 0 {
                state.push("selected");
            }
            if !info.hSubMenu.is_null() {
                state.push("submenu");
            }
            out.push(Node { role: "menuitem".into(), label: label.replace('&', ""), shortcut, class: String::new(), rect, depth, state });
        }
    }

    struct Ctx {
        target: usize,
        found: HWND,
        popups: Vec<HWND>,
    }

    unsafe extern "system" fn top(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l as *mut Ctx);
        let prop = wide("__wine_x11_whole_window");
        let x = GetPropW(h, prop.as_ptr()) as usize;
        if ctx.target != 0 && x == ctx.target {
            ctx.found = h;
        }
        if class_of(h) == "#32768" && IsWindowVisible(h) != 0 {
            ctx.popups.push(h);
        }
        1
    }

    unsafe extern "system" fn child(h: HWND, l: LPARAM) -> BOOL {
        let v = &mut *(l as *mut Vec<HWND>);
        v.push(h);
        1
    }

    pub fn run(target: usize) -> Result<String, String> {
        let mut ctx = Ctx { target, found: std::ptr::null_mut(), popups: Vec::new() };
        unsafe { EnumWindows(Some(top), &mut ctx as *mut Ctx as LPARAM) };
        let main = ctx.found;
        if main.is_null() {
            return Err(format!("окно X11 {target} не найдено среди окон этого префикса Wine"));
        }
        let mut pid_main = 0u32;
        unsafe { GetWindowThreadProcessId(main, &mut pid_main) };
        let mut nodes: Vec<Node> = Vec::new();
        // Строка меню окна.
        let menu = unsafe { GetMenu(main) };
        if !menu.is_null() {
            menu_items(main, menu, 0, &mut nodes);
        }
        // Дочерние окна: глубина — по цепочке родителей.
        let mut kids: Vec<HWND> = Vec::new();
        unsafe { EnumChildWindows(main, Some(child), &mut kids as *mut Vec<HWND> as LPARAM) };
        for h in kids {
            if unsafe { IsWindowVisible(h) } == 0 {
                continue;
            }
            let mut depth = 0;
            let mut p = unsafe { GetParent(h) };
            while !p.is_null() && p != main && depth < 30 {
                depth += 1;
                p = unsafe { GetParent(p) };
            }
            let class = class_of(h);
            let mut state = Vec::new();
            if unsafe { IsWindowEnabled(h) } == 0 {
                state.push("disabled");
            }
            let role = role_of(h, &class);
            if role == "checkbox" || role == "radiobutton" {
                let st = unsafe { SendMessageW(h, BM_GETCHECK, 0, 0) };
                if st == 1 {
                    state.push("checked");
                }
            }
            nodes.push(Node { role: role.into(), label: text_of(h).replace('&', ""), shortcut: String::new(), class, rect: rect_of(h), depth, state });
        }
        // Открытые меню этого процесса.
        for p in ctx.popups {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(p, &mut pid) };
            if pid != pid_main {
                continue;
            }
            const MN_GETHMENU: u32 = 0x01E1;
            let hm = unsafe { SendMessageW(p, MN_GETHMENU, 0, 0) } as HMENU;
            if !hm.is_null() {
                nodes.push(Node { role: "menu".into(), label: String::new(), shortcut: String::new(), class: "#32768".into(), rect: rect_of(p), depth: 0, state: Vec::new() });
                menu_items(std::ptr::null_mut(), hm, 1, &mut nodes);
            }
        }
        let wr = rect_of(main);
        let mut s = format!(
            "{{\"ok\":true,\"window\":{{\"title\":\"{}\",\"class\":\"{}\",\"screen\":[{},{},{},{}]}},\"nodes\":[",
            esc(&text_of(main)),
            esc(&class_of(main)),
            wr.left,
            wr.top,
            wr.right - wr.left,
            wr.bottom - wr.top
        );
        for (i, n) in nodes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&json(n, i + 1));
        }
        s.push_str("]}");
        Ok(s)
    }
}
