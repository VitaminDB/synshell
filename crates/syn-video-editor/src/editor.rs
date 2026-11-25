//! Состояние редактора без окна: проект, путь файла, история отмены,
//! выделение, позиция воспроизведения. Его меняют только команды
//! ([`crate::cmd`]) — одинаково из интерфейса, CLI, сокета и агента.

use std::path::PathBuf;

use crate::doc::{Id, Project};

const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Debug)]
struct Step {
    label: String,
    before: Project,
}

#[derive(Default)]
pub struct Editor {
    pub project: Project,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selection: Vec<Id>,
    pub playhead: f64,
    /// Скопированные клипы: (дорожка, клип) — для `clip.paste`.
    pub clipboard: Vec<(Id, crate::doc::Clip)>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Открытая группа отмены (ход агента): изменения внутри — один шаг.
    group: Option<String>,
    group_saved: bool,
    /// Счётчик правок — интерфейс перестраивается по нему.
    pub revision: u64,
    /// Последний шаг вне группы и когда: правки одного поля подряд (ползунок
    /// инспектора) склеиваются в один шаг отмены.
    last: Option<(String, std::time::Instant)>,
}

/// Подписи шагов, которые склеиваются, если идут подряд быстрее секунды.
const COALESCE: &[&str] = &["Свойства клипа", "Параметры эффекта", "Параметры проекта", "Переход"];

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Начать изменение проекта: снимок для отмены (внутри группы — один на группу).
    pub fn begin(&mut self, label: &str) {
        if self.group.is_some() {
            if !self.group_saved {
                let label = self.group.clone().unwrap_or_default();
                self.push(label);
                self.group_saved = true;
            }
        } else {
            let now = std::time::Instant::now();
            let glue = COALESCE.contains(&label) && self.redo.is_empty() && self.last.as_ref().is_some_and(|(l, t)| l == label && now.duration_since(*t).as_secs_f32() < 1.0) && self.undo.last().is_some_and(|s| s.label == label);
            if !glue {
                self.push(label.to_string());
            }
            self.last = Some((label.to_string(), now));
        }
        self.dirty = true;
        self.revision += 1;
    }

    /// Число шагов отмены (для тестов).
    #[cfg(test)]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    fn push(&mut self, label: String) {
        self.undo.push(Step { label, before: self.project.clone() });
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Группа отмены: всё до [`Editor::end_group`] отменяется одним шагом.
    pub fn begin_group(&mut self, label: &str) {
        self.group = Some(label.to_string());
        self.group_saved = false;
    }

    pub fn end_group(&mut self) {
        self.group = None;
        self.group_saved = false;
    }

    /// Проект до начала открытой группы (для разницы «до/после»).
    pub fn group_before(&self) -> Option<&Project> {
        (self.group.is_some() && self.group_saved).then(|| self.undo.last().map(|s| &s.before)).flatten()
    }

    /// Отменить изменение, вернуть его подпись.
    pub fn undo(&mut self) -> Option<String> {
        let step = self.undo.pop()?;
        let after = std::mem::replace(&mut self.project, step.before);
        self.redo.push(Step { label: step.label.clone(), before: after });
        self.after_history();
        Some(step.label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let step = self.redo.pop()?;
        let before = std::mem::replace(&mut self.project, step.before);
        self.undo.push(Step { label: step.label.clone(), before });
        self.after_history();
        Some(step.label)
    }

    fn after_history(&mut self) {
        self.dirty = true;
        self.revision += 1;
        let p = &self.project;
        self.selection.retain(|id| p.find_clip(*id).is_some());
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }

    /// Новый проект или открытый файл: история начинается заново.
    pub fn replace(&mut self, project: Project, path: Option<PathBuf>) {
        self.project = project;
        self.path = path;
        self.dirty = false;
        self.selection.clear();
        self.playhead = 0.0;
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
    }
}
