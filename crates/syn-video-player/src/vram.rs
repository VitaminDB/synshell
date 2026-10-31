//! Аппаратный декодер не запустился, а видеопамять держат нейросети synaptixd: спросить, освободить ли
//! её, и по согласию выгрузить модели с видеокарты и открыть ролик заново — уже аппаратно. Спрашиваем
//! один раз за ролик: «нет» — смотрим программно.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syngui::widgets::*;
use synaptixd::{DeviceKind, LoadState, ModelEntry};

use crate::ui::{reopen, St};

type W = Box<dyn Widget>;

/// Меньше этого synaptixd не мешает — не спрашивать.
const MIN_BYTES: u64 = 1 << 30;

#[derive(Clone, Debug, PartialEq)]
pub enum Vram {
    None,
    /// Спросить: модели на видеокарте (id, название) и сколько они занимают.
    Ask { models: Vec<(String, String)>, bytes: u64, reason: String },
    Freeing,
    Failed(String),
}

/// Ролики, про которые уже спросили в этом сеансе.
static ASKED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

fn first_ask(path: &Path) -> bool {
    let mut g = ASKED.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashSet::new).insert(path.to_path_buf())
}

/// Модели synaptixd на видеокарте.
fn cuda_models() -> std::result::Result<Vec<ModelEntry>, String> {
    let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
    let list = c.list().map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .filter(|m| m.state == LoadState::Loaded && m.device == Some(DeviceKind::Cuda) && m.vram_bytes > 0)
        .collect())
}

/// Из потока слежения: декодер перешёл на процессор. Фоновый поток — synaptixd отвечает не мгновенно.
pub fn on_fallback(st: St, path: &Path, reason: String) {
    if !first_ask(path) {
        return;
    }
    let models = match cuda_models() {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!("synaptixd: {e}");
            return;
        }
    };
    // Процесс держит больше, чем веса моделей (KV, пулы CUDA) — по драйверу, если он скажет.
    let bytes = process_vram("synaptixd").unwrap_or_else(|| models.iter().map(|m| m.vram_bytes).sum());
    tracing::info!("аппаратный декодер не пошёл ({reason}); synaptixd держит на видеокарте {} МБ", bytes >> 20);
    if bytes < MIN_BYTES {
        return;
    }
    let models = models.into_iter().map(|m| (m.id, if m.title.is_empty() { String::new() } else { m.title })).collect();
    run_on_main_thread(move || {
        if st.playing.get_untracked().is_some() {
            st.vram.set(Vram::Ask { models, bytes, reason });
        }
    });
}

/// Видеопамять процесса по `nvidia-smi` (байты).
fn process_vram(name: &str) -> Option<u64> {
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-compute-apps=process_name,used_memory", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    let mib: u64 = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.rsplit_once(','))
        .filter(|(p, _)| Path::new(p.trim()).file_name().is_some_and(|f| f == name))
        .filter_map(|(_, m)| m.trim().parse::<u64>().ok())
        .sum();
    (mib > 0).then_some(mib << 20)
}

/// Сколько держит synaptixd на видеокарте (байты) — для подсказки фильтров; `None` — меньше 1 ГБ
/// или службы нет.
pub fn synaptixd_bytes() -> Option<u64> {
    let models = cuda_models().ok()?;
    let bytes = process_vram("synaptixd").unwrap_or_else(|| models.iter().map(|m| m.vram_bytes).sum());
    (bytes >= MIN_BYTES).then_some(bytes)
}

/// Фильтрам не хватило видеопамяти: выгрузить модели synaptixd с видеокарты (в фоне) и вызвать
/// `then` в главном потоке с итогом.
pub fn free_for_filters(then: impl FnOnce(std::result::Result<(), String>) + Send + 'static) {
    std::thread::Builder::new()
        .name("svp-vram".into())
        .spawn(move || {
            let r = (|| -> std::result::Result<(), String> {
                let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
                for m in cuda_models()? {
                    c.unload(&m.id).map_err(|e| format!("{}: {e}", m.id))?;
                }
                Ok(())
            })();
            run_on_main_thread(move || then(r));
        })
        .ok();
}

pub fn gib_text(bytes: u64) -> String {
    gib(bytes)
}

fn free(st: St, models: Vec<(String, String)>) {
    st.vram.set(Vram::Freeing);
    std::thread::Builder::new()
        .name("svp-vram".into())
        .spawn(move || {
            let r = (|| -> std::result::Result<(), String> {
                let mut c = synaptixd::Client::connect().map_err(|e| e.to_string())?;
                for (id, _) in &models {
                    c.unload(id).map_err(|e| format!("{id}: {e}"))?;
                }
                Ok(())
            })();
            run_on_main_thread(move || match r {
                Ok(()) => {
                    st.vram.set(Vram::None);
                    // тот же ролик с того же места — кодек выбирается при открытии
                    reopen(st);
                }
                Err(e) => st.vram.set(Vram::Failed(e)),
            });
        })
        .ok();
}

fn gib(bytes: u64) -> String {
    let g = bytes as f64 / (1u64 << 30) as f64;
    if g >= 10.0 { format!("{g:.0}") } else { format!("{g:.1}").replace('.', ",") }
}

/// Окно вопроса поверх плеера.
pub fn dialog(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let close = move || st.vram.set(Vram::None);
        let (title, body, actions): (String, String, Vec<W>) = match st.vram.get() {
            Vram::None => return vec![],
            Vram::Ask { models, bytes, reason } => {
                let names: Vec<&str> = models.iter().map(|(id, t)| if t.is_empty() { id.as_str() } else { t.as_str() }).collect();
                tracing::debug!("{reason}");
                let names = names.join(", ");
                (
                    t!("Не хватает видеопамяти").to_string(),
                    t!(
                        "Аппаратный декодер не запустился: видеокарта занята. Нейросети synaptixd держат на ней {gb} ГБ ({names}). Сейчас видео декодирует процессор — он греется, а 4K может подтормаживать.\n\nОсвободить видеопамять? Модели загрузятся снова, когда понадобятся.",
                        gb = gib(bytes),
                        names = names
                    )
                    .to_string(),
                    vec![
                        crate::editor::pill("", &t!("Нет"), false, close),
                        crate::editor::pill("", &t!("Освободить"), true, move || free(st, models.clone())),
                    ],
                )
            }
            Vram::Freeing => (t!("Освобождаю видеопамять…").to_string(), t!("Выгружаю модели synaptixd.").to_string(), vec![]),
            Vram::Failed(e) => (
                t!("Не получилось").to_string(),
                t!("synaptixd не выгрузил модели: {e}", e = e).to_string(),
                vec![crate::editor::pill("", &t!("Понятно"), true, close)],
            ),
        };
        let mut col = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(title).class("set-title"))
            .child(Text::new(body).max_lines(12).class("set-row-title"));
        if actions.is_empty() {
            col = col.child(ProgressBar::new().indeterminate().class("ed-progress"));
        } else {
            col = col.child(Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End).children(actions));
        }
        vec![Box::new(
            Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("set-scrim")).child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(GestureDetector::new().on_click(|| {}).child(DecoratedBox::new().child(col).class("ed-dialog"))),
            ),
        )]
    }))
}
