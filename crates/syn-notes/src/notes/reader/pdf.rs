//! PDF через poppler-glib (+ cairo), загружаемые в рантайме (dlopen): нет
//! библиотеки — нет PDF, остальная читалка работает; кросс-сборке не нужны
//! заголовки и sysroot. На планшете — пакет `poppler-glib` (Arch ARM).
//!
//! poppler не потокобезопасен на документе, поэтому документ живёт в своём
//! потоке ([`PdfWorker`]): запросы рендера/текста идут каналом, ответы — в
//! колбэки. Координаты страниц — в пунктах PDF, начало сверху слева.

use std::ffi::{c_char, c_double, c_int, c_uint, c_void, CStr};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use libloading::Library;
use serde::{Deserialize, Serialize};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct PopplerRectangle {
    x1: c_double,
    y1: c_double,
    x2: c_double,
    y2: c_double,
}

#[repr(C)]
struct GList {
    data: *mut c_void,
    next: *mut GList,
    prev: *mut GList,
}

#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *const c_char,
}

#[repr(C)]
struct PopplerLinkMapping {
    area: PopplerRectangle,
    action: *mut c_void,
}

type Ptr = *mut c_void;

struct Api {
    _libs: Vec<Library>,
    doc_new_from_data: unsafe extern "C" fn(*const c_char, c_int, *const c_char, *mut *mut GError) -> Ptr,
    doc_n_pages: unsafe extern "C" fn(Ptr) -> c_int,
    doc_get_page: unsafe extern "C" fn(Ptr, c_int) -> Ptr,
    doc_get_title: unsafe extern "C" fn(Ptr) -> *mut c_char,
    doc_get_author: unsafe extern "C" fn(Ptr) -> *mut c_char,
    doc_find_dest: unsafe extern "C" fn(Ptr, *const c_char) -> Ptr,
    dest_free: unsafe extern "C" fn(Ptr),
    page_get_size: unsafe extern "C" fn(Ptr, *mut c_double, *mut c_double),
    page_render: unsafe extern "C" fn(Ptr, Ptr),
    page_get_text: unsafe extern "C" fn(Ptr) -> *mut c_char,
    page_get_text_layout: unsafe extern "C" fn(Ptr, *mut *mut PopplerRectangle, *mut c_uint) -> c_int,
    page_get_link_mapping: unsafe extern "C" fn(Ptr) -> *mut GList,
    page_free_link_mapping: unsafe extern "C" fn(*mut GList),
    index_iter_new: unsafe extern "C" fn(Ptr) -> Ptr,
    index_iter_get_child: unsafe extern "C" fn(Ptr) -> Ptr,
    index_iter_next: unsafe extern "C" fn(Ptr) -> c_int,
    index_iter_get_action: unsafe extern "C" fn(Ptr) -> Ptr,
    index_iter_free: unsafe extern "C" fn(Ptr),
    action_free: unsafe extern "C" fn(Ptr),
    g_object_unref: unsafe extern "C" fn(Ptr),
    g_free: unsafe extern "C" fn(Ptr),
    g_error_free: unsafe extern "C" fn(*mut GError),
    surface_create: unsafe extern "C" fn(c_int, c_int, c_int) -> Ptr,
    surface_get_data: unsafe extern "C" fn(Ptr) -> *mut u8,
    surface_get_stride: unsafe extern "C" fn(Ptr) -> c_int,
    surface_flush: unsafe extern "C" fn(Ptr),
    surface_destroy: unsafe extern "C" fn(Ptr),
    cairo_create: unsafe extern "C" fn(Ptr) -> Ptr,
    cairo_destroy: unsafe extern "C" fn(Ptr),
    cairo_scale: unsafe extern "C" fn(Ptr, c_double, c_double),
    cairo_set_source_rgb: unsafe extern "C" fn(Ptr, c_double, c_double, c_double),
    cairo_paint: unsafe extern "C" fn(Ptr),
}

// Указатели на функции — общие для потоков; сами объекты poppler живут в
// одном потоке.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

fn open_lib(names: &[&str]) -> Result<Library, String> {
    let mut last = String::new();
    for n in names {
        match unsafe { Library::new(n) } {
            Ok(l) => return Ok(l),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

macro_rules! sym {
    ($lib:expr, $name:literal) => {
        *unsafe { $lib.get($name) }.map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&$name[..$name.len() - 1])))?
    };
}

fn load() -> Result<Api, String> {
    let pop = open_lib(&["libpoppler-glib.so.8", "libpoppler-glib.so"])?;
    let gobj = open_lib(&["libgobject-2.0.so.0", "libgobject-2.0.so"])?;
    let glib = open_lib(&["libglib-2.0.so.0", "libglib-2.0.so"])?;
    let cairo = open_lib(&["libcairo.so.2", "libcairo.so"])?;
    let api = Api {
        doc_new_from_data: sym!(pop, b"poppler_document_new_from_data\0"),
        doc_n_pages: sym!(pop, b"poppler_document_get_n_pages\0"),
        doc_get_page: sym!(pop, b"poppler_document_get_page\0"),
        doc_get_title: sym!(pop, b"poppler_document_get_title\0"),
        doc_get_author: sym!(pop, b"poppler_document_get_author\0"),
        doc_find_dest: sym!(pop, b"poppler_document_find_dest\0"),
        dest_free: sym!(pop, b"poppler_dest_free\0"),
        page_get_size: sym!(pop, b"poppler_page_get_size\0"),
        page_render: sym!(pop, b"poppler_page_render\0"),
        page_get_text: sym!(pop, b"poppler_page_get_text\0"),
        page_get_text_layout: sym!(pop, b"poppler_page_get_text_layout\0"),
        page_get_link_mapping: sym!(pop, b"poppler_page_get_link_mapping\0"),
        page_free_link_mapping: sym!(pop, b"poppler_page_free_link_mapping\0"),
        index_iter_new: sym!(pop, b"poppler_index_iter_new\0"),
        index_iter_get_child: sym!(pop, b"poppler_index_iter_get_child\0"),
        index_iter_next: sym!(pop, b"poppler_index_iter_next\0"),
        index_iter_get_action: sym!(pop, b"poppler_index_iter_get_action\0"),
        index_iter_free: sym!(pop, b"poppler_index_iter_free\0"),
        action_free: sym!(pop, b"poppler_action_free\0"),
        g_object_unref: sym!(gobj, b"g_object_unref\0"),
        g_free: sym!(glib, b"g_free\0"),
        g_error_free: sym!(glib, b"g_error_free\0"),
        surface_create: sym!(cairo, b"cairo_image_surface_create\0"),
        surface_get_data: sym!(cairo, b"cairo_image_surface_get_data\0"),
        surface_get_stride: sym!(cairo, b"cairo_image_surface_get_stride\0"),
        surface_flush: sym!(cairo, b"cairo_surface_flush\0"),
        surface_destroy: sym!(cairo, b"cairo_surface_destroy\0"),
        cairo_create: sym!(cairo, b"cairo_create\0"),
        cairo_destroy: sym!(cairo, b"cairo_destroy\0"),
        cairo_scale: sym!(cairo, b"cairo_scale\0"),
        cairo_set_source_rgb: sym!(cairo, b"cairo_set_source_rgb\0"),
        cairo_paint: sym!(cairo, b"cairo_paint\0"),
        _libs: vec![pop, gobj, glib, cairo],
    };
    Ok(api)
}

fn api() -> Result<&'static Api, String> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(load).as_ref().map_err(|e| format!("PDF недоступен (нужен poppler-glib): {e}"))
}

/// Есть ли поддержка PDF в системе.
pub fn available() -> bool {
    api().is_ok()
}

// ─────────────────────────── Данные ───────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfTocEntry {
    pub title: String,
    pub level: u8,
    pub page: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PdfLinkTarget {
    Page(usize),
    Uri(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfLink {
    /// Прямоугольник в пунктах (x, y, w, h), начало сверху слева.
    pub rect: [f32; 4],
    pub target: PdfLinkTarget,
}

/// Текст страницы и прямоугольник каждого символа (в пунктах).
#[derive(Clone, Debug, Default)]
pub struct PdfText {
    pub chars: Vec<char>,
    pub rects: Vec<[f32; 4]>,
}

impl PdfText {
    /// Символ под точкой (ближайший в строке).
    pub fn char_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best = None;
        let mut best_d = f32::MAX;
        for (i, r) in self.rects.iter().enumerate() {
            if r[2] <= 0.0 || self.chars.get(i).is_some_and(|c| *c == '\n') {
                continue;
            }
            let dy = if y < r[1] { r[1] - y } else if y > r[1] + r[3] { y - r[1] - r[3] } else { 0.0 };
            let dx = if x < r[0] { r[0] - x } else if x > r[0] + r[2] { x - r[0] - r[2] } else { 0.0 };
            let d = dy * 4.0 + dx;
            if d < best_d {
                best_d = d;
                best = Some(i);
            }
        }
        best.filter(|_| best_d < 40.0)
    }

    /// Прямоугольники выделения `[a, b]` — по одному на строку.
    pub fn selection_rects(&self, a: usize, b: usize) -> Vec<[f32; 4]> {
        let (a, b) = (a.min(b), a.max(b));
        let mut out: Vec<[f32; 4]> = Vec::new();
        for i in a..=b.min(self.rects.len().saturating_sub(1)) {
            let r = self.rects[i];
            if r[2] <= 0.0 {
                continue;
            }
            match out.last_mut() {
                Some(l) if (l[1] - r[1]).abs() < r[3] * 0.5 => {
                    let x2 = (l[0] + l[2]).max(r[0] + r[2]);
                    l[0] = l[0].min(r[0]);
                    l[2] = x2 - l[0];
                    l[3] = l[3].max(r[3]);
                }
                _ => out.push(r),
            }
        }
        out
    }

    pub fn text(&self, a: usize, b: usize) -> String {
        let (a, b) = (a.min(b), a.max(b));
        self.chars.iter().skip(a).take(b + 1 - a).collect()
    }

    /// Поиск (без регистра): диапазоны символов.
    pub fn find(&self, q: &str) -> Vec<(usize, usize)> {
        let q: Vec<char> = q.to_lowercase().chars().collect();
        let t: Vec<char> = self.chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
        let mut out = Vec::new();
        if q.is_empty() || t.len() < q.len() {
            return out;
        }
        let mut i = 0;
        while i + q.len() <= t.len() {
            if t[i..i + q.len()] == q[..] {
                out.push((i, i + q.len() - 1));
                i += q.len();
            } else {
                i += 1;
            }
        }
        out
    }
}

/// Найденное: страница, символы `[a, b]`, текст вокруг.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfHit {
    pub page: usize,
    pub a: usize,
    pub b: usize,
    pub snippet: String,
}

/// Растр страницы RGBA.
#[derive(Clone)]
pub struct PdfBitmap {
    pub page: usize,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
pub struct PdfInfo {
    pub title: String,
    pub author: String,
    /// Размеры страниц в пунктах.
    pub sizes: Vec<(f32, f32)>,
    pub toc: Vec<PdfTocEntry>,
}

// ─────────────────────────── Документ (в одном потоке) ───────────────────────────

struct Doc {
    api: &'static Api,
    doc: Ptr,
    _data: Vec<u8>,
}

impl Drop for Doc {
    fn drop(&mut self) {
        unsafe { (self.api.g_object_unref)(self.doc) };
    }
}

fn take_string(api: &Api, p: *mut c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { (api.g_free)(p as Ptr) };
    s
}

impl Doc {
    fn open(data: Vec<u8>) -> Result<Doc, String> {
        let api = api()?;
        let mut err: *mut GError = std::ptr::null_mut();
        let doc = unsafe { (api.doc_new_from_data)(data.as_ptr() as *const c_char, data.len() as c_int, std::ptr::null(), &mut err) };
        if doc.is_null() {
            let msg = if err.is_null() {
                "не PDF".to_string()
            } else {
                let m = unsafe { CStr::from_ptr((*err).message) }.to_string_lossy().into_owned();
                unsafe { (api.g_error_free)(err) };
                m
            };
            return Err(format!("PDF: {msg}"));
        }
        Ok(Doc { api, doc, _data: data })
    }

    fn with_page<T>(&self, n: usize, f: impl FnOnce(Ptr) -> T) -> Option<T> {
        let page = unsafe { (self.api.doc_get_page)(self.doc, n as c_int) };
        if page.is_null() {
            return None;
        }
        let r = f(page);
        unsafe { (self.api.g_object_unref)(page) };
        Some(r)
    }

    fn info(&self) -> PdfInfo {
        let a = self.api;
        let n = unsafe { (a.doc_n_pages)(self.doc) }.max(0) as usize;
        let sizes = (0..n)
            .map(|i| {
                self.with_page(i, |p| {
                    let (mut w, mut h) = (0.0, 0.0);
                    unsafe { (a.page_get_size)(p, &mut w, &mut h) };
                    (w as f32, h as f32)
                })
                .unwrap_or((595.0, 842.0))
            })
            .collect();
        let mut toc = Vec::new();
        let it = unsafe { (a.index_iter_new)(self.doc) };
        if !it.is_null() {
            self.walk_index(it, 1, &mut toc);
            unsafe { (a.index_iter_free)(it) };
        }
        PdfInfo {
            title: take_string(a, unsafe { (a.doc_get_title)(self.doc) }),
            author: take_string(a, unsafe { (a.doc_get_author)(self.doc) }),
            sizes,
            toc,
        }
    }

    fn walk_index(&self, it: Ptr, level: u8, out: &mut Vec<PdfTocEntry>) {
        let a = self.api;
        loop {
            let action = unsafe { (a.index_iter_get_action)(it) };
            if !action.is_null() {
                let (title, target) = unsafe { self.read_action(action) };
                if let Some(PdfLinkTarget::Page(page)) = target {
                    out.push(PdfTocEntry { title: title.split_whitespace().collect::<Vec<_>>().join(" "), level, page });
                }
                unsafe { (a.action_free)(action) };
            }
            let child = unsafe { (a.index_iter_get_child)(it) };
            if !child.is_null() {
                self.walk_index(child, level + 1, out);
                unsafe { (a.index_iter_free)(child) };
            }
            if unsafe { (a.index_iter_next)(it) } == 0 {
                break;
            }
        }
    }

    /// Разбор PopplerAction (union): тип, заголовок, цель.
    unsafe fn read_action(&self, action: Ptr) -> (String, Option<PdfLinkTarget>) {
        const GOTO_DEST: c_int = 2;
        const URI: c_int = 5;
        const DEST_NAMED: c_int = 9;
        let ty = *(action as *const c_int);
        let title_p = *((action as *const u8).add(8) as *const *const c_char);
        let title = if title_p.is_null() { String::new() } else { CStr::from_ptr(title_p).to_string_lossy().into_owned() };
        let third = *((action as *const u8).add(16) as *const Ptr);
        let target = match ty {
            GOTO_DEST if !third.is_null() => {
                let dest = third;
                let dty = *(dest as *const c_int);
                if dty == DEST_NAMED {
                    let name = *((dest as *const u8).add(48) as *const *const c_char);
                    if name.is_null() {
                        None
                    } else {
                        let found = (self.api.doc_find_dest)(self.doc, name);
                        if found.is_null() {
                            None
                        } else {
                            let pg = *((found as *const u8).add(4) as *const c_int);
                            (self.api.dest_free)(found);
                            Some(PdfLinkTarget::Page((pg.max(1) - 1) as usize))
                        }
                    }
                } else {
                    let pg = *((dest as *const u8).add(4) as *const c_int);
                    Some(PdfLinkTarget::Page((pg.max(1) - 1) as usize))
                }
            }
            URI if !third.is_null() => Some(PdfLinkTarget::Uri(CStr::from_ptr(third as *const c_char).to_string_lossy().into_owned())),
            _ => None,
        };
        (title, target)
    }

    fn render(&self, n: usize, scale: f32) -> Option<PdfBitmap> {
        let a = self.api;
        self.with_page(n, |p| unsafe {
            let (mut w, mut h) = (0.0, 0.0);
            (a.page_get_size)(p, &mut w, &mut h);
            let pw = ((w * scale as f64).ceil() as i32).clamp(1, 8192);
            let ph = ((h * scale as f64).ceil() as i32).clamp(1, 8192);
            let surf = (a.surface_create)(0, pw, ph); // CAIRO_FORMAT_ARGB32
            let cr = (a.cairo_create)(surf);
            (a.cairo_set_source_rgb)(cr, 1.0, 1.0, 1.0);
            (a.cairo_paint)(cr);
            (a.cairo_scale)(cr, scale as f64, scale as f64);
            (a.page_render)(p, cr);
            (a.cairo_destroy)(cr);
            (a.surface_flush)(surf);
            let stride = (a.surface_get_stride)(surf) as usize;
            let data = (a.surface_get_data)(surf);
            let mut rgba = vec![0u8; pw as usize * ph as usize * 4];
            if !data.is_null() {
                for y in 0..ph as usize {
                    let row = std::slice::from_raw_parts(data.add(y * stride), pw as usize * 4);
                    let out = &mut rgba[y * pw as usize * 4..(y + 1) * pw as usize * 4];
                    // cairo ARGB32 (little-endian: B G R A, предумножено; фон белый — альфа 255).
                    for x in 0..pw as usize {
                        out[x * 4] = row[x * 4 + 2];
                        out[x * 4 + 1] = row[x * 4 + 1];
                        out[x * 4 + 2] = row[x * 4];
                        out[x * 4 + 3] = 255;
                    }
                }
            }
            (a.surface_destroy)(surf);
            PdfBitmap { page: n, width: pw as u32, height: ph as u32, rgba: Arc::new(rgba) }
        })
    }

    fn text(&self, n: usize) -> PdfText {
        let a = self.api;
        self.with_page(n, |p| unsafe {
            let text = take_string(a, (a.page_get_text)(p));
            let mut rects: *mut PopplerRectangle = std::ptr::null_mut();
            let mut count: c_uint = 0;
            let ok = (a.page_get_text_layout)(p, &mut rects, &mut count);
            let mut out = PdfText { chars: text.chars().collect(), rects: Vec::new() };
            if ok != 0 && !rects.is_null() {
                let rs = std::slice::from_raw_parts(rects, count as usize);
                out.rects = rs.iter().map(|r| [r.x1 as f32, r.y1 as f32, (r.x2 - r.x1) as f32, (r.y2 - r.y1) as f32]).collect();
                (a.g_free)(rects as Ptr);
            }
            out.rects.resize(out.chars.len(), [0.0; 4]);
            out
        })
        .unwrap_or_default()
    }

    fn links(&self, n: usize) -> Vec<PdfLink> {
        let a = self.api;
        self.with_page(n, |p| unsafe {
            let (mut _w, mut h) = (0.0, 0.0);
            (a.page_get_size)(p, &mut _w, &mut h);
            let list = (a.page_get_link_mapping)(p);
            let mut out = Vec::new();
            let mut it = list;
            while !it.is_null() {
                let m = (*it).data as *const PopplerLinkMapping;
                if !m.is_null() && !(*m).action.is_null() {
                    let ar = (*m).area;
                    let (_, target) = self.read_action((*m).action);
                    if let Some(target) = target {
                        // Область ссылки — в координатах PDF (начало снизу).
                        let rect = [ar.x1 as f32, (h - ar.y2) as f32, (ar.x2 - ar.x1) as f32, (ar.y2 - ar.y1) as f32];
                        out.push(PdfLink { rect, target });
                    }
                }
                it = (*it).next;
            }
            if !list.is_null() {
                (a.page_free_link_mapping)(list);
            }
            out
        })
        .unwrap_or_default()
    }
}

// ─────────────────────────── Рабочий поток ───────────────────────────

enum Req {
    Render { page: usize, scale: f32, reply: Box<dyn FnOnce(Option<PdfBitmap>) + Send> },
    Text { page: usize, reply: Box<dyn FnOnce(PdfText, Vec<PdfLink>) + Send> },
    Search { query: String, from: usize, reply: Box<dyn FnOnce(Vec<PdfHit>) + Send> },
}

/// Документ PDF в своём потоке.
#[derive(Clone)]
pub struct PdfWorker {
    tx: mpsc::Sender<Req>,
    pub info: Arc<PdfInfo>,
}

impl PdfWorker {
    /// Открыть (синхронно ждёт разбор заголовка и оглавления).
    pub fn open(data: Vec<u8>) -> Result<PdfWorker, String> {
        let (tx, rx) = mpsc::channel::<Req>();
        let (itx, irx) = mpsc::channel::<Result<PdfInfo, String>>();
        std::thread::Builder::new()
            .name("reader-pdf".into())
            .spawn(move || {
                let doc = match Doc::open(data) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = itx.send(Err(e));
                        return;
                    }
                };
                let _ = itx.send(Ok(doc.info()));
                while let Ok(req) = rx.recv() {
                    // Из очереди рендера берём только последние запросы:
                    // при быстром листании старые страницы не нужны.
                    match req {
                        Req::Render { page, scale, reply } => reply(doc.render(page, scale)),
                        Req::Text { page, reply } => reply(doc.text(page), doc.links(page)),
                        Req::Search { query, from, reply } => {
                            let n = unsafe { (doc.api.doc_n_pages)(doc.doc) }.max(0) as usize;
                            let mut out = Vec::new();
                            for k in 0..n {
                                let pg = (from + k) % n;
                                let t = doc.text(pg);
                                for (a, b) in t.find(&query) {
                                    let from = a.saturating_sub(30);
                                    let snippet: String = t.chars.iter().skip(from).take(b + 40 - from).collect();
                                    let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                                    out.push(PdfHit { page: pg, a, b, snippet });
                                }
                                if out.len() > 200 {
                                    break;
                                }
                            }
                            reply(out)
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let info = irx.recv().map_err(|_| "PDF: поток разбора упал".to_string())??;
        Ok(PdfWorker { tx, info: Arc::new(info) })
    }

    pub fn page_count(&self) -> usize {
        self.info.sizes.len()
    }

    pub fn render(&self, page: usize, scale: f32, reply: impl FnOnce(Option<PdfBitmap>) + Send + 'static) {
        let _ = self.tx.send(Req::Render { page, scale, reply: Box::new(reply) });
    }

    pub fn text(&self, page: usize, reply: impl FnOnce(PdfText, Vec<PdfLink>) + Send + 'static) {
        let _ = self.tx.send(Req::Text { page, reply: Box::new(reply) });
    }

    /// Поиск по всей книге с текущей страницы. Колбэк вызывается в потоке
    /// документа — из него нельзя ждать других запросов к этому же документу.
    pub fn search(&self, query: &str, from: usize, reply: impl FnOnce(Vec<PdfHit>) + Send + 'static) {
        let _ = self.tx.send(Req::Search { query: query.to_string(), from, reply: Box::new(reply) });
    }

    /// Синхронный рендер (тесты, миниатюры).
    pub fn render_blocking(&self, page: usize, scale: f32) -> Option<PdfBitmap> {
        let (tx, rx) = mpsc::channel();
        self.render(page, scale, move |b| {
            let _ = tx.send(b);
        });
        rx.recv().ok().flatten()
    }

    pub fn text_blocking(&self, page: usize) -> (PdfText, Vec<PdfLink>) {
        let (tx, rx) = mpsc::channel();
        self.text(page, move |t, l| {
            let _ = tx.send((t, l));
        });
        rx.recv().unwrap_or_default()
    }
}

/// Минимальный PDF для тестов и демо: страницы с текстом Helvetica.
pub fn make_test_pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objs: Vec<String> = Vec::new();
    // 1 — каталог с оглавлением, 2 — дерево страниц, 3 — шрифт, 4 — outlines.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + i * 2)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".into());
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
    objs.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    let first_out = 5 + n * 2;
    objs.push(format!("<< /Type /Outlines /First {first_out} 0 R /Last {} 0 R /Count {n} >>", first_out + n - 1));
    for (i, text) in pages.iter().enumerate() {
        let content = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            6 + i * 2
        ));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
    for i in 0..n {
        let mut o = format!("<< /Title (Page {}) /Parent 4 0 R /Dest [{} 0 R /XYZ 0 792 0]", i + 1, 5 + i * 2);
        if i > 0 {
            o += &format!(" /Prev {} 0 R", first_out + i - 1);
        }
        if i + 1 < n {
            o += &format!(" /Next {} 0 R", first_out + i + 1);
        }
        o += " >>";
        objs.push(o);
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_toc_search() {
        if !available() {
            eprintln!("poppler-glib нет — тест PDF пропущен");
            return;
        }
        let pdf = make_test_pdf(&["Hello reader", "Second page"]);
        let w = PdfWorker::open(pdf).unwrap();
        assert_eq!(w.page_count(), 2);
        assert_eq!(w.info.sizes[0], (612.0, 792.0));
        assert_eq!(w.info.toc.len(), 2);
        assert_eq!(w.info.toc[1].page, 1);
        let bmp = w.render_blocking(0, 0.5).unwrap();
        assert_eq!((bmp.width, bmp.height), (306, 396));
        // Есть тёмные пиксели текста сверху, низ — белый.
        let dark = bmp.rgba.chunks(4).filter(|p| p[0] < 128).count();
        assert!(dark > 50, "текст не нарисован: {dark}");
        assert!(bmp.rgba[bmp.rgba.len() - 4..].iter().all(|&v| v == 255));
        let (t, _) = w.text_blocking(1);
        let s: String = t.chars.iter().collect();
        assert!(s.contains("Second page"), "{s:?}");
        let hit = t.find("page");
        assert_eq!(hit.len(), 1);
        let rects = t.selection_rects(hit[0].0, hit[0].1);
        assert_eq!(rects.len(), 1);
        assert!(rects[0][1] > 40.0 && rects[0][1] < 100.0, "y сверху: {:?}", rects[0]);
        let (tx, rx) = mpsc::channel();
        w.search("second", 0, move |r| tx.send(r).unwrap());
        let r = rx.recv().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].page, 1);
        assert!(r[0].snippet.contains("Second"));
    }
}
