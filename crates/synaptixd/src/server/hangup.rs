//! Сторож разрыва: клиент отменяет запрос, закрывая сокет (`Canceller`), а
//! служба узнаёт об этом, только когда пишет ответ. Долгий префилл или
//! стадия без событий так и досчитывались до конца, держа видеокарту.
//! Сторож опрашивает сокет на POLLRDHUP и зовёт `on_gone` один раз.

use std::os::fd::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct Watch {
    stop: Arc<AtomicBool>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Клиент закрыл свою сторону сокета.
pub fn peer_gone(fd: RawFd) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLRDHUP, revents: 0 };
    let n = unsafe { libc::poll(&mut p, 1, 0) };
    n > 0 && p.revents & (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR) != 0
}

/// Следить за сокетом, пока жив `Watch`.
pub fn watch(fd: RawFd, on_gone: impl FnOnce() + Send + 'static) -> Watch {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let _ = std::thread::Builder::new().name("hangup".into()).spawn(move || {
        while !s.load(Ordering::SeqCst) {
            if peer_gone(fd) {
                on_gone();
                return;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    });
    Watch { stop }
}
