//! Окна сохранения и управления пресетами раскладок панелей (как в hexspire):
//! - сохранить текущую раскладку как пресет (название + описание);
//! - список пресетов: применить, записать текущий вид, переименовать, дублировать,
//!   удалить (или сбросить встроенный), открыть папку пресетов.

use syngui::prelude::*;
use syngui::widgets::*;

use super::St;
use crate::layout;

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Save,
    Manage,
    Rename(String),
}

pub fn open_save(st: St) {
    st.layout_name.set("Мой вид".to_string());
    st.layout_desc.set(String::new());
    st.layout_modal.set(Some(Mode::Save));
}

pub fn open_manage(st: St) {
    st.layout_modal.set(Some(Mode::Manage));
}

fn open_rename(st: St, key: String, name: String, desc: String) {
    st.layout_name.set(name);
    st.layout_desc.set(desc);
    st.layout_modal.set(Some(Mode::Rename(key)));
}

fn close(st: St) {
    st.layout_modal.set(None);
}

fn bump(st: St) {
    st.layout_rev.set(st.layout_rev.get_untracked().wrapping_add(1));
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let Some(mode) = st.layout_modal.get() else { return Vec::new() };
        let card: Box<dyn Widget> = match mode {
            Mode::Save => Box::new(form_card(st, None)),
            Mode::Rename(key) => Box::new(form_card(st, Some(key))),
            Mode::Manage => Box::new(manage_card(st)),
        };
        vec![Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(GestureDetector::new().on_click(move || close(st)).child(DecoratedBox::new().class("fp-scrim")))
                .child(Center::new().child(EventHook::new().on_key_down(move |k, _| {
                    if k == Key::Escape {
                        close(st);
                        return KeyReply::Handled;
                    }
                    KeyReply::Ignore
                }).child(card))),
        )]
    })
}

fn form_card(st: St, rename_key: Option<String>) -> impl Widget {
    let is_rename = rename_key.is_some();
    let title = if is_rename { "Переименовать пресет вида" } else { "Сохранить вид как пресет" };
    let submit = {
        let rk = rename_key.clone();
        move || {
            let name = st.layout_name.get_untracked();
            let desc = st.layout_desc.get_untracked();
            match &rk {
                Some(key) => match layout::rename(key, &name, &desc) {
                    Ok(()) => {
                        bump(st);
                        st.toast.set(Some(format!("Пресет «{}» обновлён", name.trim())));
                        st.layout_modal.set(Some(Mode::Manage));
                    }
                    Err(e) => st.toast.set(Some(format!("Ошибка: {e}"))),
                },
                None => {
                    let Some(spec) = st.dock.spec() else {
                        st.toast.set(Some("Ошибка: нет открытых панелей".into()));
                        return;
                    };
                    match layout::save_new(&spec, &name, &desc) {
                        Ok(p) => {
                            st.view.set(p.key.clone());
                            layout::save_current(&spec, &p.key);
                            bump(st);
                            st.toast.set(Some(format!("Пресет вида «{}» сохранён", p.name)));
                            close(st);
                        }
                        Err(e) => st.toast.set(Some(format!("Ошибка: {e}"))),
                    }
                }
            }
        }
    };
    let submit_enter = submit.clone();
    let submit_btn = submit.clone();
    let cancel_is_rename = is_rename;
    let cancel = move || {
        if cancel_is_rename {
            st.layout_modal.set(Some(Mode::Manage));
        } else {
            close(st);
        }
    };

    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title).class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));

    let name_field = TextField::with_text(st.layout_name.get_untracked())
        .placeholder("Например: Цветокоррекция")
        .autofocus(true)
        .on_change(move |s: &str| st.layout_name.set(s.to_string()))
        .on_submit(move |_| submit_enter())
        .on_escape(move || close(st));
    let desc_field = TextField::with_text(st.layout_desc.get_untracked())
        .placeholder("Короткое описание (необязательно)")
        .on_change(move |s: &str| st.layout_desc.set(s.to_string()))
        .on_escape(move || close(st));

    let actions = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("Отмена").class("head-btn").on_click(cancel))
        .child(Button::new("Сохранить").class("primary-btn").on_click(submit_btn));

    DecoratedBox::new()
        .class("fp-card")
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(head)
                .child(Text::new("Название").class("lp-label"))
                .child(name_field)
                .child(Text::new("Описание").class("lp-label"))
                .child(desc_field)
                .child(actions),
        )
        .style("width", 440.0f32)
}

fn manage_card(st: St) -> impl Widget {
    let dir = layout::presets_dir();
    let dir_open = dir.clone();
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new("Пресеты видов").class("fp-title"))
        .child(DecoratedBox::new().class("grow"))
        .child(Button::new("+ Сохранить текущий…").class("primary-btn").on_click(move || open_save(st)))
        .child(Button::new("Сбросить раскладку").class("head-btn").on_click(move || {
            st.reset_layout();
            bump(st);
        }))
        .child(Button::new("Папка").class("head-btn").on_click(move || {
            let _ = std::fs::create_dir_all(&dir_open);
            let _ = std::process::Command::new("xdg-open").arg(&dir_open).spawn();
        }))
        .child(ToolButton::new("\u{E5CD}").tooltip("Закрыть (Esc)").on_click(move || close(st)));

    let note = Text::new(format!(
        "Текущая раскладка панелей сохраняется автоматически. Пресеты хранятся в {}.",
        dir.display()
    ))
    .max_lines(2)
    .class("lp-desc");

    let list = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.layout_rev.get();
        let cur = st.view.get();
        let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for p in layout::list() {
            let active = p.key == cur;
            let badge = if active {
                "активен"
            } else if p.modified {
                "встроенный · изменён"
            } else if p.builtin {
                "встроенный"
            } else {
                "свой"
            };
            let (k_apply, k_over, k_ren, k_dup, k_del) = (p.key.clone(), p.key.clone(), p.key.clone(), p.key.clone(), p.key.clone());
            let (n_over, n_ren, d_ren) = (p.name.clone(), p.name.clone(), p.description.clone());
            let can_delete = p.path.is_some();
            let del_label = if p.builtin { "Сбросить" } else { "Удалить" };

            let mut btns = Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    Button::new("Применить")
                        .class(if active { "head-btn" } else { "primary-btn" })
                        .on_click(move || {
                            st.set_view(&k_apply);
                            close(st);
                        }),
                )
                .child(Button::new("Записать текущий").class("head-btn").on_click(move || {
                    let Some(spec) = st.dock.spec() else { return };
                    match layout::overwrite(&k_over, &spec) {
                        Ok(()) => {
                            st.view.set(k_over.clone());
                            layout::save_current(&spec, &k_over);
                            bump(st);
                            st.toast.set(Some(format!("Пресет «{n_over}» перезаписан текущим видом")));
                        }
                        Err(e) => st.toast.set(Some(format!("Ошибка: {e}"))),
                    }
                }))
                .child(Button::new("Переименовать…").class("head-btn").on_click(move || {
                    open_rename(st, k_ren.clone(), n_ren.clone(), d_ren.clone());
                }))
                .child(Button::new("Дублировать").class("head-btn").on_click(move || match layout::duplicate(&k_dup) {
                    Ok(copy) => {
                        bump(st);
                        st.toast.set(Some(format!("Создан пресет «{}»", copy.name)));
                    }
                    Err(e) => st.toast.set(Some(format!("Ошибка: {e}"))),
                }));
            if can_delete {
                let is_builtin = p.builtin;
                let name_del = p.name.clone();
                btns = btns.child(Button::new(del_label).class("head-btn").on_click(move || match layout::delete(&k_del) {
                    Ok(()) => {
                        if is_builtin && st.view.get_untracked() == k_del {
                            st.set_view(&k_del);
                        }
                        bump(st);
                        let msg = if is_builtin {
                            format!("Пресет «{name_del}» сброшен к умолчанию")
                        } else {
                            format!("Пресет «{name_del}» удалён")
                        };
                        st.toast.set(Some(msg));
                    }
                    Err(e) => st.toast.set(Some(format!("Ошибка: {e}"))),
                }));
            }

            let mut item = Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(p.name.clone()).class("lp-name"))
                        .child(DecoratedBox::new().class(if active { "lp-badge lp-badge-on" } else { "lp-badge" }).child(Text::new(badge).class("lp-badge-text"))),
                );
            if !p.description.is_empty() {
                item = item.child(Text::new(p.description.clone()).max_lines(2).class("lp-desc"));
            }
            if let Some(path) = &p.path {
                item = item.child(Text::new(path.display().to_string()).max_lines(1).elide(Elide::Middle).class("lp-path"));
            }
            item = item.child(btns);
            col = col.child(DecoratedBox::new().class(if active { "lp-item lp-item-on" } else { "lp-item" }).child(item));
        }
        vec![Box::new(col)]
    });

    DecoratedBox::new()
        .class("fp-card")
        .child(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(head)
                .child(note)
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().vertical().child(list))),
        )
        .style("width", 660.0f32)
        .style("height", 560.0f32)
}
