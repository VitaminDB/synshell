//! Сочетания клавиш: разбор имён клавиш в keysym (xkb) и поиск действия.
//!
//! Сопоставление идёт по «латинскому» keysym клавиши
//! (`raw_latin_sym_or_raw_current_sym`), поэтому `Super+Q` работает и в
//! русской раскладке, а `Super+Shift+1` — это «1» с Shift, а не «!».

use smithay::input::keyboard::{xkb, Keysym, ModifiersState};
use syndesktop_common::{
    action::{Action, KeyCombo, Mods},
    Config,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Key(Keysym),
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub mods: Mods,
    pub trigger: Trigger,
    pub action: Action,
    /// Как записано в конфиге — для сообщений.
    pub combo: String,
}

#[derive(Debug, Default)]
pub struct Bindings {
    pub list: Vec<Binding>,
    /// Модификатор для перетаскивания окон мышью.
    pub mod_key: Mods,
}

impl Bindings {
    pub fn from_config(config: &Config) -> (Self, Vec<String>) {
        let (combos, mut errors) = config.resolved_keybindings();
        let mut list = Vec::with_capacity(combos.len());
        for (combo, action) in combos {
            match parse_trigger(&combo) {
                Some(trigger) => list.push(Binding {
                    mods: combo.mods,
                    trigger,
                    action,
                    combo: combo.to_string(),
                }),
                None => errors.push(format!("неизвестная клавиша «{}» в «{combo}»", combo.key)),
            }
        }
        let mod_key = Mods::parse(&config.windows.mod_key).unwrap_or(Mods::SUPER);
        (Self { list, mod_key }, errors)
    }

    pub fn find(&self, mods: Mods, trigger: Trigger) -> Option<&Binding> {
        self.list.iter().find(|b| b.mods == mods && b.trigger == trigger)
    }
}

fn parse_trigger(combo: &KeyCombo) -> Option<Trigger> {
    let k = combo.key.as_str();
    match k.to_ascii_lowercase().as_str() {
        "wheelup" | "wheel_up" | "scrollup" => return Some(Trigger::WheelUp),
        "wheeldown" | "wheel_down" | "scrolldown" => return Some(Trigger::WheelDown),
        "wheelleft" => return Some(Trigger::WheelLeft),
        "wheelright" => return Some(Trigger::WheelRight),
        _ => {}
    }
    keysym_from_name(k).map(Trigger::Key)
}

/// Имя клавиши → keysym. Однобуквенные имена приводятся к нижнему регистру
/// (с Shift keysym латинской клавиши всё равно строчный).
pub fn keysym_from_name(name: &str) -> Option<Keysym> {
    let alias = match name.to_ascii_lowercase().as_str() {
        "enter" => "Return",
        "esc" => "Escape",
        "del" => "Delete",
        "ins" => "Insert",
        "pgup" | "pageup" => "Page_Up",
        "pgdn" | "pagedown" => "Page_Down",
        "space" => "space",
        "plus" => "plus",
        "minus" => "minus",
        "equal" | "equals" => "equal",
        "comma" => "comma",
        "period" | "dot" => "period",
        "slash" => "slash",
        "backslash" => "backslash",
        "semicolon" => "semicolon",
        "apostrophe" => "apostrophe",
        "grave" | "backtick" => "grave",
        "bracketleft" => "bracketleft",
        "bracketright" => "bracketright",
        "printscreen" | "print" => "Print",
        _ => "",
    };
    let name = if alias.is_empty() { name } else { alias };
    let lowered;
    let name = if name.chars().count() == 1 {
        lowered = name.to_lowercase();
        lowered.as_str()
    } else {
        name
    };
    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if sym.raw() != 0 {
        return Some(sym);
    }
    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    (sym.raw() != 0).then_some(sym)
}

pub fn mods_from_state(m: &ModifiersState) -> Mods {
    let mut out = Mods::empty();
    if m.logo {
        out |= Mods::SUPER;
    }
    if m.ctrl {
        out |= Mods::CTRL;
    }
    if m.alt {
        out |= Mods::ALT;
    }
    if m.shift {
        out |= Mods::SHIFT;
    }
    out
}

/// Keysym — сам по себе модификатор (для завершения Alt+Tab по отпусканию).
pub fn is_modifier(sym: Keysym) -> bool {
    matches!(
        sym,
        Keysym::Alt_L
            | Keysym::Alt_R
            | Keysym::Super_L
            | Keysym::Super_R
            | Keysym::Control_L
            | Keysym::Control_R
            | Keysym::Shift_L
            | Keysym::Shift_R
            | Keysym::Meta_L
            | Keysym::Meta_R
            | Keysym::ISO_Level3_Shift
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(keysym_from_name("Return"), Some(Keysym::Return));
        assert_eq!(keysym_from_name("Q"), Some(Keysym::q));
        assert_eq!(keysym_from_name("1"), Some(Keysym::_1));
        assert_eq!(keysym_from_name("XF86AudioRaiseVolume"), Some(Keysym::XF86_AudioRaiseVolume));
        assert_eq!(keysym_from_name("Print"), Some(Keysym::Print));
        assert!(keysym_from_name("NoSuchKey").is_none());
    }

    #[test]
    fn defaults_resolve() {
        let (b, errors) = Bindings::from_config(&Config::default());
        assert!(errors.is_empty(), "{errors:?}");
        assert!(b.find(Mods::SUPER, Trigger::Key(Keysym::Return)).is_some());
        assert!(b.find(Mods::SUPER, Trigger::WheelDown).is_some());
    }
}
