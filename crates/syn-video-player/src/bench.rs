//! `--bench`: декодирование файла полным ходом (кадры в RGBA, с `--yuv` — в YUV, как при показе) без окна и звука —
//! проверка аппаратного декодера и его скорости.

use std::time::{Duration, Instant};

use syngui::video::{HwAccel, VideoDecoder};

/// `yuv` — кадры в YUV для шейдера (как при показе в окне), иначе swscale в RGBA.
pub fn run(file: &str, hw: HwAccel, yuv: bool) -> i32 {
    let t0 = Instant::now();
    let mut dec = match VideoDecoder::open_with_hwaccel(file, hw) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{file}: {e}");
            return 1;
        }
    };
    // звук не нужен — канал забираем и бросаем, чтобы декодер не ждал его
    drop(dec.take_audio_rx());
    dec.set_yuv_frames(yuv);
    let meta = dec.meta().clone();
    println!("{file}: {}x{} {:.1} с", meta.width, meta.height, meta.duration_sec);
    let cpu0 = cpu_time();
    let mut frames = 0u64;
    let mut first: Option<Duration> = None;
    let mut last_pts = 0.0;
    let mut seek_at: Option<(Instant, u64)> = None;
    loop {
        match dec.try_recv_video() {
            Ok(f) => {
                if first.is_none() {
                    first = Some(t0.elapsed());
                }
                frames += 1;
                last_pts = f.pts_sec;
                // перемотка на середину: проверка сброса декодера
                if frames == 30 && meta.duration_sec > 2.0 {
                    let to = meta.duration_sec / 2.0;
                    println!("перемотка на {to:.2} с с {:.2} с", f.pts_sec);
                    dec.seek(to);
                    seek_at = Some((Instant::now(), f.seek_generation));
                }
                if let Some((t, gen)) = seek_at {
                    if f.seek_generation != gen {
                        println!("после перемотки: кадр {:.2} с через {} мс", f.pts_sec, t.elapsed().as_millis());
                        seek_at = None;
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if dec.reached_eof() {
                    // хвост очереди
                    std::thread::sleep(Duration::from_millis(50));
                    if dec.try_recv_video().is_err() {
                        break;
                    }
                    frames += 1;
                    continue;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    let cpu = cpu_time() - cpu0;
    println!(
        "кадров {frames} (последний pts {last_pts:.2} с) за {wall:.2} с — {:.0} кадр/с; первый кадр через {} мс; процессор {cpu:.2} с ({:.0}% одного ядра)",
        frames as f64 / wall,
        first.map(|d| d.as_millis()).unwrap_or(0),
        cpu / wall * 100.0
    );
    0
}

fn cpu_time() -> f64 {
    // SAFETY: getrusage заполняет структуру целиком.
    let r = unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut r);
        r
    };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(r.ru_utime) + tv(r.ru_stime)
}
