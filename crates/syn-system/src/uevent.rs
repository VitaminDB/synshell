//! Сообщения ядра об устройствах (netlink `KOBJECT_UEVENT`): отдельный поток
//! читает их и зовёт обработчик для подходящих.

/// Слушать uevent; `matches` отбирает сообщения (поля `KEY=value` через NUL),
/// `changed` зовётся из потока чтения.
pub fn watch(
    name: &str,
    matches: impl Fn(&[u8]) -> bool + Send + 'static,
    mut changed: impl FnMut() + Send + 'static,
) -> std::io::Result<()> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_KOBJECT_UEVENT) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    addr.nl_groups = 1;
    let rc = unsafe { libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_nl>() as u32) };
    if rc < 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut buf = vec![0u8; 16384];
            loop {
                let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                if matches(&buf[..n as usize]) {
                    changed();
                }
            }
            unsafe { libc::close(fd) };
        })?;
    Ok(())
}

/// Есть ли в сообщении поле (`b"SUBSYSTEM=usb"`).
pub fn has(msg: &[u8], field: &[u8]) -> bool {
    msg.split(|b| *b == 0).any(|f| f == field)
}
