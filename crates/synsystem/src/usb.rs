use crate::Sys;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsbDev {
    pub sysname: String,
    pub vid: u16,
    pub pid: u16,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub busnum: u32,
    pub devnum: u32,
    pub speed_mbps: Option<u32>,
    pub max_ma: Option<u32>,
    pub serial: Option<String>,
}

impl UsbDev {
    pub fn id(&self) -> String {
        format!("{:04x}:{:04x}", self.vid, self.pid)
    }

    pub fn key(&self) -> String {
        format!("{}#{}", self.sysname, self.devnum)
    }

    pub fn name(&self) -> Option<String> {
        match (&self.vendor, &self.product) {
            (Some(v), Some(p)) if p.starts_with(v.as_str()) => Some(p.clone()),
            (Some(v), Some(p)) => Some(format!("{v} {p}")),
            (Some(v), None) => Some(v.clone()),
            (None, Some(p)) => Some(p.clone()),
            (None, None) => None,
        }
    }

    pub fn lsusb_line(&self) -> String {
        match self.name() {
            Some(n) => format!("ID {} {n}", self.id()),
            None => format!("ID {}", self.id()),
        }
    }
}

const DEVS: &str = "/sys/bus/usb/devices";
const IDS: &[&str] = &["/usr/share/hwdata/usb.ids", "/usr/share/misc/usb.ids", "/usr/share/usb.ids", "/usr/share/usbutils/usb.ids"];

pub fn ids_lookup(sys: &Sys, vid: u16, pid: u16) -> (Option<String>, Option<String>) {
    let Some(text) = IDS.iter().find_map(|p| std::fs::read(sys.path(p)).ok()) else { return (None, None) };
    let text = String::from_utf8_lossy(&text);
    let (v, p) = (format!("{vid:04x}"), format!("{pid:04x}"));
    let mut vendor = None;
    for line in text.lines() {
        if vendor.is_none() {
            if let Some(rest) = line.strip_prefix(v.as_str()).filter(|_| !line.starts_with('\t')) {
                if rest.starts_with("  ") {
                    vendor = Some(rest.trim().to_string());
                }
            }
            continue;
        }
        if !line.starts_with('\t') {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            break;
        }
        if let Some(rest) = line.strip_prefix('\t').and_then(|l| l.strip_prefix(p.as_str())) {
            if rest.starts_with("  ") {
                return (vendor, Some(rest.trim().to_string()));
            }
        }
    }
    (vendor, None)
}

fn hex16(s: Option<String>) -> Option<u16> {
    u16::from_str_radix(s?.trim(), 16).ok()
}

pub fn device(sys: &Sys, sysname: &str) -> Option<UsbDev> {
    let dir = format!("{DEVS}/{sysname}");
    let r = |k: &str| sys.read(format!("{dir}/{k}")).filter(|v| !v.is_empty());
    let vid = hex16(r("idVendor"))?;
    let pid = hex16(r("idProduct"))?;
    let (ids_vendor, ids_product) = ids_lookup(sys, vid, pid);
    let num = |k: &str| r(k).and_then(|v| v.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok());
    Some(UsbDev {
        sysname: sysname.to_string(),
        vid,
        pid,
        vendor: ids_vendor.or_else(|| r("manufacturer")),
        product: ids_product.or_else(|| r("product")),
        busnum: num("busnum").unwrap_or(0),
        devnum: num("devnum").unwrap_or(0),
        speed_mbps: r("speed").and_then(|s| s.split('.').next()?.parse().ok()),
        max_ma: num("bMaxPower").filter(|v| *v > 0),
        serial: r("serial"),
    })
}

pub fn devices(sys: &Sys) -> Vec<UsbDev> {
    let mut v: Vec<UsbDev> = sys.list(DEVS).iter().filter(|n| !n.contains(':') && !n.starts_with("usb")).filter_map(|n| device(sys, n)).collect();
    v.sort_by(|a, b| (a.busnum, a.devnum).cmp(&(b.busnum, b.devnum)));
    v
}

pub fn watch(mut changed: impl FnMut() + Send + 'static) -> std::io::Result<()> {
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
        .name("usb-uevent".into())
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
                if is_usb_device_event(&buf[..n as usize]) {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                    changed();
                }
            }
            unsafe { libc::close(fd) };
        })?;
    Ok(())
}

fn is_usb_device_event(msg: &[u8]) -> bool {
    let mut action = false;
    let (mut subsystem, mut devtype) = (false, false);
    for f in msg.split(|b| *b == 0) {
        match f {
            b"ACTION=add" | b"ACTION=remove" => action = true,
            b"SUBSYSTEM=usb" => subsystem = true,
            b"DEVTYPE=usb_device" => devtype = true,
            _ => {}
        }
    }
    action && subsystem && devtype
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const IDS_TXT: &str = "# comment\n048d  Integrated Technology Express, Inc.\n\t1234  Something\n17ef  Lenovo\n\t6047  ThinkPad Keyboard\n2717  Xiaomi Inc.\n\tff40  Mi/Redmi series (MTP)\n\tff48  Mi/Redmi series (MTP + ADB)\nC 00  (Defined at Interface level)\n";

    fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let ids = d.path().join("usr/share/hwdata");
        fs::create_dir_all(&ids).unwrap();
        fs::write(ids.join("usb.ids"), IDS_TXT).unwrap();
        for (name, kv) in [
            ("usb3", vec![("idVendor", "1d6b"), ("idProduct", "0002"), ("busnum", "3"), ("devnum", "1")]),
            ("3-1", vec![("idVendor", "2717"), ("idProduct", "ff40"), ("busnum", "3"), ("devnum", "7"), ("product", "Redmi Note 14 Pro"), ("manufacturer", "Xiaomi"), ("speed", "480"), ("bMaxPower", "0mA")]),
            ("3-2", vec![("idVendor", "17ef"), ("idProduct", "f006"), ("busnum", "3"), ("devnum", "2"), ("product", "Idea5003"), ("speed", "12"), ("bMaxPower", "100mA")]),
            ("3-9", vec![("idVendor", "abcd"), ("idProduct", "0001"), ("busnum", "3"), ("devnum", "9")]),
        ] {
            let p = d.path().join("sys/bus/usb/devices").join(name);
            fs::create_dir_all(&p).unwrap();
            for (k, v) in kv {
                fs::write(p.join(k), v).unwrap();
            }
        }
        fs::create_dir_all(d.path().join("sys/bus/usb/devices/3-1:1.0")).unwrap();
        d
    }

    #[test]
    fn names_like_lsusb() {
        let d = fixture();
        let v = devices(&Sys::at(d.path()));
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].lsusb_line(), "ID 17ef:f006 Lenovo Idea5003");
        assert_eq!(v[0].max_ma, Some(100));
        assert_eq!(v[1].lsusb_line(), "ID 2717:ff40 Xiaomi Inc. Mi/Redmi series (MTP)");
        assert_eq!(v[1].max_ma, None);
        assert_eq!(v[1].speed_mbps, Some(480));
        assert_eq!(v[1].key(), "3-1#7");
        assert_eq!(v[2].lsusb_line(), "ID abcd:0001");
    }

    #[test]
    fn uevent_filter() {
        assert!(is_usb_device_event(b"add@/devices/x/3-1\0ACTION=add\0DEVPATH=/devices/x/3-1\0SUBSYSTEM=usb\0DEVTYPE=usb_device\0"));
        assert!(!is_usb_device_event(b"add@/devices/x/3-1:1.0\0ACTION=add\0SUBSYSTEM=usb\0DEVTYPE=usb_interface\0"));
        assert!(!is_usb_device_event(b"change@/x\0ACTION=change\0SUBSYSTEM=usb\0DEVTYPE=usb_device\0"));
    }
}
