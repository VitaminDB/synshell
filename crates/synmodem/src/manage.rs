//! Настройки и сведения модема для «Параметров»: модем и SIM, сота, поиск и выбор сети, технологии, точки
//! доступа (профили WDS), PIN-код, SMS-центр, USSD, услуги вызовов (ожидание, переадресация).

use std::sync::mpsc;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::api::{Apn, CallServices, Cell, Forward, Info, Modes, Network, Slot};
use crate::euicc;
use crate::qmi::{Client, Message, Reader};

const T: Duration = Duration::from_secs(10);

fn string(m: &Message, t: u8) -> String {
    m.get(t).map(|b| String::from_utf8_lossy(b).trim_matches(char::from(0)).trim().to_string()).unwrap_or_default()
}

/// Временный клиент службы (индикации не нужны).
pub fn temp_client(service: u32) -> Result<std::sync::Arc<Client>> {
    let (tx, _rx) = mpsc::channel();
    Client::connect(service, T, tx)
}

// ─── Модем и SIM ────────────────────────────────────────────────────────────

/// Чтение файла SIM (прозрачного) по пути от MF.
fn read_sim_file(uim: &Client, session: u8, path: &[u16], file: u16) -> Option<Vec<u8>> {
    let mut f = file.to_le_bytes().to_vec();
    f.push((path.len() * 2) as u8);
    for p in path {
        f.extend_from_slice(&p.to_le_bytes());
    }
    let r = uim.call(Message::new(0x20).tlv(0x01, vec![session, 0]).tlv(0x02, f).tlv(0x03, vec![0, 0, 0, 0]), T).ok()?;
    let mut rd = r.reader(0x11)?;
    rd.arr16().map(|b| b.to_vec())
}

fn imsi_text(b: &[u8]) -> String {
    let Some((&len, rest)) = b.split_first() else { return String::new() };
    let rest = &rest[..(len as usize).min(rest.len())];
    let mut s = String::new();
    for (i, x) in rest.iter().enumerate() {
        // Первый полубайт — признак чётности, не цифра
        if i > 0 && x & 0x0F < 10 {
            s.push((b'0' + (x & 0x0F)) as char);
        }
        if x >> 4 < 10 {
            s.push((b'0' + (x >> 4)) as char);
        }
    }
    s
}

fn spn_text(b: &[u8]) -> String {
    let name = b.get(1..).unwrap_or(&[]);
    let name: Vec<u8> = name.iter().copied().take_while(|c| *c != 0xFF).collect();
    if name.first() == Some(&0x80) {
        let u: Vec<u16> = name[1..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&u).trim().to_string();
    }
    // Алфавит GSM по умолчанию, по септету в байте (латиница совпадает с ASCII)
    name.iter().map(|c| if c.is_ascii_graphic() || *c == b' ' { *c as char } else { '?' }).collect::<String>().trim().to_string()
}

fn u16_list(m: &Message, t: u8) -> Vec<u16> {
    let Some(mut rd) = m.reader(t) else { return Vec::new() };
    let n = rd.u16().unwrap_or(0);
    (0..n).filter_map(|_| rd.u16()).collect()
}

pub fn slots(uim: &Client) -> (Vec<Slot>, String) {
    let Ok(r) = uim.call(Message::new(0x47), T) else { return (Vec::new(), String::new()) };
    let mut slots = Vec::new();
    if let Some(mut rd) = r.reader(0x10) {
        let n = rd.u8().unwrap_or(0);
        for k in 0..n {
            let (Some(card), Some(active), Some(logical), Some(iccid)) = (rd.u32(), rd.u32(), rd.u8(), rd.arr8()) else { break };
            slots.push(Slot {
                physical: k + 1,
                card: card == 2,
                active: active == 1,
                logical,
                iccid: euicc::iccid_text(iccid),
                euicc: false,
            });
        }
    }
    if let Some(mut rd) = r.reader(0x11) {
        let n = rd.u8().unwrap_or(0);
        for k in 0..n as usize {
            let (Some(_), Some(_), Some(_), Some(e)) = (rd.u32(), rd.u8(), rd.arr8(), rd.u8()) else { break };
            if let Some(s) = slots.get_mut(k) {
                s.euicc = e != 0;
            }
        }
    }
    let mut eid = String::new();
    if let Some(mut rd) = r.reader(0x12) {
        let n = rd.u8().unwrap_or(0);
        for _ in 0..n {
            if let Some(e) = rd.arr8() {
                if eid.is_empty() && !e.is_empty() {
                    eid = e.iter().map(|b| format!("{b:02X}")).collect();
                }
            }
        }
    }
    (slots, eid)
}

pub fn info(dms: &Client, uim: &Client) -> Info {
    let mut i = Info::default();
    if let Ok(r) = dms.call(Message::new(0x25), T) {
        i.imei = string(&r, 0x11);
        i.meid = string(&r, 0x12);
        i.imei_sv = string(&r, 0x13);
    }
    let s = |id| dms.call(Message::new(id), T).map(|r| string(&r, 0x01)).unwrap_or_default();
    i.revision = s(0x23);
    i.msisdn = s(0x24);
    i.hw_revision = s(0x2C);
    i.sw_version = s(0x51);
    if let Ok(r) = dms.call(Message::new(0x45), T) {
        i.lte_bands = u16_list(&r, 0x12);
        i.nr_bands = u16_list(&r, 0x13);
    }
    let (slots, eid) = slots(uim);
    i.iccid = slots.iter().find(|s| s.card && s.active && s.logical == 1).map(|s| s.iccid.clone()).unwrap_or_default();
    i.eid = eid;
    i.slots = slots;
    // Файлы USIM: IMSI (6F07) и имя оператора (6F46) в ADF (путь 3F00/7FFF), основная подписка
    if let Some(b) = read_sim_file(uim, 0, &[0x3F00, 0x7FFF], 0x6F07) {
        i.imsi = imsi_text(&b);
    }
    if let Some(b) = read_sim_file(uim, 0, &[0x3F00, 0x7FFF], 0x6F46) {
        i.spn = spn_text(&b);
    }
    i
}

// ─── Сота ───────────────────────────────────────────────────────────────────

fn band_name(b: u16) -> String {
    const LTE: &[(u16, u16)] = &[
        (120, 1), (121, 2), (122, 3), (123, 4), (124, 5), (125, 6), (126, 7), (127, 8), (128, 9), (129, 10), (130, 11),
        (131, 12), (132, 13), (133, 14), (134, 17), (143, 18), (144, 19), (145, 20), (146, 21), (152, 23), (147, 24),
        (148, 25), (153, 26), (164, 27), (158, 28), (159, 29), (160, 30), (165, 31), (154, 32), (135, 33), (136, 34),
        (137, 35), (138, 36), (139, 37), (140, 38), (141, 39), (142, 40), (149, 41), (150, 42), (151, 43), (163, 46),
        (166, 47), (167, 48), (161, 66), (168, 71),
    ];
    const NR: &[(u16, u16)] = &[
        (250, 1), (251, 2), (252, 3), (253, 5), (254, 7), (255, 8), (256, 20), (257, 28), (258, 38), (259, 41), (260, 50),
        (261, 51), (262, 66), (263, 70), (264, 71), (265, 74), (266, 75), (267, 76), (268, 77), (269, 78), (270, 79),
        (271, 80), (272, 81), (273, 82), (274, 83), (275, 84), (276, 85), (277, 257), (278, 258), (279, 259), (280, 260),
        (281, 261), (282, 12), (283, 25), (284, 34), (285, 39), (286, 40), (287, 65), (288, 86), (289, 48), (290, 14),
        (291, 13), (292, 18), (293, 26), (294, 30), (295, 29), (296, 53), (297, 46),
    ];
    if let Some((_, n)) = LTE.iter().find(|(k, _)| *k == b) {
        return format!("B{n}");
    }
    if let Some((_, n)) = NR.iter().find(|(k, _)| *k == b) {
        return format!("n{n}");
    }
    match b {
        40..=48 => ["GSM 450", "GSM 480", "GSM 750", "GSM 850", "GSM 900", "GSM 900", "GSM 900", "GSM 1800", "GSM 1900"][(b - 40) as usize].into(),
        80..=91 => format!("UMTS ({b})"),
        _ => format!("{b}"),
    }
}

fn bandwidth(v: u32) -> Option<f32> {
    Some(match v {
        0 => 1.4,
        1 => 3.0,
        2 | 6 | 22 => 5.0,
        3 | 7 | 23 => 10.0,
        4 | 8 => 15.0,
        5 | 9 => 20.0,
        10 => 25.0,
        11 => 30.0,
        12 => 40.0,
        13 => 50.0,
        14 => 60.0,
        24 => 70.0,
        15 => 80.0,
        16 => 90.0,
        17 => 100.0,
        18 => 200.0,
        19 => 400.0,
        20 => 0.2,
        _ => return None,
    })
}

pub fn cell(nas: &Client, technology: &str, plmn: &str) -> Cell {
    let mut c = Cell { technology: technology.into(), plmn: plmn.into(), ..Cell::default() };
    if let Ok(r) = nas.call(Message::new(0x24), T) {
        c.lac = r.reader(0x1C).and_then(|mut rd| rd.u16()).map(u32::from);
        c.cell_id = r.reader(0x1D).and_then(|mut rd| rd.u32());
        c.tac = r.reader(0x24).and_then(|mut rd| rd.u16()).map(u32::from);
    }
    if let Ok(r) = nas.call(Message::new(0x4F), T) {
        if let Some(mut rd) = r.reader(0x14) {
            c.rssi = rd.i8().map(i32::from);
            c.rsrq = rd.i8().map(i32::from);
            c.rsrp = rd.i16().map(i32::from);
            c.snr = rd.i16().map(|v| f32::from(v) / 10.0);
        }
        if let Some(mut rd) = r.reader(0x17) {
            if technology == "5G" {
                c.rsrp = rd.i16().map(i32::from);
                c.snr = rd.i16().map(|v| f32::from(v) / 10.0);
            }
        }
        if c.rssi.is_none() {
            c.rssi = r.get(0x12).and_then(|b| b.first()).map(|v| i32::from(*v as i8)).or_else(|| r.reader(0x13).and_then(|mut rd| rd.i8()).map(i32::from));
        }
    }
    if let Ok(r) = nas.call(Message::new(0x31), T) {
        let mut bands = Vec::new();
        if let Some(mut rd) = r.reader(0x11) {
            let n = rd.u8().unwrap_or(0);
            for _ in 0..n {
                let (Some(_), Some(b), Some(ch)) = (rd.i8(), rd.u16(), rd.u32()) else { break };
                bands.push((b, ch));
            }
        } else if let Some(mut rd) = r.reader(0x01) {
            let n = rd.u8().unwrap_or(0);
            for _ in 0..n {
                let (Some(_), Some(b), Some(ch)) = (rd.i8(), rd.u16(), rd.u16()) else { break };
                bands.push((b, u32::from(ch)));
            }
        }
        // 5G NSA: LTE-якорь и NR — обе полосы
        c.band = bands.iter().map(|(b, _)| band_name(*b)).collect::<Vec<_>>().join(" + ");
        c.channel = bands.first().map(|(_, ch)| *ch);
        if let Some(mut rd) = r.reader(0x12) {
            if rd.u8().unwrap_or(0) > 0 {
                rd.i8();
                c.bandwidth_mhz = rd.u32().and_then(bandwidth);
            }
        }
    }
    c
}

// ─── Сети и технологии ──────────────────────────────────────────────────────

fn rat_name(r: i8) -> &'static str {
    match r {
        4 => "2G",
        5 | 9 => "3G",
        8 => "LTE",
        12 => "5G",
        _ => "",
    }
}

pub fn scan(nas: &Client) -> Result<Vec<Network>> {
    let r = nas.call(Message::new(0x21), Duration::from_secs(300)).context("поиск сетей")?;
    let mut out: Vec<Network> = Vec::new();
    if let Some(mut rd) = r.reader(0x10) {
        let n = rd.u16().unwrap_or(0);
        for _ in 0..n {
            let (Some(mcc), Some(mnc), Some(st), Some(name)) = (rd.u16(), rd.u16(), rd.u8(), rd.str8()) else { break };
            match out.iter_mut().find(|x| x.mcc == mcc && x.mnc == mnc) {
                Some(x) => {
                    x.current |= st & 1 != 0;
                    if x.name.is_empty() {
                        x.name = name;
                    }
                }
                None => out.push(Network {
                    mcc,
                    mnc,
                    name,
                    technologies: Vec::new(),
                    current: st & 1 != 0,
                    forbidden: st & 0x10 != 0,
                    home: st & 0x04 != 0,
                }),
            }
        }
    }
    if let Some(mut rd) = r.reader(0x11) {
        let n = rd.u16().unwrap_or(0);
        for _ in 0..n {
            let (Some(mcc), Some(mnc), Some(rat)) = (rd.u16(), rd.u16(), rd.i8()) else { break };
            if let Some(x) = out.iter_mut().find(|x| x.mcc == mcc && x.mnc == mnc) {
                let t = rat_name(rat).to_string();
                if !t.is_empty() && !x.technologies.contains(&t) {
                    x.technologies.push(t);
                }
            }
        }
    }
    Ok(out)
}

pub fn select(nas: &Client, plmn: Option<(u16, u16)>) -> Result<()> {
    let m = match plmn {
        None => Message::new(0x22).u8(0x01, 1),
        Some((mcc, mnc)) => {
            let mut v = mcc.to_le_bytes().to_vec();
            v.extend_from_slice(&mnc.to_le_bytes());
            // Технологию выберет модем (LTE — самая частая; NO_CHANGE сеть не принимает)
            v.push(8);
            Message::new(0x22).u8(0x01, 2).tlv(0x10, v).u8(0x11, 1)
        }
    };
    nas.call(m, Duration::from_secs(60)).context("регистрация в сети")?;
    Ok(())
}

const MODE_BITS: &[(&str, u16)] = &[("2g", 4), ("3g", 8), ("4g", 16), ("5g", 64)];

pub fn modes(nas: &Client) -> Result<Modes> {
    let r = nas.call(Message::new(0x34), T)?;
    let bits = r.reader(0x11).and_then(|mut rd| rd.u16()).unwrap_or(0);
    let manual = match r.get(0x16).and_then(|b| b.first()) {
        Some(1) => r.reader(0x1B).and_then(|mut rd| Some((rd.u16()?, rd.u16()?))),
        _ => None,
    };
    Ok(Modes {
        allowed: MODE_BITS.iter().filter(|(_, b)| bits & b != 0).map(|(n, _)| n.to_string()).collect(),
        manual,
        data_roaming: false,
    })
}

pub fn set_modes(nas: &Client, allowed: &[String]) -> Result<()> {
    let bits: u16 = MODE_BITS.iter().filter(|(n, _)| allowed.iter().any(|a| a == n)).map(|(_, b)| b).sum();
    if bits == 0 {
        bail!("нужна хотя бы одна технология");
    }
    // Порядок поиска: от новых к старым
    let order: Vec<u8> = [(64u16, 12u8), (16, 8), (8, 5), (4, 4)].iter().filter(|(b, _)| bits & b != 0).map(|(_, r)| *r).collect();
    let mut acq = vec![order.len() as u8];
    acq.extend(order);
    nas.call(Message::new(0x33).u16(0x11, bits).u8(0x17, 1).tlv(0x1E, acq), T)?;
    Ok(())
}

// ─── Точки доступа ──────────────────────────────────────────────────────────

fn apn_of(wds: &Client, index: u8, default: u8) -> Option<Apn> {
    let r = wds.call(Message::new(0x2B).tlv(0x01, vec![0, index]), T).ok()?;
    let auth = r.get(0x1D).and_then(|b| b.first().copied()).unwrap_or(0);
    let pdp = r.get(0x11).and_then(|b| b.first().copied()).unwrap_or(0);
    Some(Apn {
        index,
        name: string(&r, 0x10),
        apn: string(&r, 0x14),
        user: string(&r, 0x1B),
        password: string(&r, 0x1C),
        auth: match auth & 3 {
            1 => "pap",
            2 => "chap",
            3 => "pap-chap",
            _ => "none",
        }
        .into(),
        ip: match pdp {
            2 => "ipv6",
            3 => "ipv4v6",
            _ => "ipv4",
        }
        .into(),
        default: index == default,
        roaming_disallowed: r.get(0x3E).and_then(|b| b.first()).is_some_and(|v| *v != 0),
    })
}

pub fn default_profile(wds: &Client) -> u8 {
    wds.call(Message::new(0x49).tlv(0x01, vec![0, 0]), T).ok().and_then(|r| r.get(0x01).and_then(|b| b.first().copied())).unwrap_or(1)
}

pub fn apns(wds: &Client) -> Result<Vec<Apn>> {
    let r = wds.call(Message::new(0x2A).u8(0x10, 0), T)?;
    let default = default_profile(wds);
    let mut out = Vec::new();
    if let Some(mut rd) = r.reader(0x01) {
        let n = rd.u8().unwrap_or(0);
        let mut idx = Vec::new();
        for _ in 0..n {
            let (Some(_t), Some(i), Some(_name)) = (rd.u8(), rd.u8(), rd.str8()) else { break };
            idx.push(i);
        }
        for i in idx {
            if let Some(a) = apn_of(wds, i, default) {
                out.push(a);
            }
        }
    }
    Ok(out)
}

fn apn_fields(m: Message, a: &Apn) -> Message {
    let auth = match a.auth.as_str() {
        "pap" => 1,
        "chap" => 2,
        "pap-chap" => 3,
        _ => 0,
    };
    let pdp = match a.ip.as_str() {
        "ipv6" => 2,
        "ipv4v6" => 3,
        _ => 0,
    };
    m.tlv(0x10, a.name.as_bytes().to_vec())
        .u8(0x11, pdp)
        .tlv(0x14, a.apn.as_bytes().to_vec())
        .tlv(0x1B, a.user.as_bytes().to_vec())
        .tlv(0x1C, a.password.as_bytes().to_vec())
        .u8(0x1D, auth)
        .u8(0x3E, u8::from(a.roaming_disallowed))
}

pub fn save_apn(wds: &Client, a: &Apn) -> Result<u8> {
    let index = if a.index == 0 {
        let r = wds.call(apn_fields(Message::new(0x27).u8(0x01, 0), a), T).context("новая точка доступа")?;
        r.reader(0x01).and_then(|mut rd| {
            rd.u8();
            rd.u8()
        })
        .context("модем не вернул номер профиля")?
    } else {
        wds.call(apn_fields(Message::new(0x28).tlv(0x01, vec![0, a.index]), a), T).context("изменение точки доступа")?;
        a.index
    };
    if a.default {
        wds.call(Message::new(0x4A).tlv(0x01, vec![0, 0, index]), T).context("точка доступа по умолчанию")?;
    }
    Ok(index)
}

pub fn delete_apn(wds: &Client, index: u8) -> Result<()> {
    wds.call(Message::new(0x29).tlv(0x01, vec![0, index]), T).context("удаление точки доступа")?;
    Ok(())
}

// ─── PIN ────────────────────────────────────────────────────────────────────

fn pin_result(r: Result<Message>) -> Result<()> {
    let r = r?;
    if let Err(e) = r.result() {
        let left = r.reader(0x10).and_then(|mut rd| Some((rd.u8()?, rd.u8()?)));
        match left {
            Some((v, u)) => bail!("{e}; осталось попыток PIN: {v}, PUK: {u}"),
            None => bail!("{e}"),
        }
    }
    Ok(())
}

fn str8(s: &str) -> Vec<u8> {
    let mut v = vec![s.len() as u8];
    v.extend_from_slice(s.as_bytes());
    v
}

const SESSION: [u8; 2] = [0, 0];

pub fn pin_enable(uim: &Client, on: bool, pin: &str) -> Result<()> {
    let info = [vec![1, u8::from(on)], str8(pin)].concat();
    pin_result(uim.call_raw(Message::new(0x25).tlv(0x01, SESSION.to_vec()).tlv(0x02, info), T))
}

pub fn pin_change(uim: &Client, old: &str, new: &str) -> Result<()> {
    let info = [vec![1], str8(old), str8(new)].concat();
    pin_result(uim.call_raw(Message::new(0x28).tlv(0x01, SESSION.to_vec()).tlv(0x02, info), T))
}

pub fn pin_verify(uim: &Client, pin: &str) -> Result<()> {
    let info = [vec![1], str8(pin)].concat();
    pin_result(uim.call_raw(Message::new(0x26).tlv(0x01, SESSION.to_vec()).tlv(0x02, info), T))
}

pub fn pin_unblock(uim: &Client, puk: &str, new: &str) -> Result<()> {
    let info = [vec![1], str8(puk), str8(new)].concat();
    pin_result(uim.call_raw(Message::new(0x27).tlv(0x01, SESSION.to_vec()).tlv(0x02, info), T))
}

// ─── SMS-центр ──────────────────────────────────────────────────────────────

pub fn smsc(wms: &Client) -> Result<String> {
    let r = wms.call(Message::new(0x34), T)?;
    let mut rd = r.reader(0x01).context("нет адреса SMS-центра")?;
    rd.bytes(3);
    Ok(rd.str8().unwrap_or_default())
}

pub fn set_smsc(wms: &Client, number: &str) -> Result<()> {
    let n = number.trim();
    let kind = if n.starts_with('+') { b"145" } else { b"129" };
    wms.call(Message::new(0x35).tlv(0x01, n.as_bytes().to_vec()).tlv(0x10, kind.to_vec()), T)?;
    Ok(())
}

// ─── USSD ───────────────────────────────────────────────────────────────────

pub fn ussd_start(voice: &Client, code: &str) -> Result<()> {
    let data = [vec![1u8], str8(code)].concat();
    voice.call(Message::new(0x43).tlv(0x01, data), T).context("USSD")?;
    Ok(())
}

pub fn ussd_answer(voice: &Client, text: &str) -> Result<()> {
    let data = [vec![1u8], str8(text)].concat();
    voice.call(Message::new(0x3B).tlv(0x01, data), Duration::from_secs(60)).context("ответ USSD")?;
    Ok(())
}

pub fn ussd_cancel(voice: &Client) -> Result<()> {
    voice.call(Message::new(0x3C), T)?;
    Ok(())
}

/// Текст USSD из индикации: UTF-16 (если есть) или данные в кодировке DCS.
pub fn ussd_text(m: &Message, data_tlv: u8, utf16_tlv: u8) -> Option<String> {
    if let Some(mut rd) = m.reader(utf16_tlv) {
        let n = rd.u8()? as usize;
        let units: Vec<u16> = (0..n).filter_map(|_| rd.u16()).collect();
        return Some(String::from_utf16_lossy(&units));
    }
    let mut rd: Reader = m.reader(data_tlv)?;
    let dcs = rd.u8()?;
    let data = rd.arr8()?;
    Some(match dcs {
        3 => {
            let u: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u)
        }
        _ => String::from_utf8_lossy(data).into_owned(),
    })
}

// ─── Услуги вызовов ─────────────────────────────────────────────────────────

const FORWARDS: &[(&str, u8)] = &[("always", 1), ("busy", 2), ("no-reply", 3), ("unreachable", 4)];

pub fn call_services(voice: &Client) -> CallServices {
    let mut s = CallServices::default();
    match voice.call(Message::new(0x34).u8(0x10, 1), Duration::from_secs(30)) {
        Ok(r) => s.waiting = r.get(0x10).and_then(|b| b.first()).map(|c| c & 1 != 0),
        Err(e) => {
            tracing::warn!("ожидание вызова: {e:#}");
            s.error = format!("{e:#}");
        }
    }
    for (name, reason) in FORWARDS {
        let mut f = Forward { reason: name.to_string(), ..Forward::default() };
        let q = voice.call(Message::new(0x38).u8(0x01, *reason), Duration::from_secs(30));
        if let Err(e) = &q {
            tracing::warn!("переадресация {name}: {e:#}");
            if s.error.is_empty() {
                s.error = format!("{e:#}");
            }
        }
        if let Ok(r) = q {
            tracing::debug!("переадресация {name}: {:02x?}", r.tlvs);
            if let Some(mut rd) = r.reader(0x10) {
                let n = rd.u8().unwrap_or(0);
                for _ in 0..n {
                    let (Some(st), Some(class), Some(num), Some(timer)) = (rd.u8(), rd.u8(), rd.str8(), rd.u8()) else { break };
                    // Голосовые вызовы — класс 1
                    if class & 1 != 0 || class == 0 {
                        f.active = st != 0;
                        f.number = num;
                        f.timer = (timer > 0).then_some(timer);
                    }
                }
            }
        }
        s.forwards.push(f);
    }
    s
}

pub fn set_call_waiting(voice: &Client, on: bool) -> Result<()> {
    voice.call(Message::new(0x33).tlv(0x01, vec![if on { 1 } else { 2 }, 0x0F]).u8(0x10, 1), Duration::from_secs(30))?;
    Ok(())
}

pub fn set_forward(voice: &Client, reason: &str, number: &str, timer: Option<u8>) -> Result<()> {
    let code = FORWARDS.iter().find(|(n, _)| *n == reason).map(|(_, c)| *c).context("неизвестная переадресация")?;
    let n = number.trim();
    let mut m = Message::new(0x33).tlv(0x01, vec![if n.is_empty() { 4 } else { 3 }, code]).u8(0x10, 1);
    if !n.is_empty() {
        m = m.tlv(0x12, n.as_bytes().to_vec());
        if let Some(t) = timer.filter(|_| code == 3) {
            m = m.u8(0x13, t);
        }
    }
    voice.call(m, Duration::from_secs(30))?;
    Ok(())
}

/// Номер с учётом скрытия своего номера (MMI: #31# — скрыть, *31# — показать).
pub fn with_clir(number: &str, mode: &str) -> String {
    match mode {
        "hide" => format!("#31#{number}"),
        "show" => format!("*31#{number}"),
        _ => number.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imsi_decode() {
        // IMSI 250021234567890: длина 8, первый байт — 2 в старшем полубайте и признак 9
        let b = [0x08, 0x29, 0x05, 0x20, 0x21, 0x43, 0x65, 0x87, 0x09];
        assert_eq!(imsi_text(&b), "250021234567890");
    }

    #[test]
    fn bands() {
        assert_eq!(band_name(122), "B3");
        assert_eq!(band_name(269), "n78");
        assert_eq!(band_name(47), "GSM 1800");
    }

    #[test]
    fn spn() {
        assert_eq!(spn_text(&[0x01, b'M', b'e', b'g', b'a', 0xFF, 0xFF]), "Mega");
    }
}
