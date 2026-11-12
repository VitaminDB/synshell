//! «Диск инструментов DOS» syn-gpu: машинам с BIOS и видеокартой syn-gpu — маленький жёсткий
//! диск FAT16 (метка SYNGPU) с каталогом `SYNGPU\`: SYNGPU.COM (установщик Glide для игр) и
//! GLIDE2X.OVL (Glide 2.x, 3D на видеокарте хоста) из проекта syn-virt. DOS видит его через
//! BIOS без драйверов (CD-драйвер у FreeDOS грузится не всегда). Образ собирается здесь, своим
//! кодом (без mtools), в `~/.cache/syn-vm/syn-gpu-dos-tools.img` и пересобирается при смене
//! файлов. Подключается при запуске машины (`snapshot=on`: записи гостя не сохраняются).
//! Горячего подключения нет: у AHCI/IDE QEMU нет hotplug.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use syngui::t;

use crate::model::{Firmware, OsKind, Vm};

pub const LABEL: &str = "SYNGPU";
/// Имя устройства и диска у QEMU.
pub const ID: &str = "dostools";
const MAIN: &str = "SYNGPU.COM";

const README: &str = "syn-gpu tools for DOS\r\n\
\r\n\
SYNGPU.COM   - installer: finds games with GLIDE2X.OVL on the hard disks,\r\n\
               installs the syn-gpu GLIDE2X.OVL (the original is kept),\r\n\
               restores the original, shows the video card.\r\n\
GLIDE2X.OVL  - Glide 2.x for DOS: 3D on the host video card (syn-gpu).\r\n\
\r\n\
Run SYNGPU.COM from this folder. Changes to this disk are not saved.\r\n";

/// Машина, которой нужен этот диск: BIOS, не Windows и не Linux (DOS, FreeDOS и подобные).
pub fn is_dos(vm: &Vm) -> bool {
    vm.firmware == Firmware::Bios && !vm.os.is_windows() && vm.os != OsKind::Linux
}

/// Диск подключается при запуске: включён, BIOS и syn-gpu.
pub fn wanted(vm: &Vm) -> bool {
    vm.dos_tools && vm.firmware == Firmware::Bios && vm.gpu_on()
}

/// Каталог с SYNGPU.COM: SYN_GPU_DOS_TOOLS, сборка syn-virt рядом с реальной программой syn-gpu
/// (`syn-virt/target/<профиль>/syn-gpu` → `syn-virt/out/dos-tools`), рядом с ней, пакет syn-gpu.
pub fn tools_dir() -> Option<PathBuf> {
    let mut c: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("SYN_GPU_DOS_TOOLS") {
        c.push(PathBuf::from(p));
    }
    if let Some(bin) = crate::qemu::syn_gpu_bin().and_then(|b| std::fs::canonicalize(b).ok()) {
        if let Some(root) = bin.parent().and_then(Path::parent).and_then(Path::parent) {
            c.push(root.join("out/dos-tools"));
        }
        if let Some(dir) = bin.parent() {
            c.push(dir.join("dos-tools"));
            c.push(dir.join("../lib/syn-gpu/dos-tools"));
        }
    }
    c.push(PathBuf::from("/usr/lib/syn-gpu/dos-tools"));
    c.push(PathBuf::from("/usr/share/syn-gpu/dos-tools"));
    c.into_iter().find(|d| d.join(MAIN).is_file())
}

pub fn image_path() -> PathBuf {
    crate::guesttools::cache_dir().join("syn-gpu-dos-tools.img")
}

fn mtime(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Имя 8.3 (заглавные, без пробелов) → 11 байт записи каталога.
fn short_name(name: &str) -> Option<[u8; 11]> {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let ok = |s: &str, n: usize| !s.is_empty() && s.len() <= n && s.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b"_-~!#$%&'()@^{}".contains(&b));
    if !ok(base, 8) || !(ext.is_empty() || ok(ext, 3)) {
        return None;
    }
    let mut r = [b' '; 11];
    r[..base.len()].copy_from_slice(base.as_bytes());
    r[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    Some(r)
}

/// Время → дата и время DOS (местное время).
fn dos_time(secs: i64) -> (u16, u16) {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() || tm.tm_year < 80 {
        return (0x21, 0); // 1980-01-01
    }
    let date = (((tm.tm_year - 80) as u16) << 9) | (((tm.tm_mon + 1) as u16) << 5) | tm.tm_mday as u16;
    let time = ((tm.tm_hour as u16) << 11) | ((tm.tm_min as u16) << 5) | (tm.tm_sec as u16 / 2);
    (date, time)
}

/// Файл образа: имя 8.3, содержимое, время изменения (секунды Unix).
pub struct File {
    pub name: String,
    pub data: Vec<u8>,
    pub mtime: i64,
}

const SEC: usize = 512;
const HEADS: usize = 16;
const SPT: usize = 63;
const ROOT_ENTRIES: usize = 512;

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn dirent(name: &[u8; 11], attr: u8, cluster: u16, size: u32, (date, time): (u16, u16)) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    put16(&mut e, 14, time);
    put16(&mut e, 16, date);
    put16(&mut e, 18, date);
    put16(&mut e, 22, time);
    put16(&mut e, 24, date);
    put16(&mut e, 26, cluster);
    put32(&mut e, 28, size);
    e
}

/// Жёсткий диск: MBR с одним разделом FAT16 (геометрия 16×63, раздел с сектора 63), в корне —
/// метка тома `label` и каталог `dir` с файлами `files`.
pub fn build_disk(label: &str, dir: &str, files: &[File]) -> Result<Vec<u8>> {
    let label_n = short_name(label).context("метка тома")?;
    let dir_n = short_name(dir).context("имя каталога")?;
    let names: Vec<[u8; 11]> = files.iter().map(|f| short_name(&f.name).with_context(|| format!("имя не 8.3: {}", f.name))).collect::<Result<_>>()?;
    let spc = 1usize; // кластер — один сектор: кластеров заведомо больше 4085 (FAT16)
    let clusters_of = |n: usize| n.div_ceil(SEC * spc);
    let dir_clusters = clusters_of((files.len() + 2) * 32).max(1);
    let need: usize = dir_clusters + files.iter().map(|f| clusters_of(f.data.len())).sum::<usize>();
    // Цилиндров — от 16 (~8 МБ) и с запасом; раздел меньше 32 МБ (FAT16, тип 04h).
    let cyl_secs = HEADS * SPT;
    let cyls = ((need * spc + 200 + SPT).div_ceil(cyl_secs)).max(16);
    let total = cyls * cyl_secs;
    let part_start = SPT;
    let part_len = total - part_start;
    if part_len >= 65536 {
        bail!("файлы инструментов DOS больше 32 МБ");
    }
    let root_secs = ROOT_ENTRIES * 32 / SEC;
    let mut fat_secs = 1;
    let clusters = loop {
        let c = (part_len - 1 - 2 * fat_secs - root_secs) / spc;
        let n = ((c + 2) * 2).div_ceil(SEC);
        if n <= fat_secs {
            break c;
        }
        fat_secs = n;
    };
    let mut img = vec![0u8; total * SEC];
    let serial = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for f in files {
            f.name.hash(&mut h);
            f.data.hash(&mut h);
        }
        h.finish() as u32
    };

    // MBR: загрузка с него — дальше по списку (int 18h); один активный раздел.
    img[..4].copy_from_slice(&[0xCD, 0x18, 0xEB, 0xFE]);
    put32(&mut img, 440, serial);
    let pe = 446;
    let last_cyl = cyls - 1;
    img[pe] = 0x80;
    img[pe + 1..pe + 4].copy_from_slice(&[1, 1, 0]); // C0 H1 S1
    img[pe + 4] = 0x04; // FAT16 < 32 МБ
    img[pe + 5..pe + 8].copy_from_slice(&[(HEADS - 1) as u8, SPT as u8 | (((last_cyl >> 8) & 3) << 6) as u8, (last_cyl & 0xff) as u8]);
    put32(&mut img, pe + 8, part_start as u32);
    put32(&mut img, pe + 12, part_len as u32);
    img[510] = 0x55;
    img[511] = 0xAA;

    // Загрузочный сектор раздела (BPB FAT16).
    let bs = part_start * SEC;
    let b = &mut img[bs..bs + SEC];
    b[..3].copy_from_slice(&[0xEB, 0x3C, 0x90]);
    b[3..11].copy_from_slice(b"MSDOS5.0");
    put16(b, 11, SEC as u16);
    b[13] = spc as u8;
    put16(b, 14, 1);
    b[16] = 2;
    put16(b, 17, ROOT_ENTRIES as u16);
    put16(b, 19, part_len as u16);
    b[21] = 0xF8;
    put16(b, 22, fat_secs as u16);
    put16(b, 24, SPT as u16);
    put16(b, 26, HEADS as u16);
    put32(b, 28, part_start as u32);
    b[36] = 0x80;
    b[38] = 0x29;
    put32(b, 39, serial);
    b[43..54].copy_from_slice(&label_n);
    b[54..62].copy_from_slice(b"FAT16   ");
    b[62..66].copy_from_slice(&[0xCD, 0x18, 0xEB, 0xFE]);
    b[510] = 0x55;
    b[511] = 0xAA;

    // Размещение: каталог, затем файлы подряд.
    let fat0 = bs + SEC;
    let root = fat0 + 2 * fat_secs * SEC;
    let data = root + root_secs * SEC;
    let mut fat = vec![0u16; clusters + 2];
    fat[0] = 0xFFF8;
    fat[1] = 0xFFFF;
    let mut next = 2usize;
    let mut alloc = |n: usize, fat: &mut Vec<u16>| -> Result<u16> {
        if n == 0 {
            return Ok(0);
        }
        if next + n > clusters + 2 {
            bail!("не хватает места на диске инструментов DOS");
        }
        let first = next;
        for c in first..first + n {
            fat[c] = if c + 1 < first + n { (c + 1) as u16 } else { 0xFFFF };
        }
        next += n;
        Ok(first as u16)
    };
    let cl_off = |c: u16| data + (c as usize - 2) * spc * SEC;

    let now = dos_time(files.iter().map(|f| f.mtime).max().unwrap_or(0));
    let dir_c = alloc(dir_clusters, &mut fat)?;
    let mut entries: Vec<[u8; 32]> = vec![dirent(b".          ", 0x10, dir_c, 0, now), dirent(b"..         ", 0x10, 0, 0, now)];
    for (f, n) in files.iter().zip(&names) {
        let c = alloc(clusters_of(f.data.len()), &mut fat)?;
        if c != 0 {
            let o = cl_off(c);
            img[o..o + f.data.len()].copy_from_slice(&f.data);
        }
        entries.push(dirent(n, 0x20, c, f.data.len() as u32, dos_time(f.mtime)));
    }
    let o = cl_off(dir_c);
    for (i, e) in entries.iter().enumerate() {
        img[o + i * 32..o + i * 32 + 32].copy_from_slice(e);
    }
    let mut lab = dirent(&label_n, 0x08, 0, 0, now);
    lab[14..18].fill(0);
    img[root..root + 32].copy_from_slice(&lab);
    img[root + 32..root + 64].copy_from_slice(&dirent(&dir_n, 0x10, dir_c, 0, now));
    for k in 0..2 {
        let at = fat0 + k * fat_secs * SEC;
        for (i, v) in fat.iter().enumerate() {
            put16(&mut img, at + i * 2, *v);
        }
    }
    Ok(img)
}

/// Файлы каталога инструментов (имена 8.3 заглавными) и README.TXT.
fn collect(dir: &Path) -> Result<Vec<File>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let e = e?;
        let m = std::fs::metadata(e.path())?;
        let name = e.file_name().to_string_lossy().to_uppercase();
        if !m.is_file() || name == "README.TXT" {
            continue;
        }
        if short_name(&name).is_none() {
            tracing::warn!("инструменты DOS: пропущен {} (имя не 8.3)", e.path().display());
            continue;
        }
        v.push(File { name, data: std::fs::read(e.path())?, mtime: mtime(&m) });
    }
    v.sort_by(|a, b| (a.name != MAIN, &a.name).cmp(&(b.name != MAIN, &b.name)));
    let t = v.iter().map(|f| f.mtime).max().unwrap_or(0);
    // Все файлы каталога попадают на диск; неописанные в README — списком в конце.
    let mut readme = README.to_string();
    let extra: Vec<&str> = v.iter().map(|f| f.name.as_str()).filter(|n| !README.contains(n)).collect();
    if !extra.is_empty() {
        readme += &format!("\r\nAlso here: {}\r\n", extra.join(", "));
    }
    v.push(File { name: "README.TXT".into(), data: readme.into_bytes(), mtime: t });
    Ok(v)
}

/// Собрать (или взять готовый) образ диска из найденных файлов.
pub fn image() -> Result<PathBuf> {
    let dir = tools_dir().with_context(|| t!("Нет инструментов DOS syn-gpu (SYNGPU.COM): /usr/lib/syn-gpu/dos-tools или out/dos-tools сборки syn-virt"))?;
    // Отметка — по тем самым файлам, что попадут в образ (содержимое): файл, сменившийся во время
    // сборки (syn-virt собирает SYNMOUSE.COM), не оставит устаревший образ с «совпавшей» отметкой.
    image_from(&dir, &image_path())
}

/// Образ из каталога `dir` в `out` (готовый с той же отметкой — как есть).
fn image_from(dir: &Path, out: &Path) -> Result<PathBuf> {
    let files = collect(dir)?;
    let mut stamp = format!("v2\n{}\n", dir.display());
    for f in &files {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&f.data, &mut h);
        stamp += &format!("{} {} {:016x}\n", f.name, f.data.len(), std::hash::Hasher::finish(&h));
    }
    let stamp_path = out.with_extension("stamp");
    if out.is_file() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp) {
        return Ok(out.to_path_buf());
    }
    let img = build_disk(LABEL, "SYNGPU", &files)?;
    std::fs::create_dir_all(out.parent().unwrap())?;
    // Через переименование: работающий QEMU держит прежний файл.
    let tmp = out.with_extension("img.part");
    std::fs::write(&tmp, img)?;
    std::fs::rename(&tmp, out)?;
    std::fs::write(&stamp_path, stamp)?;
    tracing::info!("диск инструментов DOS: {} из {}", out.display(), dir.display());
    Ok(out.to_path_buf())
}

/// Аргументы QEMU: диск инструментов на свободном порту AHCI, без bootindex (последний по
/// счёту жёсткий диск). Записи гостя — во временный слой (`snapshot=on`): IDE-диск не бывает
/// «только для чтения».
/// `at` — место на шине у i440fx (`Some(None)` — свободных мест IDE нет), `None` — q35 (порт AHCI
/// подбирается здесь).
pub fn qemu_args(vm: &Vm, at: Option<Option<String>>) -> Option<Vec<String>> {
    if !wanted(vm) {
        return None;
    }
    let at = match at {
        Some(Some(a)) => a,
        Some(None) => {
            tracing::warn!("диск инструментов DOS: у i440fx нет свободного места IDE");
            return None;
        }
        None => {
            let used: Vec<usize> = if vm.disk_bus == crate::model::DiskBus::Sata { (0..vm.disks.len()).map(|i| i.min(3)).collect() } else { Vec::new() };
            let port = [0usize, 1, 2, 3, 5].into_iter().find(|p| !used.contains(p) && !(*p == 5 && vm.tools_cd))?;
            format!("bus=ide.{port}")
        }
    };
    let img = image().map_err(|e| tracing::warn!("диск инструментов DOS: {e}")).ok()?;
    let file = img.to_string_lossy().replace(',', ",,");
    Some(vec![
        "-drive".into(),
        format!("file={file},if=none,id={ID},format=raw,snapshot=on"),
        "-device".into(),
        format!("ide-hd,drive={ID},{at},id={ID}"),
    ])
}

/// Диск подключён к работающей машине (QMP query-block).
pub fn attached(vm: &Vm) -> bool {
    crate::qmp::call(vm, "query-block", serde_json::json!({}))
        .ok()
        .and_then(|v| v.as_array().cloned())
        .is_some_and(|a| a.iter().any(|b| b["device"] == ID || b["qdev"].as_str().is_some_and(|q| q.contains(ID))))
}

/// Подсказка, как запустить установщик в госте (буква — после дисков машины).
pub fn howto(vm: &Vm) -> String {
    let letter = (b'C' + vm.disks.len().min(23) as u8) as char;
    t!("В DOS запустите {p} (диск с меткой SYNGPU — после дисков машины; буква зависит от их разделов, проверьте командой DIR)", p = format!("{letter}:\\SYNGPU\\SYNGPU.COM"))
}

/// `--gpu-tools МАШИНА install` для DOS: включить диск (сохраняется в машине), собрать образ.
pub fn install(vm: &Vm) -> Result<String> {
    if !vm.gpu_on() {
        bail!("{}", t!("Включите «3D-ускорение (syn-gpu)» — инструментам DOS нужна видеокарта syn-gpu"));
    }
    image()?;
    if !vm.dos_tools {
        let mut v = vm.clone();
        v.dos_tools = true;
        v.save()?;
    }
    let s = if crate::qemu::pid(vm).is_none() {
        t!("Диск инструментов DOS включён: подключится при запуске машины")
    } else if attached(vm) {
        t!("Диск инструментов DOS подключён к машине")
    } else {
        t!("Диск инструментов DOS появится при следующем запуске машины (горячее подключение дисков IDE QEMU не умеет): выключите и включите её")
    };
    Ok(format!("{s}\n{}", howto(vm)))
}

/// Состояние для `--gpu-tools МАШИНА status` у DOS-машины.
pub fn status(vm: &Vm) -> String {
    let files = match tools_dir() {
        Some(d) => d.display().to_string(),
        None => return t!("Нет инструментов DOS syn-gpu (SYNGPU.COM): /usr/lib/syn-gpu/dos-tools или out/dos-tools сборки syn-virt"),
    };
    let on = if !vm.dos_tools {
        t!("диск инструментов DOS выключен (syn-vm --gpu-tools {id} install)", id = vm.id)
    } else if crate::qemu::pid(vm).is_none() {
        t!("диск инструментов DOS включён — подключится при запуске")
    } else if attached(vm) {
        t!("диск инструментов DOS подключён")
    } else {
        t!("диск инструментов DOS включён — подключится при следующем запуске")
    };
    format!("{on}; {files}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16at(b: &[u8], a: usize) -> usize {
        u16::from_le_bytes([b[a], b[a + 1]]) as usize
    }

    /// Все файлы каталога — на диск (новые тоже, например SYNMOUSE.COM), README перечисляет
    /// неописанные; образ пересобирается при смене содержимого, даже того же размера.
    #[test]
    fn all_files_and_rebuild() {
        let root = std::env::temp_dir().join(format!("syn-vm-dostools-all-{}", std::process::id()));
        let tools = root.join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("SYNGPU.COM"), [0xC3; 100]).unwrap();
        std::fs::write(tools.join("SYNMOUSE.COM"), [1u8; 50]).unwrap();
        let files = collect(&tools).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["SYNGPU.COM", "SYNMOUSE.COM", "README.TXT"]);
        assert!(String::from_utf8_lossy(&files[2].data).contains("Also here: SYNMOUSE.COM"));
        let out = root.join("cache/tools.img");
        let a = std::fs::read(image_from(&tools, &out).unwrap()).unwrap();
        std::fs::write(tools.join("SYNMOUSE.COM"), [2u8; 50]).unwrap();
        let b = std::fs::read(image_from(&tools, &out).unwrap()).unwrap();
        assert_ne!(a, b, "тот же размер, другое содержимое — образ должен пересобраться");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn short_names() {
        assert_eq!(&short_name("SYNGPU.COM").unwrap(), b"SYNGPU  COM");
        assert_eq!(&short_name("SYNGPU").unwrap(), b"SYNGPU     ");
        assert!(short_name("syngpu.com").is_none());
        assert!(short_name("LONGNAME12.TXT").is_none());
        assert!(short_name("A.ABCD").is_none());
    }

    /// Свой разбор образа: MBR → BPB → корень → каталог → файлы по цепочкам FAT.
    #[test]
    fn disk_layout() {
        let big: Vec<u8> = (0..33096u32).map(|i| (i * 7 + i / 513) as u8).collect();
        let files = vec![
            File { name: "SYNGPU.COM".into(), data: vec![0xC3; 23956], mtime: 1_794_000_000 },
            File { name: "GLIDE2X.OVL".into(), data: big.clone(), mtime: 1_794_000_000 },
            File { name: "EMPTY.TXT".into(), data: vec![], mtime: 0 },
        ];
        let img = build_disk(LABEL, "SYNGPU", &files).unwrap();
        assert_eq!(img.len() % (HEADS * SPT * SEC), 0);
        assert_eq!(&img[510..512], &[0x55, 0xAA]);
        assert_eq!(img[446 + 4], 0x04);
        let ps = u32::from_le_bytes(img[454..458].try_into().unwrap()) as usize;
        let pl = u32::from_le_bytes(img[458..462].try_into().unwrap()) as usize;
        assert_eq!(ps + pl, img.len() / SEC);
        let b = &img[ps * SEC..ps * SEC + SEC];
        assert_eq!(&b[510..512], &[0x55, 0xAA]);
        assert_eq!(&b[43..54], b"SYNGPU     ");
        let (spc, rsv, nf, re, fs) = (b[13] as usize, u16at(b, 14), b[16] as usize, u16at(b, 17), u16at(b, 22));
        let fat = ps * SEC + rsv * SEC;
        let root = fat + nf * fs * SEC;
        let data = root + re * 32;
        let clusters = (u16at(b, 19) - rsv - nf * fs - re * 32 / SEC) / spc;
        assert!((4085..65525).contains(&clusters), "FAT16: {clusters}");
        // Копии FAT одинаковы.
        assert_eq!(img[fat..fat + fs * SEC], img[fat + fs * SEC..fat + 2 * fs * SEC]);
        assert_eq!(&img[root..root + 11], b"SYNGPU     ");
        assert_eq!(img[root + 11], 0x08);
        assert_eq!(&img[root + 32..root + 43], b"SYNGPU     ");
        assert_eq!(img[root + 43], 0x10);
        let read = |c0: usize, size: usize| -> Vec<u8> {
            let mut out = Vec::new();
            let mut c = c0;
            while c >= 2 && c < 0xFFF8 {
                let o = data + (c - 2) * spc * SEC;
                out.extend_from_slice(&img[o..o + spc * SEC]);
                c = u16at(&img, fat + c * 2);
            }
            out.truncate(size);
            out
        };
        let d = read(u16at(&img, root + 32 + 26), SEC * spc);
        assert_eq!(&d[..11], b".          ");
        assert_eq!(&d[32..43], b"..         ");
        for (i, f) in files.iter().enumerate() {
            let e = &d[(i + 2) * 32..(i + 3) * 32];
            assert_eq!(&e[..11], &short_name(&f.name).unwrap());
            let size = u32::from_le_bytes(e[28..32].try_into().unwrap()) as usize;
            assert_eq!(size, f.data.len());
            assert_eq!(read(u16at(e, 26), size), f.data, "{}", f.name);
        }
        assert_eq!(d[5 * 32], 0, "конец каталога");
    }

    /// Проверка чужими средствами, если есть: fsck.fat и mtools.
    #[test]
    fn disk_external_tools() {
        let files = vec![File { name: "SYNGPU.COM".into(), data: b"hello".to_vec(), mtime: 1_794_000_000 }];
        let img = build_disk(LABEL, "SYNGPU", &files).unwrap();
        let dir = std::env::temp_dir().join(format!("syn-vm-dostools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let disk = dir.join("disk.img");
        std::fs::write(&disk, &img).unwrap();
        let part = dir.join("part.img");
        std::fs::write(&part, &img[SPT * SEC..]).unwrap();
        if let Ok(o) = std::process::Command::new("fsck.fat").arg("-n").arg(&part).output() {
            assert!(o.status.success(), "fsck.fat: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        }
        if let Ok(o) = std::process::Command::new("mtype").arg("-i").arg(format!("{}@@{}", disk.display(), SPT * SEC)).arg("::SYNGPU/SYNGPU.COM").env("MTOOLS_SKIP_CHECK", "1").output() {
            assert!(o.status.success(), "mtype: {}", String::from_utf8_lossy(&o.stderr));
            assert_eq!(o.stdout, b"hello");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
