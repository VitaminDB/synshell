//! Клавиатура, комбинации клавиш, мышь и тачпад.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use synshell_common::action::{Action, KeyCombo};
use synshell_common::config::{default_keybindings, AccelProfile};
use syngui::prelude::*;
use syngui::widgets::*;

use crate::op;
use crate::state;
use crate::store;
use crate::sys::{self, XkbData};
use crate::ui::*;

fn xkb() -> &'static XkbData {
    static XKB: OnceLock<XkbData> = OnceLock::new();
    XKB.get_or_init(sys::xkb_data)
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|x| x.trim().to_string()).collect()
}

/// Записать согласованные списки раскладок и вариантов.
fn write_layouts(layouts: &[(String, String)]) {
    let l: Vec<&str> = layouts.iter().map(|(l, _)| l.as_str()).collect();
    let v: Vec<&str> = layouts.iter().map(|(_, v)| v.as_str()).collect();
    set(&op!["input", "keyboard", "layouts"], l.join(","));
    // Варианты пишем, только если хоть один задан — иначе пустая строка.
    let vs = if v.iter().all(|x| x.is_empty()) { String::new() } else { v.join(",") };
    set(&op!["input", "keyboard", "variants"], vs);
    state::bump();
}

pub fn keyboard() -> W {
    let c = store::config();
    let k = &c.input.keyboard;
    let names = split_list(&k.layouts);
    let mut variants = split_list(&k.variants);
    variants.resize(names.len(), String::new());
    let current: Vec<(String, String)> =
        names.into_iter().zip(variants).filter(|(l, _)| !l.is_empty()).collect();
    let data = xkb();

    let mut rows: Vec<W> = Vec::new();
    let n = current.len();
    for (i, (layout, variant)) in current.iter().enumerate() {
        let desc = data.layouts.iter().find(|l| l.name == *layout);
        let title = desc.map(|d| d.description.clone()).unwrap_or_else(|| layout.clone());
        let mut vdd = Dropdown::new().width(260.0).max_height(360.0).item(DropdownItem::new("", t!("Стандартный")));
        if let Some(d) = desc {
            for (vn, vd) in &d.variants {
                vdd = vdd.item(DropdownItem::new(vn.clone(), vd.clone()));
            }
        }
        let (c1, c2, c3, c4) = (current.clone(), current.clone(), current.clone(), current.clone());
        rows.push(row(
            &format!("{}. {}", i + 1, title),
            layout,
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(vdd.selected(variant.clone()).on_change(move |v: &str| {
                    let mut l = c1.clone();
                    l[i].1 = v.to_string();
                    write_layouts(&l);
                }))
                .child(icon_button(icons::UP, move || {
                    if i > 0 {
                        let mut l = c2.clone();
                        l.swap(i, i - 1);
                        write_layouts(&l);
                    }
                }))
                .child(icon_button(icons::DOWN, move || {
                    if i + 1 < n {
                        let mut l = c3.clone();
                        l.swap(i, i + 1);
                        write_layouts(&l);
                    }
                }))
                .child(danger_icon_button(icons::DELETE, move || {
                    let mut l = c4.clone();
                    if l.len() > 1 {
                        l.remove(i);
                        write_layouts(&l);
                    }
                })),
        ));
    }
    let pick = use_signal(String::from("de"));
    let mut add = Dropdown::new().width(300.0).max_height(400.0);
    for l in &data.layouts {
        add = add.item(DropdownItem::new(l.name.clone(), format!("{} ({})", l.description, l.name)));
    }
    let cur_for_add = current.clone();
    rows.push(row(
        &t!("Добавить раскладку"),
        "",
        Row::new()
            .gap(8.0)
            .child(add.selected("de").on_change(move |v: &str| pick.set(v.to_string())))
            .child(primary_button(&t!("Добавить"), move || {
                let mut l = cur_for_add.clone();
                let name = pick.get_untracked();
                if !l.iter().any(|(x, v)| *x == name && v.is_empty()) {
                    l.push((name, String::new()));
                    write_layouts(&l);
                }
            })),
    ));

    // Переключение раскладок — одна опция grp:*, остальные опции не трогаем.
    let opts = split_list(&k.options);
    let grp = opts.iter().find(|o| o.starts_with("grp:")).cloned().unwrap_or_default();
    let mut sw = Dropdown::new().width(320.0).max_height(400.0).item(DropdownItem::new("", t!("Не назначено")));
    for (n, d) in &data.switch_options {
        sw = sw.item(DropdownItem::new(n.clone(), d.clone()));
    }
    let sw = sw.selected(grp).on_change(|v: &str| {
        let mut opts: Vec<String> =
            split_list(&store::config().input.keyboard.options).into_iter().filter(|o| !o.is_empty() && !o.starts_with("grp:")).collect();
        if !v.is_empty() {
            opts.insert(0, v.to_string());
        }
        set(&op!["input", "keyboard", "options"], opts.join(","));
        state::bump();
    });

    page(
        t!("Клавиатура"),
        t!("Раскладки XKB, их переключение и автоповтор."),
        vec![
            group(&t!("Раскладки"), rows),
            group(
                &t!("Переключение"),
                vec![
                    row(&t!("Сочетание для смены раскладки"), "", sw),
                    switch_row(
                        t!("Раскладка для каждого окна"),
                        t!("Окно помнит свою раскладку"),
                        op!["input", "keyboard", "per_window_layout"],
                        k.per_window_layout,
                    ),
                    row_wide(
                        t!("Все опции XKB"),
                        t!("Через запятую: grp:alt_shift_toggle,caps:escape,compose:ralt"),
                        text(op!["input", "keyboard", "options"], &k.options, "", 520.0),
                    ),
                    text_row(t!("Модель клавиатуры"), t!("Обычно пусто"), op!["input", "keyboard", "model"], &k.model, "pc105"),
                ],
            ),
            group(
                &t!("Набор"),
                vec![
                    int_row(t!("Задержка автоповтора"), t!("мс"), op!["input", "keyboard", "repeat_delay"], k.repeat_delay as i64, 100, 2000, 25),
                    int_row(t!("Скорость автоповтора"), t!("нажатий в секунду"), op!["input", "keyboard", "repeat_rate"], k.repeat_rate as i64, 1, 100, 1),
                    switch_row(&t!("NumLock при входе"), "", op!["input", "keyboard", "numlock"], k.numlock),
                ],
            ),
        ],
    )
}

// ─── Комбинации клавиш ──────────────────────────────────────────────────────

/// Человеческое название действия.
pub fn action_label(a: &Action) -> String {
    use synshell_common::action::{Direction, WorkspaceTarget};
    let dir = |d: &Direction| match d {
        Direction::Left => t!("влево"),
        Direction::Right => t!("вправо"),
        Direction::Up => t!("вверх"),
        Direction::Down => t!("вниз"),
    };
    let ws = |t: &WorkspaceTarget| match t {
        WorkspaceTarget::Index(n) => format!("{n}"),
        WorkspaceTarget::Next => t!("следующий").into(),
        WorkspaceTarget::Prev => t!("предыдущий").into(),
        WorkspaceTarget::Last => t!("прошлый").into(),
    };
    match a {
        Action::Spawn(c) => t!("Запустить: {c}", c = c),
        Action::Close => t!("Закрыть окно").into(),
        Action::Kill => t!("Убить процесс окна").into(),
        Action::ToggleFloating => t!("Плавающее/плиточное").into(),
        Action::ToggleFullscreen => t!("Во весь экран").into(),
        Action::ToggleMaximize => t!("Развернуть/восстановить").into(),
        Action::Minimize => t!("Свернуть").into(),
        Action::MinimizeAll => t!("Свернуть все окна").into(),
        Action::ToggleSticky => t!("На всех столах").into(),
        Action::ToggleAlwaysOnTop => t!("Поверх других").into(),
        Action::Snap(d) => t!("Прилепить {v}", v = dir(d)),
        Action::Center => t!("По центру").into(),
        Action::Focus(d) => t!("Фокус {v}", v = dir(d)),
        Action::Move(d) => t!("Сдвинуть окно {v}", v = dir(d)),
        Action::FocusNext => t!("Следующее окно").into(),
        Action::FocusPrev => t!("Предыдущее окно").into(),
        Action::Workspace(t) => t!("Стол {v}", v = ws(t)),
        Action::MoveToWorkspace(t) => t!("Окно на стол {v}", v = ws(t)),
        Action::MoveToWorkspaceFollow(t) => t!("Окно на стол {v} и перейти", v = ws(t)),
        Action::FocusOutput(d) => t!("Монитор {v}", v = dir(d)),
        Action::MoveToOutput(d) => t!("Окно на монитор {v}", v = dir(d)),
        Action::Layout(l) => t!("Раскладка: {l}", l = l.as_str()),
        Action::CycleLayout => t!("Следующая раскладка окон").into(),
        Action::MasterRatio(r) => t!("Мастер-область {r}", r = format!("{:+}", r)),
        Action::MasterCount(c) => t!("Окон в мастере {c}", c = format!("{:+}", c)),
        Action::KeyboardLayoutNext => t!("Следующая раскладка клавиатуры").into(),
        Action::KeyboardLayout(i) => t!("Раскладка клавиатуры {i}", i = i),
        Action::Screenshot => t!("Снимок экрана").into(),
        Action::ScreenshotWindow => t!("Снимок окна").into(),
        Action::ScreenshotInteractive => t!("Снимок с выбором области").into(),
        Action::Overview => t!("Обзор").into(),
        Action::ReloadConfig => t!("Перечитать настройки").into(),
        Action::RestartShell => t!("Перезапустить оболочку").into(),
        Action::Restart => t!("Перезапустить композитор").into(),
        Action::Quit => t!("Выйти из сеанса").into(),
        Action::Lock => t!("Заблокировать").into(),
        Action::Suspend => t!("Сон").into(),
        Action::Reboot => t!("Перезагрузка").into(),
        Action::PowerOff => t!("Выключение").into(),
        Action::PowerOffMonitors => t!("Погасить мониторы").into(),
        Action::ScreenToggle => t!("Погасить и заблокировать / включить экран").into(),
        Action::ScreenOff => t!("Погасить экран и заблокировать").into(),
        Action::ScreenOn => t!("Включить экран").into(),
        Action::ProximityBlank(true) => t!("Погасить экран (у уха)").into(),
        Action::ProximityBlank(false) => t!("Включить экран после «у уха»").into(),
        Action::Shell(c) => t!("Оболочка: {c}", c = c),
        Action::Back => t!("Назад").into(),
        Action::Key(k) => t!("Клавиша {k}", k = k),
        Action::MobileMode(m) => t!("Режим окон: {v}", v = match m {
                synshell_common::action::MobileMode::Pages => t!("страницы"),
                synshell_common::action::MobileMode::Free => t!("свободный"),
            }),
        Action::MobileModeCycle => t!("Следующий режим окон").into(),
        Action::Page(p) => t!("Страница приложений: {p}", p = p),
        Action::CameraHome => t!("Стол — к началу").into(),
        Action::Rotate(r) => t!("Повернуть экран: {v}", v = if *r == synshell_common::action::Rotation::Normal { t!("как обычно") } else { format!("{r}°") }),
        Action::None => t!("Ничего").into(),
    }
}

/// Имя клавиши syngui → имя keysym для конфига.
pub fn key_name(k: Key) -> Option<String> {
    use Key::*;
    let s = match k {
        A => "A", B => "B", C => "C", D => "D", E => "E", F => "F", G => "G", H => "H", I => "I",
        J => "J", K => "K", L => "L", M => "M", N => "N", O => "O", P => "P", Q => "Q", R => "R",
        S => "S", T => "T", U => "U", V => "V", W => "W", X => "X", Y => "Y", Z => "Z",
        Num0 => "0", Num1 => "1", Num2 => "2", Num3 => "3", Num4 => "4", Num5 => "5", Num6 => "6",
        Num7 => "7", Num8 => "8", Num9 => "9",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6", F7 => "F7",
        F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        Escape => "Escape", Enter => "Return", Tab => "Tab", Backspace => "BackSpace",
        Delete => "Delete", Insert => "Insert", Home => "Home", End => "End",
        PageUp => "Page_Up", PageDown => "Page_Down", Left => "Left", Right => "Right",
        Up => "Up", Down => "Down", Space => "Space",
        MediaPlayPause => "XF86AudioPlay", MediaStop => "XF86AudioStop",
        MediaNext => "XF86AudioNext", MediaPrevious => "XF86AudioPrev",
        ContextMenu => "Menu",
        Shift | Ctrl | Alt | Meta | MediaRewind | MediaFastForward | Unknown(_) => return None,
    };
    Some(s.to_string())
}

pub fn combo_from(k: Key, m: Modifiers) -> Option<String> {
    let key = key_name(k)?;
    let mut s = String::new();
    if m.meta {
        s.push_str("Super+");
    }
    if m.ctrl {
        s.push_str("Ctrl+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push_str(&key);
    Some(s)
}

fn norm(c: &str) -> String {
    c.parse::<KeyCombo>().map(|k| k.to_string().to_ascii_lowercase()).unwrap_or_else(|_| c.to_ascii_lowercase())
}

/// Строки таблицы: (сочетание, действие, встроенное, изменённое пользователем).
fn binding_rows() -> Vec<(String, String, bool, bool)> {
    let c = store::config();
    let (resolved, _) = c.resolved_keybindings();
    let builtin: BTreeMap<String, String> =
        default_keybindings().into_iter().map(|(k, a)| (norm(k), a.to_string())).collect();
    let user: BTreeMap<String, (String, Action)> =
        c.keybindings.iter().map(|(k, a)| (norm(k), (k.clone(), a.clone()))).collect();
    let mut out: Vec<(String, String, bool, bool)> = resolved
        .into_iter()
        .map(|(combo, _)| {
            let n = combo.to_string().to_ascii_lowercase();
            let is_builtin = builtin.contains_key(&n);
            let (shown, action) = match user.get(&n) {
                Some((orig, a)) => (orig.clone(), a.to_string()),
                None => (combo.to_string(), builtin.get(&n).cloned().unwrap_or_default()),
            };
            (shown, action, is_builtin, user.contains_key(&n))
        })
        .collect();
    // Снятые встроенные («none») — тоже показываем, чтобы можно было вернуть.
    for (n, (orig, a)) in &user {
        if *a == Action::None && builtin.contains_key(n) {
            out.push((orig.clone(), "none".into(), true, true));
        }
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out
}

/// Поменять сочетание у привязки: старое снять, новое назначить.
fn rebind(old: &str, new: &str, action: &str, old_builtin: bool) {
    if norm(old) == norm(new) {
        return;
    }
    let user = store::config().keybindings;
    if let Some(k) = user.keys().find(|k| norm(k) == norm(old)).cloned() {
        unset(&op!["keybindings", k]);
    }
    if old_builtin {
        set(&op!["keybindings", old], "none");
    }
    set(&op!["keybindings", new], action.to_string());
    state::bump();
}

/// Корневой перехватчик нажатий для режима «нажмите сочетание».
pub fn capture_hook(child: W) -> W {
    let ctx = state::ctx();
    boxed(EventHook::new().on_key_down(move |k, m| {
        let Some(target) = ctx.capture.get_untracked() else { return KeyReply::Ignore };
        if matches!(k, Key::Escape) && !m.ctrl && !m.alt && !m.meta && !m.shift {
            ctx.capture.set(None);
            return KeyReply::Handled;
        }
        let Some(combo) = combo_from(k, m) else { return KeyReply::Handled };
        ctx.capture.set(None);
        if target == "+new" {
            NEW_COMBO.with(|c| {
                if let Some(s) = c.get() {
                    s.set(combo);
                }
            });
        } else if let Some((old, action, builtin)) = decode_target(&target) {
            rebind(&old, &combo, &action, builtin);
        }
        KeyReply::Handled
    }).child(child))
}

thread_local! {
    static NEW_COMBO: std::cell::Cell<Option<RwSignal<String>>> = const { std::cell::Cell::new(None) };
}

fn encode_target(combo: &str, action: &str, builtin: bool) -> String {
    format!("{}\u{1}{}\u{1}{}", combo, action, builtin)
}

fn decode_target(t: &str) -> Option<(String, String, bool)> {
    let mut it = t.split('\u{1}');
    Some((it.next()?.to_string(), it.next()?.to_string(), it.next()? == "true"))
}

fn combo_button(label: String, target: String) -> impl Widget {
    let cap = state::ctx().capture;
    let t2 = target;
    DecoratedBox::new().child(Reactive::new(move || -> Vec<W> {
        let listening = cap.get().as_deref() == Some(t2.as_str());
        let t = t2.clone();
        vec![boxed(
            Button::new(if listening { t!("Нажмите сочетание…").to_string() } else { label.clone() })
                .class(if listening { "btn combo listening" } else { "btn combo" })
                .on_click(move || {
                    cap.set(if cap.get_untracked().as_deref() == Some(t.as_str()) { None } else { Some(t.clone()) })
                }),
        )]
    }))
    .class("combo-wrap")
}

pub fn shortcuts() -> W {
    let search = use_signal(String::new());
    let rev = use_signal(0u64);
    let new_combo = use_signal(String::new());
    NEW_COMBO.with(|c| c.set(Some(new_combo)));
    let new_action = use_signal(String::new());
    let (_, errors) = store::config().resolved_keybindings();

    let list = Reactive::new(move || -> Vec<W> {
        rev.get();
        let q = search.get().to_lowercase();
        let rows = binding_rows();
        let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
        let mut shown = 0;
        for (combo, action, builtin, changed) in rows {
            let parsed: Option<Action> = action.parse().ok();
            let human = parsed.as_ref().map(action_label).unwrap_or_else(|| action.clone());
            if !q.is_empty()
                && !combo.to_lowercase().contains(&q)
                && !action.to_lowercase().contains(&q)
                && !human.to_lowercase().contains(&q)
            {
                continue;
            }
            if shown > 0 {
                col = col.child(DecoratedBox::new().class("row-sep"));
            }
            shown += 1;
            let disabled = action == "none";
            let target = encode_target(&combo, &action, builtin);
            let combo2 = combo.clone();
            let combo3 = combo.clone();
            let mut r = Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class(if disabled { "setting-row kb-row disabled" } else { "setting-row kb-row" })
                .child(combo_button(combo.clone(), target))
                .child(
                    Column::new()
                        .gap(2.0)
                        .class("grow")
                        .child(Text::new(if disabled { t!("Отключено").to_string() } else { human }).class("row-label"))
                        .child(Text::new(if builtin && !changed { t!("встроенное").to_string() } else if builtin { t!("встроенное, изменено").to_string() } else { t!("своё").to_string() }).class("row-hint")),
                )
                .child(
                    TextField::with_text(action.clone()).width(260.0).placeholder(t!("действие")).on_change(move |s| {
                        if s.parse::<Action>().is_ok() {
                            set(&op!["keybindings", combo2.as_str()], s.to_string());
                        }
                    }),
                );
            if changed {
                r = r.child(icon_button(icons::UNDO, move || {
                    // Вернуть встроенное / удалить своё.
                    let user = store::config().keybindings;
                    if let Some(k) = user.keys().find(|k| norm(k) == norm(&combo3)).cloned() {
                        unset(&op!["keybindings", k]);
                    }
                    rev.set(rev.get_untracked() + 1);
                }));
            }
            if !disabled {
                let combo4 = combo.clone();
                r = r.child(danger_icon_button(icons::DELETE, move || {
                    let user = store::config().keybindings;
                    if let Some(k) = user.keys().find(|k| norm(k) == norm(&combo4)).cloned() {
                        unset(&op!["keybindings", k]);
                    }
                    if builtin {
                        set(&op!["keybindings", combo4.as_str()], "none");
                    }
                    rev.set(rev.get_untracked() + 1);
                }));
            }
            col = col.child(r);
        }
        if shown == 0 {
            col = col.child(Text::new(t!("Ничего не найдено")).class("row-hint pad"));
        }
        vec![boxed(col)]
    });

    // Подсказка по действиям — все имена.
    let names = Action::NAMES.join(", ");
    let add_row = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(combo_button_new(new_combo))
        .child(TextField::new().placeholder(t!("или впишите: Super+Shift+X")).width(200.0).on_change(move |s| new_combo.set(s.to_string())))
        .child(TextField::new().placeholder("spawn firefox").width(240.0).on_change(move |s| new_action.set(s.to_string())))
        .child(primary_button(&t!("Добавить"), move || {
            let combo = new_combo.get_untracked();
            let action = new_action.get_untracked();
            match (combo.parse::<KeyCombo>(), action.parse::<Action>()) {
                (Ok(_), Ok(_)) => {
                    set(&op!["keybindings", combo.as_str()], action);
                    rev.set(rev.get_untracked() + 1);
                    state::toast(t!("Сочетание добавлено"));
                }
                (Err(e), _) | (_, Err(e)) => state::toast(t!("Ошибка: {e}", e = e)),
            }
        }));

    let mut body: Vec<W> = vec![
        boxed(
            TextField::new()
                .placeholder(t!("Поиск по сочетанию или действию"))
                .prefix_icon(icons::SEARCH)
                .width(420.0)
                .on_change(move |s| search.set(s.to_string())),
        ),
        group(&t!("Новое сочетание"), vec![boxed(Column::new().gap(8.0).class("setting-row-wide").child(add_row).child(Text::new(t!("Действия: {names}", names = names)).class("row-hint")))]),
    ];
    if !errors.is_empty() {
        body.push(boxed(Text::new(t!("Ошибки: {v}", v = errors.join("; "))).class("error-text")));
    }
    body.push(boxed(list));
    body.push(note(
        &t!("Щёлкните по сочетанию и нажмите новое (Esc — отмена). Клавиши, которых нет на кнопке захвата (Print, XF86…), впишите в поле «Новое сочетание». Сочетание композитора работает, пока окно настроек не в фокусе."),
    ));
    page(t!("Комбинации клавиш"), t!("Встроенные и свои сочетания. Изменения применяются сразу."), body)
}

fn combo_button_new(new_combo: RwSignal<String>) -> impl Widget {
    let cap = state::ctx().capture;
    Reactive::new(move || -> Vec<W> {
        let listening = cap.get().as_deref() == Some("+new");
        let cur = new_combo.get();
        let label = if listening {
            t!("Нажмите сочетание…").to_string()
        } else if cur.is_empty() {
            t!("Записать сочетание").to_string()
        } else {
            cur
        };
        vec![boxed(Button::new(label).class(if listening { "btn combo listening" } else { "btn combo" }).on_click(move || {
            cap.set(if cap.get_untracked().as_deref() == Some("+new") { None } else { Some("+new".into()) })
        }))]
    })
}

// ─── Мышь и тачпад ──────────────────────────────────────────────────────────

fn profile(p: AccelProfile) -> &'static str {
    match p {
        AccelProfile::Adaptive => "adaptive",
        AccelProfile::Flat => "flat",
    }
}

pub fn mouse() -> W {
    let c = store::config();
    let m = &c.input.mouse;
    let t = &c.input.touchpad;
    let prof: &[(&str, &str)] = &[("adaptive", n_!("Адаптивное")), ("flat", n_!("Без ускорения"))];
    page(
        t!("Мышь и тачпад"),
        t!("Скорость указателя, прокрутка, касания."),
        vec![
            group(
                &t!("Мышь"),
                vec![
                    slider_row(&t!("Скорость указателя"), "−1 … 1", op!["input", "mouse", "accel_speed"], m.accel_speed, -1.0, 1.0, 0.05, 2),
                    choice_row(&t!("Ускорение"), "", op!["input", "mouse", "accel_profile"], profile(m.accel_profile), prof),
                    switch_row(t!("Естественная прокрутка"), t!("Содержимое движется за пальцем"), op!["input", "mouse", "natural_scroll"], m.natural_scroll),
                    slider_row(&t!("Скорость прокрутки"), "", op!["input", "mouse", "scroll_factor"], m.scroll_factor, 0.1, 5.0, 0.1, 1),
                    switch_row(t!("Для левой руки"), t!("Поменять кнопки местами"), op!["input", "mouse", "left_handed"], m.left_handed),
                    switch_row(t!("Эмуляция средней кнопки"), t!("Нажатие обеих кнопок"), op!["input", "mouse", "middle_emulation"], m.middle_emulation),
                ],
            ),
            group(
                &t!("Тачпад"),
                vec![
                    switch_row(&t!("Касание — щелчок"), "", op!["input", "touchpad", "tap"], t.tap),
                    switch_row(&t!("Перетаскивание касанием"), "", op!["input", "touchpad", "tap_drag"], t.tap_drag),
                    switch_row(&t!("Естественная прокрутка"), "", op!["input", "touchpad", "natural_scroll"], t.natural_scroll),
                    switch_row(&t!("Отключать при наборе"), "", op!["input", "touchpad", "disable_while_typing"], t.disable_while_typing),
                    slider_row(&t!("Скорость указателя"), "", op!["input", "touchpad", "accel_speed"], t.accel_speed, -1.0, 1.0, 0.05, 2),
                    choice_row(&t!("Ускорение"), "", op!["input", "touchpad", "accel_profile"], profile(t.accel_profile), prof),
                    slider_row(&t!("Скорость прокрутки"), "", op!["input", "touchpad", "scroll_factor"], t.scroll_factor, 0.1, 5.0, 0.1, 1),
                    choice_row(
                        &t!("Прокрутка"),
                        "",
                        op!["input", "touchpad", "scroll_method"],
                        &t.scroll_method,
                        &[("two-finger", n_!("Двумя пальцами")), ("edge", n_!("У края")), ("none", n_!("Нет"))],
                    ),
                    choice_row(
                        &t!("Правый щелчок"),
                        "",
                        op!["input", "touchpad", "click_method"],
                        &t.click_method,
                        &[("clickfinger", n_!("Двумя пальцами")), ("button-areas", n_!("Правый нижний угол"))],
                    ),
                    switch_row(&t!("Для левой руки"), "", op!["input", "touchpad", "left_handed"], t.left_handed),
                    switch_row(&t!("Эмуляция средней кнопки"), "", op!["input", "touchpad", "middle_emulation"], t.middle_emulation),
                ],
            ),
        ],
    )
}
