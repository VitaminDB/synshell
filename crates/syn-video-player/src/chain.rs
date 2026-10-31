//! Цепочка фильтров кадров: у плеера syngui одно место под фильтр, а фильтров несколько — DLSS 5,
//! увеличение разрешения, интерполяция кадров. Цепочка — один [`FrameFilter`], ступени — по порядку:
//! улучшение на исходном разрешении (дешевле), затем увеличение, затем промежуточные кадры.

use std::sync::{Arc, RwLock};

use syngui::video::{FrameFilter, VideoFrame, VideoPlayer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Dlss = 0,
    Upscale = 1,
    Interpolate = 2,
}

const STAGES: usize = 3;

pub struct Chain {
    stages: RwLock<[Option<Arc<dyn FrameFilter>>; STAGES]>,
}

static CHAIN: std::sync::OnceLock<Arc<Chain>> = std::sync::OnceLock::new();

fn chain() -> Arc<Chain> {
    CHAIN.get_or_init(|| Arc::new(Chain { stages: RwLock::new(Default::default()) })).clone()
}

/// Поставить или снять ступень и обновить фильтр плеера: ступеней нет — фильтра нет (кадры снова
/// идут прямо, с YUV на GPU).
pub fn set(p: &VideoPlayer, stage: Stage, f: Option<Arc<dyn FrameFilter>>) {
    let c = chain();
    let any = {
        let mut s = c.stages.write().unwrap();
        s[stage as usize] = f;
        s.iter().any(Option::is_some)
    };
    p.set_frame_filter(if any { Some(c as Arc<dyn FrameFilter>) } else { None });
}

/// Стоит ли ступень.
pub fn has(stage: Stage) -> bool {
    chain().stages.read().unwrap()[stage as usize].is_some()
}

fn run(stages: &[Arc<dyn FrameFilter>], f: VideoFrame, emit: &mut dyn FnMut(VideoFrame)) {
    match stages.split_first() {
        None => emit(f),
        Some((first, rest)) => first.process(f, &mut |o| run(rest, o, emit)),
    }
}

impl FrameFilter for Chain {
    fn apply(&self, frame: &mut VideoFrame) {
        // один кадр на входе — один на выходе: берём последний, если ступени дали несколько
        let mut last = None;
        self.process(frame.clone(), &mut |o| last = Some(o));
        if let Some(o) = last {
            *frame = o;
        }
    }

    fn process(&self, frame: VideoFrame, emit: &mut dyn FnMut(VideoFrame)) {
        let stages: Vec<Arc<dyn FrameFilter>> = self.stages.read().unwrap().iter().flatten().cloned().collect();
        run(&stages, frame, emit);
    }
}
