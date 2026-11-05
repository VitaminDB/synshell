//! Точные повреждения кадра GPU: сравнение с прошлым кадром по плиткам — на самом GPU.
//!
//! Клиенты wgpu (все syngui-приложения) при каждом показе помечают изменённой всю поверхность, и чтение
//! кадра с GPU (`offload`) читало весь экран: на E Ink 2480×1860 — 17 МБ, 25–105 мс на кадр, даже когда
//! изменились часы или отрезок пера. Здесь шейдер сравнивает кадр с копией прошлого по плиткам
//! [`TILE`]×[`TILE`] и пишет карту изменений (155×117 пикселей); читается она и затем только изменённые плитки.
//! Копия прошлого кадра обновляется blit'ом тех же плиток.

use smithay::backend::renderer::gles::ffi::{self, types::GLuint, Gles2};
use smithay::utils::{Buffer as BufferCoords, Rectangle, Size};

pub const TILE: i32 = 16;
/// Больше прямоугольников — сливаем ближайшие.
const MAX_RECTS: usize = 24;

const VS: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

const FS: &str = "#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D cur;
uniform highp sampler2D prev;
uniform ivec2 size;
out vec4 o;
void main() {
    ivec2 b = ivec2(gl_FragCoord.xy) * 16;
    ivec2 e = min(b + 16, size);
    for (int y = b.y; y < e.y; y++) {
        for (int x = b.x; x < e.x; x++) {
            if (any(notEqual(texelFetch(cur, ivec2(x, y), 0), texelFetch(prev, ivec2(x, y), 0)))) {
                o = vec4(1.0);
                return;
            }
        }
    }
    o = vec4(0.0);
}
";

pub struct TileDiff {
    size: Size<i32, BufferCoords>,
    tiles: (i32, i32),
    prog: GLuint,
    loc_size: i32,
    prev_tex: GLuint,
    /// Копия текущего кадра: текстуру кадра smithay шейдер на freedreno не читает (нули), blit из неё работает.
    cur_tex: GLuint,
    fbo_cur: GLuint,
    map_tex: GLuint,
    fbo_map: GLuint,
    fbo_read: GLuint,
    fbo_prev: GLuint,
    /// Копия прошлого кадра заполнена.
    valid: bool,
}

unsafe fn compile(gl: &Gles2, kind: u32, src: &str) -> Option<GLuint> {
    let s = gl.CreateShader(kind);
    let ptr = src.as_ptr() as *const ffi::types::GLchar;
    let len = src.len() as i32;
    gl.ShaderSource(s, 1, &ptr, &len);
    gl.CompileShader(s);
    let mut ok = 0;
    gl.GetShaderiv(s, ffi::COMPILE_STATUS, &mut ok);
    if ok == 0 {
        let mut log = vec![0u8; 1024];
        let mut n = 0;
        gl.GetShaderInfoLog(s, log.len() as i32, &mut n, log.as_mut_ptr() as *mut _);
        tracing::warn!(log = %String::from_utf8_lossy(&log[..n.max(0) as usize]), "сравнение кадра: шейдер не собран");
        gl.DeleteShader(s);
        return None;
    }
    Some(s)
}

unsafe fn texture(gl: &Gles2, w: i32, h: i32, bgra: bool) -> GLuint {
    let mut t = 0;
    gl.GenTextures(1, &mut t);
    gl.BindTexture(ffi::TEXTURE_2D, t);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::NEAREST as i32);
    gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::NEAREST as i32);
    // Формат — как у кадра: blit между BGRA и RGBA8 GLES не выполняет.
    let (internal, format) = if bgra { (ffi::BGRA_EXT, ffi::BGRA_EXT) } else { (ffi::RGBA8, ffi::RGBA) };
    gl.TexImage2D(ffi::TEXTURE_2D, 0, internal as i32, w, h, 0, format, ffi::UNSIGNED_BYTE, std::ptr::null());
    t
}

unsafe fn fbo(gl: &Gles2, tex: GLuint) -> GLuint {
    let mut f = 0;
    gl.GenFramebuffers(1, &mut f);
    gl.BindFramebuffer(ffi::FRAMEBUFFER, f);
    gl.FramebufferTexture2D(ffi::FRAMEBUFFER, ffi::COLOR_ATTACHMENT0, ffi::TEXTURE_2D, tex, 0);
    f
}

/// Сохранённое состояние GL (smithay просит возвращать как было).
struct Saved {
    fbo: i32,
    read_fbo: i32,
    prog: i32,
    viewport: [i32; 4],
    active: i32,
    tex0: i32,
    tex1: i32,
    blend: bool,
    scissor: bool,
    vao: i32,
}

unsafe fn save(gl: &Gles2) -> Saved {
    let mut s = Saved { fbo: 0, read_fbo: 0, prog: 0, viewport: [0; 4], active: 0, tex0: 0, tex1: 0, blend: false, scissor: false, vao: 0 };
    gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut s.fbo);
    gl.GetIntegerv(ffi::READ_FRAMEBUFFER_BINDING, &mut s.read_fbo);
    gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut s.prog);
    gl.GetIntegerv(ffi::VIEWPORT, s.viewport.as_mut_ptr());
    gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut s.active);
    gl.ActiveTexture(ffi::TEXTURE0);
    gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut s.tex0);
    gl.ActiveTexture(ffi::TEXTURE1);
    gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut s.tex1);
    s.blend = gl.IsEnabled(ffi::BLEND) != 0;
    s.scissor = gl.IsEnabled(ffi::SCISSOR_TEST) != 0;
    gl.GetIntegerv(ffi::VERTEX_ARRAY_BINDING, &mut s.vao);
    s
}

unsafe fn restore(gl: &Gles2, s: &Saved) {
    gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, s.fbo as u32);
    gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, s.read_fbo as u32);
    gl.UseProgram(s.prog as u32);
    gl.Viewport(s.viewport[0], s.viewport[1], s.viewport[2], s.viewport[3]);
    gl.ActiveTexture(ffi::TEXTURE1);
    gl.BindTexture(ffi::TEXTURE_2D, s.tex1 as u32);
    gl.ActiveTexture(ffi::TEXTURE0);
    gl.BindTexture(ffi::TEXTURE_2D, s.tex0 as u32);
    gl.ActiveTexture(s.active as u32);
    if s.blend {
        gl.Enable(ffi::BLEND);
    }
    if s.scissor {
        gl.Enable(ffi::SCISSOR_TEST);
    }
    gl.BindVertexArray(s.vao as u32);
}

unsafe fn link(gl: &Gles2, fs_src: &str) -> Option<GLuint> {
    let vs = compile(gl, ffi::VERTEX_SHADER, VS)?;
    let fs = compile(gl, ffi::FRAGMENT_SHADER, fs_src)?;
    let prog = gl.CreateProgram();
    gl.AttachShader(prog, vs);
    gl.AttachShader(prog, fs);
    gl.LinkProgram(prog);
    gl.DeleteShader(vs);
    gl.DeleteShader(fs);
    let mut ok = 0;
    gl.GetProgramiv(prog, ffi::LINK_STATUS, &mut ok);
    if ok == 0 {
        tracing::warn!("сравнение кадра: программа не собрана");
        gl.DeleteProgram(prog);
        return None;
    }
    Some(prog)
}

impl TileDiff {
    pub fn new(gl: &Gles2, size: Size<i32, BufferCoords>, bgra: bool) -> Option<Self> {
        unsafe {
            let saved = save(gl);
            let Some(prog) = link(gl, FS) else {
                restore(gl, &saved);
                return None;
            };
            gl.UseProgram(prog);
            gl.Uniform1i(gl.GetUniformLocation(prog, c"cur".as_ptr()), 0);
            gl.Uniform1i(gl.GetUniformLocation(prog, c"prev".as_ptr()), 1);
            let loc_size = gl.GetUniformLocation(prog, c"size".as_ptr());
            let tiles = ((size.w + TILE - 1) / TILE, (size.h + TILE - 1) / TILE);
            let prev_tex = texture(gl, size.w, size.h, bgra);
            let cur_tex = texture(gl, size.w, size.h, bgra);
            let fbo_cur = fbo(gl, cur_tex);
            let map_tex = texture(gl, tiles.0, tiles.1, false);
            let fbo_map = fbo(gl, map_tex);
            let fbo_prev = fbo(gl, prev_tex);
            let mut fbo_read = 0;
            gl.GenFramebuffers(1, &mut fbo_read);
            restore(gl, &saved);
            Some(Self { size, tiles, prog, loc_size, prev_tex, cur_tex, fbo_cur, map_tex, fbo_map, fbo_read, fbo_prev, valid: false })
        }
    }

    /// Освободить объекты GL (кадр вывода пересоздаётся: поворот, режим).
    pub fn release(self, gl: &Gles2) {
        unsafe {
            gl.DeleteFramebuffers(1, &self.fbo_map);
            gl.DeleteFramebuffers(1, &self.fbo_prev);
            gl.DeleteFramebuffers(1, &self.fbo_read);
            gl.DeleteTextures(1, &self.prev_tex);
            gl.DeleteTextures(1, &self.cur_tex);
            gl.DeleteFramebuffers(1, &self.fbo_cur);
            gl.DeleteTextures(1, &self.map_tex);
            gl.DeleteProgram(self.prog);
        }
    }

    /// Изменённые прямоугольники кадра `cur` (текстура размера `size`) против прошлого, в пределах `within`.
    /// `None` — прошлого кадра ещё нет (читать `within` целиком, затем [`TileDiff::remember`]).
    pub fn changed(&mut self, gl: &Gles2, cur: GLuint, within: &[Rectangle<i32, BufferCoords>]) -> Option<Vec<Rectangle<i32, BufferCoords>>> {
        if !self.valid {
            return None;
        }
        let (tw, th) = self.tiles;
        let mut map = vec![0u8; (tw * th * 4) as usize];
        unsafe {
            let saved = save(gl);
            gl.Disable(ffi::BLEND);
            gl.Disable(ffi::SCISSOR_TEST);
            self.blit(gl, cur, self.fbo_cur, &[Rectangle::from_size(self.size)]);
            gl.BindFramebuffer(ffi::FRAMEBUFFER, self.fbo_map);
            gl.Viewport(0, 0, tw, th);
            gl.UseProgram(self.prog);
            gl.Uniform2i(self.loc_size, self.size.w, self.size.h);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.BindTexture(ffi::TEXTURE_2D, self.cur_tex);
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, self.prev_tex);
            gl.BindVertexArray(0);
            gl.DrawArrays(ffi::TRIANGLES, 0, 3);
            gl.ReadPixels(0, 0, tw, th, ffi::RGBA, ffi::UNSIGNED_BYTE, map.as_mut_ptr() as *mut _);
            restore(gl, &saved);
        }
        // Плитки с изменениями, только внутри повреждений клиента.
        let inside = |tx: i32, ty: i32| {
            let t = Rectangle::<i32, BufferCoords>::new((tx * TILE, ty * TILE).into(), (TILE, TILE).into());
            within.iter().any(|w| w.overlaps(t))
        };
        let mut runs: Vec<Rectangle<i32, BufferCoords>> = Vec::new();
        for ty in 0..th {
            let mut tx = 0;
            while tx < tw {
                if map[((ty * tw + tx) * 4) as usize] > 127 && inside(tx, ty) {
                    let x0 = tx;
                    while tx < tw && map[((ty * tw + tx) * 4) as usize] > 127 && inside(tx, ty) {
                        tx += 1;
                    }
                    let r = self.clip(Rectangle::new((x0 * TILE, ty * TILE).into(), ((tx - x0) * TILE, TILE).into()));
                    // Та же полоса в строке выше — продлить вниз.
                    match runs.iter_mut().rev().find(|p| p.loc.x == r.loc.x && p.size.w == r.size.w && p.loc.y + p.size.h == r.loc.y) {
                        Some(p) => p.size.h += r.size.h,
                        None => runs.push(r),
                    }
                } else {
                    tx += 1;
                }
            }
        }
        let n = map.chunks_exact(4).filter(|p| p[0] > 127).count();
        tracing::debug!(target: "synwm::frame", tiles = n, of = tw * th, runs = runs.len(), "сравнение кадра: изменённых плиток");
        Some(merge(runs, MAX_RECTS))
    }

    /// Скопировать области `rects` текстуры `src` в кадровый буфер `dst` (тот же размер).
    unsafe fn blit(&self, gl: &Gles2, src: GLuint, dst: GLuint, rects: &[Rectangle<i32, BufferCoords>]) {
        gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, self.fbo_read);
        gl.FramebufferTexture2D(ffi::READ_FRAMEBUFFER, ffi::COLOR_ATTACHMENT0, ffi::TEXTURE_2D, src, 0);
        gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, dst);
        for r in rects {
            let (x0, y0, x1, y1) = (r.loc.x, r.loc.y, r.loc.x + r.size.w, r.loc.y + r.size.h);
            gl.BlitFramebuffer(x0, y0, x1, y1, x0, y0, x1, y1, ffi::COLOR_BUFFER_BIT, ffi::NEAREST);
        }
        gl.FramebufferTexture2D(ffi::READ_FRAMEBUFFER, ffi::COLOR_ATTACHMENT0, ffi::TEXTURE_2D, 0, 0);
    }

    fn clip(&self, r: Rectangle<i32, BufferCoords>) -> Rectangle<i32, BufferCoords> {
        r.intersection(Rectangle::from_size(self.size)).unwrap_or_default()
    }

    /// Запомнить области `rects` кадра `cur` как прошлый кадр (после чтения).
    pub fn remember(&mut self, gl: &Gles2, cur: GLuint, rects: &[Rectangle<i32, BufferCoords>]) {
        let all = [Rectangle::from_size(self.size)];
        let rects = if self.valid { rects } else { &all[..] };
        unsafe {
            while gl.GetError() != 0 {}
            let saved = save(gl);
            gl.Disable(ffi::SCISSOR_TEST);
            self.blit(gl, cur, self.fbo_prev, rects);
            let err = gl.GetError();
            restore(gl, &saved);
            if err != 0 {
                tracing::warn!(err, "сравнение кадра: копия прошлого кадра не записана");
                self.valid = false;
                return;
            }
        }
        self.valid = true;
    }
}

/// Слить прямоугольники до `max`: каждый раз пара с наименьшим приростом площади.
fn merge(mut rects: Vec<Rectangle<i32, BufferCoords>>, max: usize) -> Vec<Rectangle<i32, BufferCoords>> {
    let area = |r: &Rectangle<i32, BufferCoords>| r.size.w as i64 * r.size.h as i64;
    while rects.len() > max {
        let mut best = (0, 1, i64::MAX);
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let u = rects[i].merge(rects[j]);
                let cost = area(&u) - area(&rects[i]) - area(&rects[j]);
                if cost < best.2 {
                    best = (i, j, cost);
                }
            }
        }
        let b = rects.remove(best.1);
        rects[best.0] = rects[best.0].merge(b);
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_limits_count_and_covers() {
        let rs: Vec<_> = (0..40).map(|i| Rectangle::<i32, BufferCoords>::new((i * 100, 0).into(), (32, 32).into())).collect();
        let m = merge(rs.clone(), 10);
        assert_eq!(m.len(), 10);
        for r in rs {
            assert!(m.iter().any(|x| x.contains_rect(r)));
        }
    }
}
