//! Накопители (`synshell_common::drives`): флешки, разделы дисков, телефоны
//! и камеры — перестроить боковую панель, когда их подключают, вынимают,
//! монтируют или извлекают (`/proc/self/mounts` при вставке флешки и
//! монтировании телефона через gvfs не меняется).

pub use synshell_common::drives::*;

pub fn watch() {
    synshell_common::drives::watch(|| {
        syngui::async_runtime::run_on_main_thread(|| {
            if let Some(ctx) = crate::state::try_ctx() {
                ctx.places_rev.update(|r| *r += 1);
            }
        });
    });
}
