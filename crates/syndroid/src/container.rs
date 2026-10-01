//! Рантайм контейнера Android — своя замена LXC.
//!
//! Демон запускает себя же как «стартер» (`syndroidd __container <spec>`) в новых пространствах имён
//! mnt/uts/net/pid (IPC не отделяется: в GKI нет `IPC_NS`, а bionic SysV IPC не использует). Стартер ждёт
//! от демона «go» (за это время демон кладёт его в cgroup контейнера и отдаёт ему veth), отделяет cgroup
//! namespace и порождает PID 1 нового пространства PID. PID 1 собирает корень (образы через loop + overlay,
//! /dev, /proc, /sys, сокеты сеанса), делает pivot_root, урезает capabilities, ставит seccomp и выполняет
//! Android `/init`. Все монтирования живут в пространстве имён контейнера и исчезают вместе с ним
//! (loop — с autoclear), так что после остановки на хосте убирать почти нечего.
//!
//! Чего ждёт `/init` образа Waydroid (патч «init: start inside LXC container»): /dev (tmpfs), /dev/pts,
//! /proc и /sys монтирует хост; `/dev/kmsg`, `/dev/random`, `/dev/urandom` и `/dev/socket` init создаёт сам
//! через CHECKCALL — заранее их делать нельзя (EEXIST = падение init); `/dev/null` и `/dev/ptmx` — можно.

use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::api::Session;
use crate::config::Config;
use crate::{images, net, paths, props, sys};

/// Узлы binderfs контейнера (на хосте) → его /dev/{binder,vndbinder,hwbinder}.
pub const BINDER_NODES: [(&str, &str); 3] = [
    ("syndroid-binder", "binder"),
    ("syndroid-vndbinder", "vndbinder"),
    ("syndroid-hwbinder", "hwbinder"),
];

/// Узлы хоста, которые получает контейнер (если есть): (хост, путь внутри /dev).
const DEV_NODES: &[(&str, &str)] = &[
    ("/dev/zero", "zero"),
    ("/dev/null", "null"),
    ("/dev/full", "full"),
    ("/dev/tty", "tty"),
    ("/dev/ashmem", "ashmem"),
    ("/dev/fuse", "fuse"),
    ("/dev/ion", "ion"),
    ("/dev/kgsl-3d0", "kgsl-3d0"),
    ("/dev/uhid", "uhid"),
    ("/dev/net/tun", "tun"),
    ("/dev/sw_sync", "sw_sync"),
];

/// Узлы, к которым Android (разные uid) должен иметь доступ: права на хосте открываются (как у Waydroid).
fn shared_nodes(c: &Config) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = ["/dev/ashmem", "/dev/kgsl-3d0", "/dev/sw_sync", "/dev/ion"].iter().map(PathBuf::from).collect();
    if let Some(n) = props::drm_node(c) {
        v.push(n.into());
    }
    v.extend(fs::read_dir("/dev/dma_heap").into_iter().flatten().flatten().map(|e| e.path()));
    v.extend(BINDER_NODES.iter().map(|(n, _)| Path::new(paths::BINDERFS).join(n)));
    v
}

/// Что нужно стартеру (передаётся аргументом JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    pub set: String,
    /// Экземпляр Android (`images::instance_of`): чей `/data`.
    pub instance: String,
    pub session: Session,
    pub network: bool,
    pub drm_node: Option<String>,
}

/// Запущенный контейнер (со стороны демона).
pub struct Running {
    pub starter: Child,
    pub init_pid: i32,
    pub dnsmasq: Option<Child>,
}

fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(mode));
}

/// Подготовить хост и запустить контейнер.
pub fn start(c: &Config, session: &Session) -> Result<Running> {
    let set = c.active.clone().context("не выбран набор образов (syndroid image fetch / use)")?;
    if !images::exists(&set) {
        bail!("набор образов «{set}» не установлен");
    }
    if !session.wayland_socket().exists() {
        bail!("нет Wayland-сокета {}", session.wayland_socket().display());
    }
    let instance = images::instance_of(&set);
    migrate_shared_data(&instance);
    let names: Vec<&str> = BINDER_NODES.iter().map(|(n, _)| *n).collect();
    sys::binderfs_nodes(paths::BINDERFS, &names)?;
    for n in shared_nodes(c) {
        chmod(&n, 0o666);
    }
    for d in [
        paths::data(&instance),
        paths::overlay("system"),
        paths::overlay("vendor"),
        paths::overlay_rw(&set, "system"),
        paths::overlay_rw(&set, "vendor"),
        paths::overlay_work(&set, "system"),
        paths::overlay_work(&set, "vendor"),
        paths::image_mnt("system"),
        paths::image_mnt("vendor"),
        paths::rootfs(),
        PathBuf::from(paths::RUN),
    ] {
        fs::create_dir_all(&d).with_context(|| d.display().to_string())?;
    }
    fs::write(paths::props(), props::build(c, session))?;
    chmod(&paths::props(), 0o644);

    let dnsmasq = if c.network { net::up()? } else { None };
    let spec = Spec { set, instance, session: session.clone(), network: c.network, drm_node: props::drm_node(c) };
    let log = fs::File::create(paths::container_log())?;
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("__container")
        .arg(serde_json::to_string(&spec)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log);
    unsafe {
        cmd.pre_exec(|| {
            let flags = libc::CLONE_NEWNS | libc::CLONE_NEWUTS | libc::CLONE_NEWNET | libc::CLONE_NEWPID;
            if libc::unshare(flags) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut starter = cmd.spawn().context("стартер контейнера")?;
    let r = (|| -> Result<i32> {
        let pid = starter.id() as i32;
        fs::create_dir_all(paths::CGROUP)?;
        fs::write(Path::new(paths::CGROUP).join("cgroup.procs"), pid.to_string()).context("cgroup контейнера")?;
        if c.network {
            net::attach(pid)?;
        }
        starter.stdin.take().context("stdin")?.write_all(b"go\n")?;
        let mut line = String::new();
        BufReader::new(starter.stdout.take().context("stdout")?).read_line(&mut line)?;
        line.trim().parse::<i32>().ok().context("контейнер не запустился (см. /run/syndroid/container.log)")
    })();
    match r {
        Ok(init_pid) => Ok(Running { starter, init_pid, dnsmasq }),
        Err(e) => {
            let _ = starter.kill();
            let _ = starter.wait();
            cleanup(dnsmasq, c.network);
            Err(e)
        }
    }
}

/// Раньше `/data` был один на всех (`/var/lib/syndroid/data` — сам корень данных Android): перенести его
/// в каталог экземпляра, который запускается первым.
fn migrate_shared_data(instance: &str) {
    let data = PathBuf::from(paths::STATE).join("data");
    if !data.join("system").is_dir() || paths::data(instance).exists() {
        return;
    }
    let tmp = PathBuf::from(paths::STATE).join("data.migrate");
    let r = fs::rename(&data, &tmp)
        .and_then(|_| fs::create_dir_all(&data))
        .and_then(|_| fs::rename(&tmp, paths::data(instance)));
    match r {
        Ok(()) => tracing::info!("данные Android перенесены в {}", paths::data(instance).display()),
        Err(e) => tracing::warn!("перенос данных Android: {e}"),
    }
}

/// Убрать за остановленным контейнером: сеть, cgroup.
pub fn cleanup(dnsmasq: Option<Child>, network: bool) {
    if network || dnsmasq.is_some() {
        net::down(dnsmasq);
    }
    rmdir_tree(Path::new(paths::CGROUP));
}

fn rmdir_tree(p: &Path) {
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                rmdir_tree(&e.path());
            }
        }
    }
    let _ = fs::remove_dir(p);
}

pub fn freeze(on: bool) -> Result<()> {
    fs::write(Path::new(paths::CGROUP).join("cgroup.freeze"), if on { "1" } else { "0" }).context("cgroup.freeze")
}

// --- стартер и PID 1 (внутри пространств имён) -----------------------------------------------------

/// `syndroidd __container <spec>`: ждать «go», породить PID 1, сообщить его PID, ждать его.
pub fn starter_main(spec_json: &str) -> ! {
    let code = (|| -> Result<i32> {
        let spec: Spec = serde_json::from_str(spec_json)?;
        let mut go = String::new();
        std::io::stdin().lock().read_line(&mut go)?;
        if go.trim() != "go" {
            bail!("нет команды go");
        }
        if unsafe { libc::unshare(libc::CLONE_NEWCGROUP) } != 0 {
            bail!("unshare cgroup: {}", std::io::Error::last_os_error());
        }
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            bail!("fork: {}", std::io::Error::last_os_error());
        }
        if pid == 0 {
            let e = init_main(&spec);
            eprintln!("syndroid: контейнер: {e:#}");
            unsafe { libc::_exit(1) };
        }
        println!("{pid}");
        std::io::stdout().flush()?;
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        Ok(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) })
    })();
    match code {
        Ok(c) => std::process::exit(c),
        Err(e) => {
            eprintln!("syndroid: стартер: {e:#}");
            std::process::exit(1)
        }
    }
}

/// PID 1 контейнера: собрать корень и выполнить Android `/init`. Возвращается только с ошибкой.
fn init_main(spec: &Spec) -> anyhow::Error {
    match setup_root(spec) {
        Ok(()) => {}
        Err(e) => return e,
    }
    let init = CString::new("/init").unwrap();
    let argv = [init.as_ptr(), std::ptr::null()];
    let env: Vec<CString> = ["PATH=/system/bin:/system/xbin:/vendor/bin", "container=syndroid"]
        .iter()
        .map(|s| CString::new(*s).unwrap())
        .collect();
    let mut envp: Vec<*const libc::c_char> = env.iter().map(|s| s.as_ptr()).collect();
    envp.push(std::ptr::null());
    unsafe { libc::execve(init.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
    anyhow::anyhow!("execve /init: {}", std::io::Error::last_os_error())
}

fn setup_root(spec: &Spec) -> Result<()> {
    // Монтирования контейнера не уходят на хост
    sys::mount("", "/", "", libc::MS_REC | libc::MS_PRIVATE, "")?;
    if spec.network {
        net::configure_inside()?;
    } else {
        let _ = net::run("ip", &["link", "set", "lo", "up"]);
    }
    let set_dir = images::dir(&spec.set);
    let loopdir = Path::new(paths::RUN).join("loop");
    let (msys, mven) = (paths::image_mnt("system"), paths::image_mnt("vendor"));
    sys::mount_image(&set_dir.join("system.img"), &msys, &loopdir).context("system.img")?;
    sys::mount_image(&set_dir.join("vendor.img"), &mven, &loopdir).context("vendor.img")?;
    let r = paths::rootfs();
    // Слои: свои файлы (overlay/) → файлы платформы (/usr/share/syndroid/overlay) → образ; сверху — изменения Android
    let ovl = |lower_own: PathBuf, lower: &Path, part: &str, target: &Path| {
        let platform = paths::platform_overlay(part);
        let mid = if platform.is_dir() { format!(":{}", platform.display()) } else { String::new() };
        let data = format!(
            "lowerdir={}{mid}:{},upperdir={},workdir={}",
            lower_own.display(),
            lower.display(),
            paths::overlay_rw(&spec.set, part).display(),
            paths::overlay_work(&spec.set, part).display()
        );
        sys::mount("overlay", target, "overlay", 0, &data).with_context(|| format!("overlay {part}"))
    };
    ovl(paths::overlay("system"), &msys, "system", &r)?;
    ovl(paths::overlay("vendor"), &mven, "vendor", &r.join("vendor"))?;
    let prop = r.join("vendor/waydroid.prop");
    sys::touch(&prop)?;
    sys::bind(paths::props(), &prop, false)?;
    sys::bind(paths::data(&spec.instance), r.join("data"), false)?;

    // /dev
    let dev = r.join("dev");
    fs::create_dir_all(&dev)?;
    // 1777, как tmpfs по умолчанию у LXC: hwcomposer (uid «host», без привилегий) сам создаёт /dev/input с
    // каналами ввода (wl_touch_events и др.) — при 0755 ввод в Android не доходит
    sys::mount("tmpfs", &dev, "tmpfs", libc::MS_NOSUID, "mode=1777")?;
    let mut nodes: Vec<(PathBuf, String)> =
        DEV_NODES.iter().map(|(h, c)| (PathBuf::from(h), c.to_string())).collect();
    // Все узлы DRM: gralloc (gbm) открывает drm_node, turnip-KGSL сверяет с ними своё устройство (stat)
    use std::os::unix::fs::FileTypeExt;
    for e in fs::read_dir("/dev/dri").into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_char_device())) {
        nodes.push((e.path(), format!("dri/{}", e.file_name().to_string_lossy())));
    }
    for e in fs::read_dir("/dev/dma_heap").into_iter().flatten().flatten() {
        nodes.push((e.path(), format!("dma_heap/{}", e.file_name().to_string_lossy())));
    }
    for (host, inside) in BINDER_NODES {
        nodes.push((Path::new(paths::BINDERFS).join(host), inside.to_string()));
    }
    for (host, inside) in nodes {
        if !host.exists() {
            continue;
        }
        let t = dev.join(&inside);
        sys::touch(&t)?;
        sys::bind(&host, &t, false).with_context(|| inside.clone())?;
    }
    let pts = dev.join("pts");
    fs::create_dir_all(&pts)?;
    sys::mount("devpts", &pts, "devpts", libc::MS_NOSUID | libc::MS_NOEXEC, "newinstance,ptmxmode=0666,mode=0620")?;
    std::os::unix::fs::symlink("pts/ptmx", dev.join("ptmx"))?;

    // /proc, /sys (только чтение, как у LXC sys:ro), cgroup — на запись: Android раскладывает процессы по своим
    // группам (createProcessGroup), а в своём cgroup namespace он видит только поддерево /sys/fs/cgroup/syndroid
    let nse = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
    sys::mount("proc", r.join("proc"), "proc", nse, "")?;
    sys::mount("sysfs", r.join("sys"), "sysfs", nse | libc::MS_RDONLY, "")?;
    let _ = sys::mount("cgroup2", r.join("sys/fs/cgroup"), "cgroup2", nse, "");

    // tmpfs в местах, где в корне Android нет своей ФС
    for d in ["tmp", "var", "run", "mnt_extra"] {
        let p = r.join(d);
        fs::create_dir_all(&p)?;
        sys::mount("tmpfs", &p, "tmpfs", libc::MS_NODEV, "mode=0755")?;
    }

    // Сеанс: Wayland и PulseAudio. Каталог XDG_RUNTIME_DIR пользователя — целиком (/run/xdg-host), а сокеты в
    // /run/xdg — ссылки на него: путь разрешается при каждом подключении, и после перезапуска композитора
    // (новый сокет по тому же пути) hwcomposer подключается заново. Привязка самого файла сокета держала бы
    // старый инод — Android терял экран до своего перезапуска. Каталог 0700: внутри его видит только uid
    // владельца сеанса (hwcomposer, звук), приложения Android — нет.
    let xdg = r.join(paths::CONTAINER_XDG_RUNTIME_DIR.trim_start_matches('/'));
    fs::create_dir_all(&xdg)?;
    sys::mount("tmpfs", &xdg, "tmpfs", libc::MS_NODEV, "mode=0755")?;
    let host = r.join("run/xdg-host");
    fs::create_dir_all(&host)?;
    sys::bind(&spec.session.xdg_runtime_dir, &host, false).context("XDG_RUNTIME_DIR сеанса")?;
    let rel = |p: &Path| -> Result<PathBuf> {
        let p = p.strip_prefix(&spec.session.xdg_runtime_dir).map_err(|_| anyhow::anyhow!("{} вне XDG_RUNTIME_DIR", p.display()))?;
        Ok(PathBuf::from("/run/xdg-host").join(p))
    };
    std::os::unix::fs::symlink(rel(&spec.session.wayland_socket())?, xdg.join(paths::CONTAINER_WAYLAND_DISPLAY))?;
    let pulse_dir = if spec.session.pulse_runtime_path.is_empty() {
        Path::new(&spec.session.xdg_runtime_dir).join("pulse")
    } else {
        PathBuf::from(&spec.session.pulse_runtime_path)
    };
    if let Ok(target) = rel(&pulse_dir.join("native")) {
        fs::create_dir_all(xdg.join("pulse"))?;
        std::os::unix::fs::symlink(target, xdg.join("pulse/native"))?;
    }

    sys::sethostname("syndroid")?;
    sys::pivot_root(&r)?;
    sys::drop_caps()?;
    sys::install_seccomp()?;
    Ok(())
}

// --- вход в работающий контейнер ---------------------------------------------------------------------

/// Окружение Android для команд (`am`, `pm` и т. п. нужен BOOTCLASSPATH; CLASSPATH Android записывает сам в
/// /data/system/environ/classpath — читаем его через /proc/<pid>/root).
pub fn android_env(init_pid: i32) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = [
        ("PATH", "/product/bin:/apex/com.android.runtime/bin:/apex/com.android.art/bin:/system_ext/bin:/system/bin:/system/xbin:/odm/bin:/vendor/bin:/vendor/xbin"),
        ("ANDROID_ROOT", "/system"),
        ("ANDROID_DATA", "/data"),
        ("ANDROID_STORAGE", "/storage"),
        ("ANDROID_ART_ROOT", "/apex/com.android.art"),
        ("ANDROID_I18N_ROOT", "/apex/com.android.i18n"),
        ("ANDROID_TZDATA_ROOT", "/apex/com.android.tzdata"),
        ("ANDROID_RUNTIME_ROOT", "/apex/com.android.runtime"),
        ("HOME", "/data"),
        ("TERM", "xterm-256color"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    if let Ok(s) = fs::read_to_string(format!("/proc/{init_pid}/root/data/system/environ/classpath")) {
        for line in s.lines() {
            let mut it = line.splitn(3, ' ');
            if let (Some("export"), Some(k), Some(v)) = (it.next(), it.next(), it.next()) {
                env.push((k.to_string(), v.to_string()));
            }
        }
    }
    env
}

/// Команда, выполняемая внутри контейнера (от root): помощник `__exec` (та же программа) входит в его
/// пространства имён и порождает команду — так она и в пространстве PID контейнера (иначе Android-программам
/// не хватает /proc/self).
pub fn command_in(init_pid: i32, argv: &[String]) -> Result<Command> {
    if argv.is_empty() {
        bail!("пустая команда");
    }
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("__exec").arg(init_pid.to_string()).args(argv);
    Ok(cmd)
}

/// `<программа> __exec <pid init> <команда…>`: войти в контейнер и выполнить команду, вернуть её код.
pub fn exec_main(args: &[String]) -> ! {
    let r = (|| -> Result<i32> {
        let pid: i32 = args.first().context("нет PID")?.parse()?;
        let argv = &args[1..];
        let env = android_env(pid);
        let pidfd = sys::pidfd_open(pid).context("контейнер не запущен")?;
        let flags = libc::CLONE_NEWNS | libc::CLONE_NEWUTS | libc::CLONE_NEWNET | libc::CLONE_NEWCGROUP | libc::CLONE_NEWPID;
        if unsafe { libc::setns(pidfd.as_raw_fd(), flags) } != 0 {
            bail!("setns: {}", std::io::Error::last_os_error());
        }
        drop(pidfd);
        let child = unsafe { libc::fork() };
        if child < 0 {
            bail!("fork: {}", std::io::Error::last_os_error());
        }
        if child == 0 {
            let e = Command::new(&argv[0]).args(&argv[1..]).env_clear().envs(env).current_dir("/").exec();
            eprintln!("{}: {e}", argv[0]);
            unsafe { libc::_exit(127) };
        }
        // Ctrl-C терминала получает команда; помощник лишь ждёт её
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        }
        let mut status = 0;
        while unsafe { libc::waitpid(child, &mut status, 0) } < 0 {
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        Ok(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) })
    })();
    match r {
        Ok(c) => std::process::exit(c),
        Err(e) => {
            eprintln!("syndroid: {e:#}");
            std::process::exit(125)
        }
    }
}

/// Значение свойства Android (`getprop`), пусто — нет/не удалось.
pub fn getprop(init_pid: i32, name: &str) -> String {
    command_in(init_pid, &["/system/bin/getprop".into(), name.into()])
        .and_then(|mut c| Ok(c.stdin(Stdio::null()).stderr(Stdio::null()).output()?))
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Секунды жизни процесса (по /proc/<pid>/stat).
pub fn uptime_of(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let start: u64 = stat.rsplit(')').next()?.split_whitespace().nth(19)?.parse().ok()?;
    let mut up = String::new();
    fs::File::open("/proc/uptime").ok()?.read_to_string(&mut up).ok()?;
    let up: f64 = up.split_whitespace().next()?.parse().ok()?;
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    Some((up - start as f64 / hz).max(0.0) as u64)
}
