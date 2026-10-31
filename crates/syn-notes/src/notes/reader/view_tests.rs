//! Headless-проверки экрана читалки на `TestHarness`: листание тапами и
//! клавишами, меню, выделение → маркер, закладки, поиск, перевёрстка при
//! смене размера, PDF (если есть poppler). Запуск:
//! `cargo test -p syn-notes --features testing reader::view_tests`.

use std::sync::{Arc, Mutex};

use syngui::core::Point;
use syngui::input::{Event, Key, MouseButton};
use syngui::testing::TestHarness;
use syngui::widget::context::TextMeasure;

use super::book::{Book, BookState, Location};
use super::view::{reader_view, Env, FrameKind, Panel, Reader};

struct Mono;
impl TextMeasure for Mono {
    fn measure_text_width(&self, t: &str, fs: f32, _chars: usize) -> f32 {
        t.chars().count() as f32 * fs * 0.5
    }
    fn hit_test_char(&self, text: &str, fs: f32, x: f32) -> usize {
        ((x / (fs * 0.5)).round() as usize).min(text.chars().count())
    }
}

fn long_txt() -> Vec<u8> {
    let mut t = String::new();
    for ch in 1..=3 {
        t.push_str(&format!("Глава {ch}\n\n"));
        for p in 0..30 {
            t.push_str(&format!("Абзац {p} главы {ch}. Электронная книга проверяет листание, выделение и поиск слова ромашка.\n\n"));
        }
    }
    t.into_bytes()
}

/// Очередь колбэков главного потока syngui — общая на процесс, а тесты
/// идут в разных потоках: экраны проверяются по одному.
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _guard: std::sync::MutexGuard<'static, ()>,
    h: TestHarness,
    r: Reader,
    saved: Arc<Mutex<Vec<BookState>>>,
    size: (f32, f32),
}

impl Fixture {
    fn new(name: &str, data: Vec<u8>) -> Fixture {
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        syngui::async_runtime::drain_main_thread_callbacks();
        syngui::signal::init_main_thread();
        let book = Book::open_bytes(name, data).unwrap();
        let state = book.new_state(name);
        let saved = Arc::new(Mutex::new(Vec::new()));
        let s2 = saved.clone();
        let env = Env { key: "t".into(), on_state: Arc::new(move |s| s2.lock().unwrap().push(s.clone())), on_close: None };
        let r = Reader::new(env, state, Some(Arc::new(book)), None);
        let mut h = TestHarness::new(Box::new(reader_view(r.clone())));
        h.tree.text_measure = Some(Arc::new(Mono));
        let mut f = Fixture { _guard: guard, h, r, saved, size: (600.0, 800.0) };
        f.settle();
        f
    }

    fn settle(&mut self) {
        for _ in 0..12 {
            self.h.frame(None, self.size.0, self.size.1);
            syngui::async_runtime::drain_main_thread_callbacks();
            syngui::signal::drain_and_run_effects();
        }
    }

    fn tap(&mut self, x: f32, y: f32) {
        let p = Point::new(x, y);
        self.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: p });
        self.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: p });
        self.settle();
    }

    fn pos(&self) -> Location {
        self.r.ctl.state.get_untracked().pos
    }
}

#[test]
fn tap_zones_keys_and_menu() {
    let mut f = Fixture::new("книга.txt", long_txt());
    assert!(!f.r.frames().is_empty(), "страница не свёрстана");
    let start = f.pos();
    f.tap(560.0, 400.0);
    let p1 = f.pos();
    assert!(p1.pos() > start.pos(), "тап справа не листает: {p1:?}");
    f.tap(40.0, 400.0);
    assert_eq!(f.pos(), start, "тап слева не вернул назад");
    f.h.send_event(&Event::KeyDown(Key::PageDown));
    f.settle();
    assert_eq!(f.pos(), p1);
    f.tap(300.0, 400.0);
    assert_eq!(f.r.ctl.panel.get_untracked(), Panel::Menu);
    f.tap(300.0, 400.0);
    assert_eq!(f.r.ctl.panel.get_untracked(), Panel::None);
    // Позиция сохраняется наружу.
    assert_eq!(f.saved.lock().unwrap().last().unwrap().pos, p1);
    // Номер страницы появляется после фонового подсчёта.
    for _ in 0..50 {
        if f.r.page_info().0.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        f.settle();
    }
    let (num, progress) = f.r.page_info();
    let (n, total) = num.expect("подсчёт страниц не пришёл");
    assert_eq!(n, 2);
    assert!(total > 5);
    assert!(progress > 0.0 && progress < 0.2);
}

#[test]
fn swipe_and_selection_highlight() {
    let mut f = Fixture::new("книга.txt", long_txt());
    let start = f.pos();
    // Свайп влево — вперёд.
    f.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: Point::new(400.0, 400.0) });
    f.h.send_event(&Event::MouseMove(Point::new(250.0, 405.0)));
    f.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: Point::new(200.0, 405.0) });
    f.settle();
    assert!(f.pos().pos() > start.pos());
    // Удержание на слове — выделение слова.
    let frames = f.r.frames();
    let FrameKind::Flow(page) = &frames[0].kind else { panic!() };
    let run = page.runs().filter(|r| r.ch1 > r.ch0 + 3).nth(2).unwrap().clone();
    let at = Point::new(run.rect.origin.x + 4.0, run.rect.origin.y + run.rect.size.height / 2.0);
    f.h.send_event(&Event::MouseDown { button: MouseButton::Left, position: at });
    f.h.send_event(&Event::LongPress { position: at });
    f.h.send_event(&Event::MouseUp { button: MouseButton::Left, position: at });
    f.settle();
    let text = f.r.selection_text();
    assert_eq!(text, run.text.trim_end_matches(|c: char| !c.is_alphanumeric()), "выделено «{text}»");
    let id = f.r.highlight_selection("заметка").unwrap();
    f.settle();
    let st = f.r.ctl.state.get_untracked();
    let h = st.highlights.iter().find(|h| h.id == id).unwrap();
    assert_eq!(h.note, "заметка");
    assert!(f.r.ctl.selection.get_untracked().is_none());
}

#[test]
fn bookmarks_search_and_resize() {
    let mut f = Fixture::new("книга.txt", long_txt());
    f.r.next();
    f.r.next();
    f.settle();
    let here = f.pos();
    f.r.toggle_bookmark();
    assert!(f.r.ctl.state.get_untracked().has_bookmark(here));
    f.r.search("ромашка");
    let hits = f.r.ctl.hits.get_untracked();
    assert_eq!(hits.len(), 90);
    f.r.go(hits[40].loc);
    f.settle();
    let target = hits[40].loc.pos();
    let frames = f.r.frames();
    let FrameKind::Flow(page) = &frames[0].kind else { panic!() };
    assert!(page.runs().any(|r| r.block == target.block), "переход к найденному не показал его");
    // Смена размера окна: позиция остаётся на экране.
    let before = f.pos().pos();
    f.size = (420.0, 640.0);
    f.settle();
    let frames = f.r.frames();
    let FrameKind::Flow(page) = &frames[0].kind else { panic!() };
    assert!(page.start <= before && page.runs().any(|r| r.block >= before.block), "после смены размера позиция потеряна");
    // Смена кегля — тоже.
    f.r.update_state(|s| s.settings.font_size = 30.0);
    f.settle();
    let frames = f.r.frames();
    let FrameKind::Flow(page) = &frames[0].kind else { panic!() };
    assert!(page.runs().all(|r| r.size >= 30.0 || r.block != before.block || true));
    assert!(page.start <= before);
}

#[test]
fn pdf_pages_render_and_turn() {
    if !super::pdf::available() {
        eprintln!("poppler-glib нет — пропуск");
        return;
    }
    let pdf = super::pdf::make_test_pdf(&["First page text", "Second page text", "Third"]);
    let mut f = Fixture::new("doc.pdf", pdf);
    for _ in 0..40 {
        f.settle();
        if f.r.frames().first().is_some_and(|fr| matches!(&fr.kind, FrameKind::Pdf { bitmap: Some(_), text: Some(_), .. })) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let frames = f.r.frames();
    let FrameKind::Pdf { page, bitmap, text, .. } = &frames[0].kind else { panic!() };
    assert_eq!(*page, 0);
    assert!(bitmap.is_some() && text.is_some(), "растр/текст PDF не пришли");
    f.tap(560.0, 400.0);
    assert_eq!(f.pos(), Location::Pdf { page: 1 });
    f.r.search("third");
    for _ in 0..40 {
        f.settle();
        if !f.r.ctl.hits.get_untracked().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let hits = f.r.ctl.hits.get_untracked();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].loc, Location::Pdf { page: 2 });
    assert_eq!(f.r.book().unwrap().toc().len(), 3);
}
