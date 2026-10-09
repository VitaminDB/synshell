//! Демон `synmodemd`: подключается к службам QMI модема (DMS, NAS, UIM, WMS, Voice), поднимает подписку SIM,
//! держит состояние сети, принимает и отправляет SMS, ведёт звонки и журнал. Модем упал или остановлен —
//! ждёт его возвращения и поднимает всё заново.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};

use crate::api::{
    self, Call, CallKind, CallRecord, CallState, DataState, Event, Registration, Request, Response, SimState, Sms,
    SmsStatus, Status,
};
use crate::data;
use crate::euicc;
use crate::gnss;
use crate::manage;
use crate::pdu;
use crate::qmi::{svc, Client, Indication, Message, QmiError, SERVICE_GONE};
use crate::store::{Part, Store};
use synshell_tr::t;

const T: Duration = Duration::from_secs(10);
/// Хук звука разговора платформы: `call-audio start|stop` (маршрут голоса в DSP зависит от устройства).
const CALL_AUDIO_HOOK: &str = "/usr/lib/synmodem/call-audio";

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Клиенты служб работающего модема.
struct Modem {
    dms: Arc<Client>,
    nas: Arc<Client>,
    uim: Arc<Client>,
    wms: Arc<Client>,
    voice: Arc<Client>,
}

#[derive(Default)]
struct Inner {
    status: Status,
    /// Звонки, которые сбросил пользователь (входящий без ответа — «отклонён», а не «пропущен»).
    rejected: HashSet<u8>,
    /// Когда звонок перешёл в разговор.
    answered: HashMap<u8, i64>,
    started: HashMap<u8, i64>,
    /// Звук разговора включён хуком.
    call_audio: bool,
}

pub struct Daemon {
    inner: Mutex<Inner>,
    modem: Mutex<Option<Arc<Modem>>>,
    store: Mutex<Store>,
    subs: Mutex<Vec<Sender<Event>>>,
    /// Номер склейки исходящих длинных SMS.
    sms_ref: std::sync::atomic::AtomicU8,
    /// Сеанс передачи данных.
    data: Mutex<Option<data::Session>>,
    /// Идёт подключение данных.
    data_busy: std::sync::atomic::AtomicBool,
    /// Каналы IPA и формат модема настроены (на этот запуск модема).
    data_ready: std::sync::atomic::AtomicBool,
    /// Канал индикаций текущего запуска модема (для клиентов сеанса данных).
    ind_tx: Mutex<Option<Sender<Indication>>>,
    /// Приёмник GNSS (служба LOC, свой клиент — работает, пока есть читатели).
    gnss: std::sync::OnceLock<Arc<gnss::Engine>>,
}

impl Daemon {
    fn modem(&self) -> Result<Arc<Modem>> {
        self.modem.lock().unwrap().clone().ok_or_else(|| anyhow!("модем не запущен"))
    }

    fn emit(&self, ev: Event) {
        self.subs.lock().unwrap().retain(|s| s.send(ev.clone()).is_ok());
    }

    /// Изменить состояние; событие — только если оно действительно изменилось.
    fn update(&self, f: impl FnOnce(&mut Status)) {
        let st = {
            let mut i = self.inner.lock().unwrap();
            let before = i.status.clone();
            f(&mut i.status);
            if i.status == before {
                return;
            }
            i.status.clone()
        };
        self.emit(Event::Status { status: st });
    }

    fn refresh_unread(&self) {
        let unread = {
            let s = self.store.lock().unwrap();
            s.sms.iter().filter(|m| m.incoming && !m.read).count() as u32
        };
        self.update(|s| s.unread_sms = unread);
    }

    // ── Подключение и подготовка модема ─────────────────────────────────────────────────────────

    fn connect(&self, ind: &Sender<Indication>) -> Result<Modem> {
        // DMS появляется последней из нужных не всегда — ждём каждую
        let c = |s| Client::connect(s, Duration::from_secs(30), ind.clone());
        Ok(Modem { dms: c(svc::DMS)?, nas: c(svc::NAS)?, uim: c(svc::UIM)?, wms: c(svc::WMS)?, voice: c(svc::VOICE)? })
    }

    fn setup(&self, m: &Modem) {
        if let Ok(r) = m.dms.call(Message::new(0x25), T) {
            let imei = r.get(0x11).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            self.update(|s| s.imei = imei);
        }
        // Индикации: карта и слоты (UIM), сеть/сигнал/отказ (NAS), SMS (WMS), звонки (Voice)
        let _ = m.uim.call(Message::new(0x2E).u32(0x01, 1 | 1 << 4), T);
        let _ = m
            .nas
            .call(Message::new(0x03).u8(0x13, 1).u8(0x19, 1).tlv(0x21, vec![1, 0]).u8(0x18, 0).u8(0x10, 0), T);
        let rssi: Vec<u8> = [-103i8, -93, -83, -73].iter().map(|v| *v as u8).collect();
        let rsrp: Vec<u8> = [-115i16, -105, -95, -85].iter().flat_map(|v| v.to_le_bytes()).collect();
        let _ = m.nas.call(
            Message::new(0x50)
                .tlv(0x10, [vec![4u8], rssi].concat())
                .tlv(0x16, [vec![4u8], rsrp].concat()),
            T,
        );
        // Все сети: по умолчанию в модеме бывает «cdma-1x, gsm» (только 2G)
        if let Err(e) = m.nas.call(Message::new(0x33).u16(0x11, 4 | 8 | 16 | 64), T) {
            tracing::warn!("предпочтение сетей: {e:#}");
        }
        let _ = m.dms.call(Message::new(0x01).u8(0x14, 1), T);
        let radio_off = self.store.lock().unwrap().radio_off;
        // Сразу после старта модем отвечает DeviceUnsupported — повторять, пока не примет
        for attempt in 0..10 {
            match m.dms.call(Message::new(0x2E).u8(0x01, if radio_off { 1 } else { 0 }), T) {
                Ok(_) => break,
                Err(e) if attempt == 9 => tracing::warn!("режим модема: {e:#}"),
                Err(_) => std::thread::sleep(Duration::from_secs(2)),
            }
        }
        self.setup_sms(m);
        let _ = m.voice.call(Message::new(0x03).u8(0x13, 1).u8(0x16, 1), T);
        self.present_physical_slot(m);
        self.ensure_sim(m);
        self.refresh_network(m);
        self.refresh_calls(m);
    }

    /// Подписка SIM: первая карта с USIM — основная. Модем DSDS по умолчанию ждёт основную SIM в логическом
    /// слоте 1, а eSIM diting — в физическом слоте 2; приложение после переназначения иногда застревает в
    /// «detected» — помогает перезапуск питания карты.
    fn ensure_sim(&self, m: &Modem) {
        let mut cycled = false;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut provisioned_at: Option<Instant> = None;
        while Instant::now() < deadline {
            let Ok(cs) = m.uim.call(Message::new(0x2F), T).map(|r| parse_card_status(&r, 0x10)) else {
                return;
            };
            let Some(cs) = cs else { return };
            self.apply_card_status(&cs);
            if let Some((slot, app)) = cs.primary_gw {
                match cs.cards.get(slot).and_then(|c| c.apps.get(app)).map(|a| a.state) {
                    Some(7) => return,
                    // PIN/PUK — ждать пользователя
                    Some(2..=6) => return,
                    _ => {}
                }
                let since = *provisioned_at.get_or_insert_with(Instant::now);
                if !cycled && since.elapsed() > Duration::from_secs(10) {
                    tracing::info!("USIM не готовится — перезапуск питания карты слота {}", slot + 1);
                    let _ = m.uim.call(Message::new(0x30).u8(0x01, slot as u8 + 1), T);
                    std::thread::sleep(Duration::from_secs(2));
                    let _ = m.uim.call(Message::new(0x31).u8(0x01, slot as u8 + 1), T);
                    cycled = true;
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            // Основной подписки нет: логический слот 1 пуст, а карта есть в другом физическом — переназначить
            let first_ok = cs.cards.first().is_some_and(|c| c.state == 1);
            if !first_ok {
                if let Some(phys) = self.present_physical_slot(m) {
                    tracing::info!("основная SIM: логический слот 1 → физический {phys}");
                    let _ = m.uim.call_raw(Message::new(0x46).u8(0x01, 1).u32(0x02, phys), Duration::from_secs(30));
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
                return;
            }
            let Some(app) = cs.cards[0].apps.iter().find(|a| a.kind == 2) else { return };
            tracing::info!("подписка: USIM слота 1");
            let mut info = vec![1u8, app.aid.len() as u8];
            info.extend_from_slice(&app.aid);
            let _ = m.uim.call_raw(Message::new(0x38).tlv(0x01, vec![0, 1]).tlv(0x10, info), Duration::from_secs(30));
            provisioned_at = Some(Instant::now());
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    /// Физический слот (с 1) с картой, если логический слот 1 пуст; заодно — признак eSIM у карты.
    fn present_physical_slot(&self, m: &Modem) -> Option<u32> {
        let r = m.uim.call(Message::new(0x47), T).ok()?;
        let mut rd = r.reader(0x10)?;
        let n = rd.u8()?;
        // (карта есть, логический слот) по физическим слотам; карта: 1 — нет, 2 — есть; слот: 1 — активен
        let mut slots = Vec::new();
        for _ in 0..n {
            let card = rd.u32()?;
            let active = rd.u32()?;
            let logical = rd.u8()?;
            rd.arr8()?;
            slots.push((card == 2, if active == 1 { logical } else { 0 }));
        }
        let euicc: Vec<bool> = r
            .reader(0x11)
            .and_then(|mut rd| {
                let n = rd.u8()?;
                let mut v = Vec::new();
                for _ in 0..n {
                    rd.u32()?;
                    rd.u8()?;
                    rd.arr8()?;
                    v.push(rd.u8()? != 0);
                }
                Some(v)
            })
            .unwrap_or_default();
        let used = slots.iter().position(|(c, l)| *c && *l == 1).or_else(|| slots.iter().position(|(c, _)| *c));
        let esim = used.and_then(|k| euicc.get(k).copied()).unwrap_or(false);
        self.update(|s| s.sim.esim = esim);
        if slots.iter().any(|(c, l)| *c && *l == 1) {
            return None;
        }
        used.map(|k| k as u32 + 1)
    }

    fn apply_card_status(&self, cs: &CardStatus) {
        let state = cs.sim_state();
        let pin = cs.primary_app().map(|a| a.pin1_retries);
        // Состояние PIN1: 1/2 — включён (не введён/введён), 3 — выключен
        let pin_on = cs.primary_app().and_then(|a| match a.pin1_state {
            1 | 2 | 4 | 5 => Some(true),
            3 => Some(false),
            _ => None,
        });
        self.update(|s| {
            s.sim.state = state;
            s.sim.pin_retries = pin;
            s.sim.pin_enabled = pin_on;
        });
    }

    fn refresh_network(&self, m: &Modem) {
        if let Ok(r) = m.nas.call(Message::new(0x24), T) {
            self.apply_serving_system(m, &r, false);
        }
        if let Ok(r) = m.nas.call(Message::new(0x4F), T) {
            self.apply_signal(&r);
        }
        if let Ok(r) = m.dms.call(Message::new(0x2D), T) {
            if let Some(mode) = r.get(0x01).and_then(|b| b.first()) {
                self.update(|s| s.radio = *mode == 0);
            }
        }
        // Оператор SIM
        if let Ok(r) = m.nas.call(Message::new(0x25), T) {
            if let Some(mut rd) = r.reader(0x01) {
                let (mcc, mnc) = (rd.u16().unwrap_or(0), rd.u16().unwrap_or(0));
                let desc = rd.str8().unwrap_or_default();
                let name = plmn_name(m, mcc, mnc).filter(|n| !n.is_empty()).unwrap_or(desc);
                self.update(|s| s.sim.home_operator = name);
            }
        }
    }

    fn apply_serving_system(&self, m: &Modem, r: &Message, indication: bool) {
        let Some(mut rd) = r.reader(0x01) else { return };
        let (Some(reg), Some(_cs), Some(_ps), Some(_net)) = (rd.u8(), rd.u8(), rd.u8(), rd.u8()) else { return };
        let rats: Vec<u8> = rd.arr8().map(|a| a.to_vec()).unwrap_or_default();
        let prev = self.inner.lock().unwrap().status.clone();
        // В индикации TLV роуминга и сети бывают только при изменении — иначе прежние
        let roaming = match r.get(0x10).and_then(|b| b.first()) {
            Some(v) => *v == 0,
            None if indication => prev.roaming || prev.registration == Registration::Roaming,
            None => false,
        };
        let detailed = r.get(if indication { 0x22 } else { 0x21 }).and_then(|b| b.first().copied());
        let limited = matches!(detailed, Some(1 | 3));
        let registration = match reg {
            1 if roaming => Registration::Roaming,
            1 => Registration::Home,
            3 => Registration::Denied,
            0 | 2 if limited => Registration::Limited,
            2 => Registration::Searching,
            0 => Registration::NotRegistered,
            _ => Registration::Unknown,
        };
        let technology = if rats.contains(&0x0C) {
            "5G"
        } else if rats.contains(&0x08) {
            "LTE"
        } else if rats.iter().any(|r| matches!(r, 0x05 | 0x09)) {
            "3G"
        } else if rats.contains(&0x04) {
            "2G"
        } else {
            ""
        };
        let plmn = r.reader(0x12).and_then(|mut rd| Some((rd.u16()?, rd.u16()?, rd.str8().unwrap_or_default())));
        let (plmn_text, operator) = match &plmn {
            Some((mcc, mnc, desc)) => {
                let text = format!("{mcc:03}/{mnc:02}");
                let name = if text == prev.plmn && !prev.operator.is_empty() {
                    prev.operator.clone()
                } else {
                    plmn_name(m, *mcc, *mnc).filter(|n| !n.is_empty()).unwrap_or_else(|| desc.clone())
                };
                (text, name)
            }
            None if indication && reg != 0 => (prev.plmn.clone(), prev.operator.clone()),
            None => (String::new(), String::new()),
        };
        self.update(|s| {
            s.registration = registration;
            s.technology = if technology.is_empty() || plmn_text.is_empty() { String::new() } else { technology.into() };
            s.roaming = roaming && registration.registered();
            s.plmn = plmn_text;
            s.operator = operator;
            if registration.registered() {
                s.reject_cause = None;
            }
            if s.plmn.is_empty() {
                s.bars = None;
                s.dbm = None;
            }
        });
    }

    fn apply_signal(&self, r: &Message) {
        let tech = self.inner.lock().unwrap().status.technology.clone();
        let lte = r.reader(0x14).and_then(|mut rd| {
            rd.i8()?;
            rd.i8()?;
            rd.i16()
        });
        let nr = r.reader(0x17).and_then(|mut rd| rd.i16());
        let wcdma = r.reader(0x13).and_then(|mut rd| rd.i8());
        let gsm = r.get(0x12).and_then(|b| b.first()).map(|v| *v as i8);
        let rsrp_bars = |v: i32| match v {
            v if v >= -85 => 4,
            v if v >= -95 => 3,
            v if v >= -105 => 2,
            v if v >= -115 => 1,
            _ => 0,
        };
        let rssi_bars = |v: i32| match v {
            v if v >= -73 => 4,
            v if v >= -83 => 3,
            v if v >= -93 => 2,
            v if v >= -103 => 1,
            _ => 0,
        };
        let pick = match tech.as_str() {
            "5G" => nr.or(lte).map(|v| (v as i32, true)),
            "LTE" => lte.map(|v| (v as i32, true)),
            "3G" => wcdma.map(|v| (v as i32, false)),
            "2G" => gsm.map(|v| (v as i32, false)),
            _ => None,
        }
        .or(lte.map(|v| (v as i32, true)))
        .or(wcdma.or(gsm).map(|v| (v as i32, false)));
        self.update(|s| {
            if let Some((dbm, rsrp)) = pick {
                s.dbm = Some(dbm);
                s.bars = Some(if rsrp { rsrp_bars(dbm) } else { rssi_bars(dbm) });
            } else {
                s.dbm = None;
                s.bars = None;
            }
        });
    }

    // ── SMS ─────────────────────────────────────────────────────────────────────────────────────

    fn setup_sms(&self, m: &Modem) {
        // Все классы — в память модема с уведомлением: SMS не теряется, пока демон не забрал его к себе
        let mut routes = vec![];
        routes.extend_from_slice(&5u16.to_le_bytes());
        for class in 0..=4u8 {
            routes.extend_from_slice(&[0, class, 1, 1]);
        }
        if let Err(e) = m.wms.call(Message::new(0x32).tlv(0x01, routes), T) {
            tracing::warn!("маршруты SMS: {e:#}");
        }
        let _ = m.wms.call(Message::new(0x01).u8(0x10, 1), T);
        for storage in [1u8, 0] {
            let Ok(r) = m.wms.call(Message::new(0x31).u8(0x01, storage).u8(0x12, 1), T) else { continue };
            let Some(mut rd) = r.reader(0x01) else { continue };
            let n = rd.u32().unwrap_or(0);
            for _ in 0..n {
                let (Some(idx), Some(tag)) = (rd.u32(), rd.u8()) else { break };
                // Только входящие: отправленные с SIM не переносим
                if tag <= 1 {
                    self.take_stored_sms(m, storage, idx);
                }
            }
        }
    }

    /// Прочитать SMS из памяти модема, сохранить у себя и удалить из модема.
    fn take_stored_sms(&self, m: &Modem, storage: u8, idx: u32) {
        let mut id = vec![storage];
        id.extend_from_slice(&idx.to_le_bytes());
        let r = match m.wms.call(Message::new(0x22).tlv(0x01, id).u8(0x10, 1), T) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("SMS {storage}:{idx}: {e:#}");
                return;
            }
        };
        let Some(mut rd) = r.reader(0x01) else { return };
        let (Some(_tag), Some(format)) = (rd.u8(), rd.u8()) else { return };
        let Some(data) = rd.arr16() else { return };
        if format == 6 {
            if let Some(s) = pdu::decode(data) {
                self.receive(s);
            } else {
                tracing::warn!("SMS {storage}:{idx}: не разобран PDU");
            }
        }
        let _ = m.wms.call(Message::new(0x24).u8(0x01, storage).u32(0x10, idx).u8(0x12, 1), T);
    }

    fn receive(&self, s: pdu::Sms) {
        let t = s.timestamp.unwrap_or_else(now);
        let full = match s.concat {
            Some(c) if c.total > 1 => self.store.lock().unwrap().add_part(Part {
                number: s.number.clone(),
                reference: c.reference,
                total: c.total,
                seq: c.seq,
                text: s.text.clone(),
                time: t,
                received: now(),
            }),
            _ => Some((s.number.clone(), s.text.clone(), t)),
        };
        match full {
            Some((number, text, time)) => self.add_incoming(number, text, time),
            None => self.store.lock().unwrap().save(),
        }
    }

    fn add_incoming(&self, number: String, text: String, time: i64) {
        let msg = {
            let mut st = self.store.lock().unwrap();
            let id = st.id();
            let msg = Sms { id, number, text, time, incoming: true, read: false, status: SmsStatus::Received };
            st.sms.push(msg.clone());
            st.save();
            msg
        };
        tracing::info!("SMS от {}", msg.number);
        self.emit(Event::Sms { message: msg });
        self.refresh_unread();
    }

    fn send_sms(self: &Arc<Self>, number: String, text: String) -> Result<Sms> {
        let number: String = number.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '(' && *c != ')').collect();
        if number.is_empty() || text.is_empty() {
            bail!("пустой номер или текст");
        }
        let m = self.modem()?;
        let msg = {
            let mut st = self.store.lock().unwrap();
            let id = st.id();
            let msg = Sms { id, number: number.clone(), text: text.clone(), time: now(), incoming: false, read: true, status: SmsStatus::Sending };
            st.sms.push(msg.clone());
            st.save();
            msg
        };
        self.emit(Event::Sms { message: msg.clone() });
        let reference = self.sms_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = self.clone();
        let id = msg.id;
        std::thread::spawn(move || {
            let mut ok = true;
            for p in pdu::encode_submit(&number, &text, reference) {
                let mut raw = vec![6u8];
                raw.extend_from_slice(&(p.len() as u16).to_le_bytes());
                raw.extend_from_slice(&p);
                if let Err(e) = m.wms.call(Message::new(0x20).tlv(0x01, raw), Duration::from_secs(90)) {
                    tracing::warn!("отправка SMS: {e:#}");
                    ok = false;
                    break;
                }
            }
            d.set_sms_status(id, if ok { SmsStatus::Sent } else { SmsStatus::Failed });
        });
        Ok(msg)
    }

    fn set_sms_status(&self, id: u64, status: SmsStatus) {
        let msg = {
            let mut st = self.store.lock().unwrap();
            let Some(m) = st.sms.iter_mut().find(|m| m.id == id) else { return };
            m.status = status;
            let msg = m.clone();
            st.save();
            msg
        };
        self.emit(Event::Sms { message: msg });
    }

    // ── Звонки ──────────────────────────────────────────────────────────────────────────────────

    fn refresh_calls(&self, m: &Modem) {
        if let Ok(r) = m.voice.call(Message::new(0x2F), T) {
            self.apply_calls(&r, 0x10, 0x11);
        }
    }

    fn apply_calls(&self, r: &Message, info_tlv: u8, num_tlv: u8) {
        let mut numbers: HashMap<u8, String> = HashMap::new();
        if let Some(mut rd) = r.reader(num_tlv) {
            let n = rd.u8().unwrap_or(0);
            for _ in 0..n {
                let (Some(id), Some(_pi), Some(num)) = (rd.u8(), rd.u8(), rd.str8()) else { break };
                numbers.insert(id, num);
            }
        }
        let mut calls = Vec::new();
        if let Some(mut rd) = r.reader(info_tlv) {
            let n = rd.u8().unwrap_or(0);
            for _ in 0..n {
                let Some(b) = rd.bytes(7) else { break };
                let (id, state, kind, dir) = (b[0], b[1], b[2], b[3]);
                // Только голос (обычный, VoLTE, экстренный)
                if !matches!(kind, 0 | 2 | 9) {
                    continue;
                }
                let state = match state {
                    1 | 4 | 0x0A | 0x0B => CallState::Dialing,
                    5 => CallState::Alerting,
                    2 => CallState::Incoming,
                    7 => CallState::Waiting,
                    3 => CallState::Active,
                    6 => CallState::Held,
                    _ => CallState::Ended,
                };
                calls.push((id, state, dir == 2, numbers.get(&id).cloned().unwrap_or_default()));
            }
        }
        self.set_calls(calls);
    }

    fn set_calls(&self, list: Vec<(u8, CallState, bool, String)>) {
        let t = now();
        let mut ended: Vec<CallRecord> = Vec::new();
        let (calls, audio) = {
            let mut i = self.inner.lock().unwrap();
            let prev: HashMap<u8, Call> = i.status.calls.iter().map(|c| (c.id, c.clone())).collect();
            let seen: HashSet<u8> = list.iter().map(|c| c.0).collect();
            let mut calls = Vec::new();
            for (id, state, incoming, number) in list {
                i.started.entry(id).or_insert(t);
                if state == CallState::Active {
                    i.answered.entry(id).or_insert(t);
                }
                let number = if number.is_empty() { prev.get(&id).map(|c| c.number.clone()).unwrap_or_default() } else { number };
                let call = Call { id, number, state, incoming, answered_at: i.answered.get(&id).copied(), started_at: i.started[&id] };
                if state == CallState::Ended {
                    if prev.contains_key(&id) {
                        ended.push(Self::record_of(&mut i, &call, t));
                    }
                    continue;
                }
                calls.push(call);
            }
            // Пропали из списка без «END» — тоже завершены
            for (id, c) in &prev {
                if !seen.contains(id) {
                    ended.push(Self::record_of(&mut i, c, t));
                }
            }
            let audio = calls.iter().any(|c| matches!(c.state, CallState::Active | CallState::Alerting | CallState::Held | CallState::Dialing));
            (calls, audio)
        };
        if !ended.is_empty() {
            let mut st = self.store.lock().unwrap();
            for mut rec in ended {
                rec.id = st.id();
                st.calls.push(rec);
            }
            st.save();
            drop(st);
            self.emit(Event::CallLog);
        }
        self.update(|s| s.calls = calls);
        self.call_audio(audio);
    }

    fn record_of(i: &mut Inner, c: &Call, t: i64) -> CallRecord {
        let answered = i.answered.remove(&c.id);
        i.started.remove(&c.id);
        let rejected = i.rejected.remove(&c.id);
        let kind = match (c.incoming, answered.is_some()) {
            (false, _) => CallKind::Outgoing,
            (true, true) => CallKind::Incoming,
            (true, false) if rejected => CallKind::Rejected,
            (true, false) => CallKind::Missed,
        };
        CallRecord {
            id: 0,
            number: c.number.clone(),
            kind,
            time: c.started_at,
            duration: answered.map(|a| (t - a).max(0) as u32).unwrap_or(0),
            new: kind == CallKind::Missed,
        }
    }

    fn call_audio(&self, on: bool) {
        {
            let mut i = self.inner.lock().unwrap();
            if i.call_audio == on {
                return;
            }
            i.call_audio = on;
        }
        // Пока идёт звонок, системе спать нельзя (экран гасят кнопкой или датчик приближения у уха)
        const INHIBIT: &str = "/run/syn-sleep/inhibit.d/call";
        if on {
            let _ = std::fs::create_dir_all("/run/syn-sleep/inhibit.d");
            let _ = std::fs::write(INHIBIT, b"");
        } else {
            let _ = std::fs::remove_file(INHIBIT);
        }
        if std::path::Path::new(CALL_AUDIO_HOOK).exists() {
            let arg = if on { "start" } else { "stop" };
            match std::process::Command::new(CALL_AUDIO_HOOK).arg(arg).status() {
                Ok(s) if s.success() => {}
                r => tracing::warn!("{CALL_AUDIO_HOOK} {arg}: {r:?}"),
            }
        }
    }

    /// Логический слот карты-eUICC (у diting — eSIM, переназначенная в логический 1).
    fn euicc_slot(&self, m: &Modem) -> Result<u8> {
        let (slots, _) = manage::slots(&m.uim);
        slots
            .iter()
            .find(|s| s.euicc && s.card && s.active)
            .map(|s| s.logical)
            .ok_or_else(|| anyhow!("eSIM (eUICC) не найдена"))
    }

    /// Профиль eSIM сменился: карта перезапускается — заново поднять подписку и сеть.
    fn after_profile_switch(self: &Arc<Self>) {
        let d = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            if let Ok(m) = d.modem() {
                d.ensure_sim(&m);
                d.refresh_network(&m);
            }
        });
    }

    // ── Передача данных ─────────────────────────────────────────────────────────────────────────

    /// Привести сеанс данных к желаемому: включено и есть сеть — подключиться, иначе — отключиться.
    fn data_kick(self: &Arc<Self>) {
        use std::sync::atomic::Ordering::SeqCst;
        let (enabled, roaming_ok) = {
            let st = self.store.lock().unwrap();
            (st.data_on, st.data_roaming)
        };
        let can = {
            let i = self.inner.lock().unwrap();
            i.status.present
                && i.status.radio
                && i.status.registration.registered()
                && (roaming_ok || i.status.registration != Registration::Roaming)
        };
        if !enabled || !can {
            let s = self.data.lock().unwrap().take();
            if s.is_some() {
                tracing::info!("данные: отключение");
                data::disconnect(s.as_ref());
            }
            if !self.data_busy.load(SeqCst) {
                let state = if enabled { DataState::Waiting } else { DataState::Off };
                let roam_block = enabled && !roaming_ok && self.inner.lock().unwrap().status.registration == Registration::Roaming;
                self.update(|st| {
                    st.data.state = state;
                    st.data.address.clear();
                    if !enabled {
                        st.data.error.clear();
                    } else if roam_block {
                        st.data.error = t!("Роуминг: передача данных в роуминге выключена").into();
                    }
                });
            }
            return;
        }
        if self.data.lock().unwrap().is_some() || self.data_busy.swap(true, SeqCst) {
            return;
        }
        let d = self.clone();
        std::thread::spawn(move || {
            d.update(|st| st.data.state = DataState::Connecting);
            let r = d.data_connect();
            d.data_busy.store(false, SeqCst);
            match r {
                Ok(s) => {
                    let addr = s.settings.address.map(|a| a.to_string()).unwrap_or_default();
                    tracing::info!("данные: подключено, {addr}, DNS {:?}", s.settings.dns);
                    *d.data.lock().unwrap() = Some(s);
                    d.update(|st| {
                        st.data.state = DataState::Connected;
                        st.data.address = addr;
                        st.data.error.clear();
                    });
                    // Пока подключались, могли выключить
                    d.data_kick();
                }
                Err(e) => {
                    tracing::warn!("данные: {e:#}");
                    data::deconfigure();
                    d.update(|st| {
                        st.data.state = DataState::Error;
                        st.data.error = format!("{e:#}");
                    });
                    d.data_retry(Duration::from_secs(30));
                }
            }
        });
    }

    fn data_connect(&self) -> Result<data::Session> {
        use std::sync::atomic::Ordering::SeqCst;
        let tx = self.ind_tx.lock().unwrap().clone().ok_or_else(|| anyhow!("модем не запущен"))?;
        if !self.data_ready.load(SeqCst) {
            let pair = data::setup_kernel()?;
            data::setup_modem(&tx, pair)?;
            self.data_ready.store(true, SeqCst);
        }
        let s = data::connect(&tx)?;
        if let Err(e) = data::configure(&s.settings) {
            data::disconnect(Some(&s));
            return Err(e);
        }
        Ok(s)
    }

    /// Раз в 30 с — прирост байтов `rmnet_data0` в счётчики трафика.
    fn usage_loop(self: &Arc<Self>) {
        let d = self.clone();
        std::thread::spawn(move || {
            let read = |f: &str| -> Option<u64> {
                std::fs::read_to_string(format!("/sys/class/net/{}/statistics/{f}", data::IFACE)).ok()?.trim().parse().ok()
            };
            let mut last: Option<(u64, u64)> = None;
            let mut dirty = 0u32;
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let cur = read("rx_bytes").zip(read("tx_bytes"));
                if let (Some((rx, tx)), Some((lrx, ltx))) = (cur, last) {
                    // Счётчики интерфейса сбросились (пересоздан) — прирост с нуля
                    let drx = if rx >= lrx { rx - lrx } else { rx };
                    let dtx = if tx >= ltx { tx - ltx } else { tx };
                    if drx + dtx > 0 {
                        let mut st = d.store.lock().unwrap();
                        st.add_usage(drx, dtx, now());
                        dirty += 1;
                        // На диск — раз в 5 минут
                        if dirty >= 10 {
                            st.save();
                            dirty = 0;
                        }
                    }
                }
                last = cur;
            }
        });
    }

    fn data_retry(self: &Arc<Self>, after: Duration) {
        let d = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(after);
            if d.store.lock().unwrap().data_on && d.data.lock().unwrap().is_none() {
                d.data_kick();
            }
        });
    }

    /// Сеанс оборвал модем (сеть, оператор): снять настройки и переподключиться.
    fn data_lost(self: &Arc<Self>, msg: &Message) {
        let status = msg.get(0x01).and_then(|b| b.first().copied());
        if status != Some(1) {
            return;
        }
        let reason = msg.reader(0x11).and_then(|mut rd| Some((rd.u16()?, rd.i16()?)));
        tracing::info!("данные: сеанс закрыт модемом {reason:?}");
        // Клиент WDS закрывать не нужно — сеанс уже закрыт
        let s = self.data.lock().unwrap().take();
        drop(s);
        data::deconfigure();
        self.update(|st| {
            st.data.state = DataState::Waiting;
            st.data.address.clear();
        });
        self.data_retry(Duration::from_secs(5));
    }

    // ── Индикации ───────────────────────────────────────────────────────────────────────────────

    fn indication(self: &Arc<Self>, m: &Modem, service: u32, msg: Message) {
        match (service, msg.id) {
            (svc::NAS, 0x24) => self.apply_serving_system(m, &msg, true),
            (svc::NAS, 0x51) => self.apply_signal(&msg),
            (svc::NAS, 0x68) => {
                let cause = msg.get(0x03).and_then(|b| b.first().copied());
                tracing::info!("сеть отказала в регистрации: причина {cause:?}");
                self.update(|s| s.reject_cause = cause);
            }
            (svc::UIM, 0x32) => {
                if let Some(cs) = parse_card_status(&msg, 0x10) {
                    self.apply_card_status(&cs);
                }
            }
            (svc::WMS, 0x01) => {
                if let Some(mut rd) = msg.reader(0x10) {
                    if let (Some(storage), Some(idx)) = (rd.u8(), rd.u32()) {
                        self.take_stored_sms(m, storage, idx);
                    }
                }
                // Маршрут «передать»: PDU прямо в индикации, подтверждение — от нас
                if let Some(mut rd) = msg.reader(0x11) {
                    let (ack, txn, format) = (rd.u8(), rd.u32(), rd.u8());
                    if let (Some(data), Some(6)) = (rd.arr16(), format) {
                        if let Some(s) = pdu::decode(data) {
                            self.receive(s);
                        }
                    }
                    if let (Some(0), Some(txn)) = (ack, txn) {
                        let mut info = txn.to_le_bytes().to_vec();
                        info.extend_from_slice(&[1, 1]);
                        let _ = m.wms.call(Message::new(0x37).tlv(0x01, info), T);
                    }
                }
            }
            (svc::VOICE, 0x2E) => self.apply_calls(&msg, 0x01, 0x10),
            (svc::WDS, 0x22) => self.data_lost(&msg),
            (svc::VOICE, 0x43) => {
                let text = manage::ussd_text(&msg, 0x12, 0x14);
                let err = msg.get(0x10).is_some() || msg.get(0x11).is_some();
                let text = text.unwrap_or_else(|| if err { t!("Запрос не выполнен").into() } else { String::new() });
                self.emit(Event::Ussd { text, reply: !err, done: err });
            }
            (svc::VOICE, 0x3E) => {
                let reply = msg.get(0x01).and_then(|b| b.first()).is_some_and(|v| *v == 2);
                let text = manage::ussd_text(&msg, 0x10, 0x11).unwrap_or_default();
                self.emit(Event::Ussd { text, reply, done: !reply });
            }
            (svc::VOICE, 0x3D) => self.emit(Event::Ussd { text: String::new(), reply: false, done: true }),
            (svc::DMS, 0x01) => {
                if let Some(mode) = msg.get(0x14).and_then(|b| b.first()) {
                    self.update(|s| s.radio = *mode == 0);
                }
            }
            _ => {}
        }
    }

    /// Жизнь с одним запуском модема: подключиться, подготовить, обрабатывать индикации до ухода службы.
    fn modem_session(self: &Arc<Self>) -> Result<()> {
        let (tx, rx): (Sender<Indication>, Receiver<Indication>) = mpsc::channel();
        let m = Arc::new(self.connect(&tx)?);
        *self.ind_tx.lock().unwrap() = Some(tx);
        tracing::info!("модем: службы QMI подключены");
        *self.modem.lock().unwrap() = Some(m.clone());
        self.update(|s| s.present = true);
        self.setup(&m);
        self.data_kick();
        loop {
            match rx.recv_timeout(Duration::from_secs(600)) {
                Ok((_, msg)) if msg.id == SERVICE_GONE => break,
                Ok((service, msg)) => {
                    self.indication(&m, service, msg);
                    // Регистрация, радио — сеанс данных к желаемому состоянию
                    if service != svc::WDS {
                        self.data_kick();
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Склейка, которая не дождалась частей
                    let expired = self.store.lock().unwrap().expired_parts(now());
                    for (n, t, time) in expired {
                        self.add_incoming(n, t, time);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        tracing::warn!("модем: служба ушла (остановлен или перезапускается)");
        *self.modem.lock().unwrap() = None;
        *self.ind_tx.lock().unwrap() = None;
        drop(self.data.lock().unwrap().take());
        data::deconfigure();
        self.data_ready.store(false, std::sync::atomic::Ordering::SeqCst);
        self.set_calls(Vec::new());
        self.update(|s| {
            let keep = (s.radio, s.unread_sms, s.imei.clone(), s.data.enabled, s.gnss);
            *s = Status { radio: keep.0, unread_sms: keep.1, imei: keep.2, gnss: keep.4, ..Status::default() };
            s.data.enabled = keep.3;
            s.data.state = if keep.3 { DataState::Waiting } else { DataState::Off };
        });
        Ok(())
    }

    // ── Запросы ─────────────────────────────────────────────────────────────────────────────────

    fn handle(self: &Arc<Self>, req: Request, peer: Peer) -> Result<Response> {
        if !matches!(req, Request::Status) && !peer.trusted {
            bail!("нет доступа: SMS и звонки — для root и групп wheel/network");
        }
        Ok(match req {
            Request::Status => Response::Status { status: self.inner.lock().unwrap().status.clone() },
            Request::SetRadio { on } => {
                let m = self.modem()?;
                m.dms.call(Message::new(0x2E).u8(0x01, if on { 0 } else { 1 }), T)?;
                {
                    let mut st = self.store.lock().unwrap();
                    st.radio_off = !on;
                    st.save();
                }
                self.update(|s| s.radio = on);
                // Звонки сверить с модемом (в режиме полёта их нет)
                self.refresh_calls(&m);
                Response::Ok
            }
            Request::SetData { on } => {
                {
                    let mut st = self.store.lock().unwrap();
                    st.data_on = on;
                    st.save();
                }
                self.update(|s| s.data.enabled = on);
                self.data_kick();
                Response::Ok
            }
            Request::SmsList => Response::SmsList { messages: self.store.lock().unwrap().sms.clone() },
            Request::SmsSend { number, text } => Response::Sms { message: self.send_sms(number, text)? },
            Request::SmsRead { number } => {
                {
                    let mut st = self.store.lock().unwrap();
                    for m in st.sms.iter_mut().filter(|m| m.number == number) {
                        m.read = true;
                    }
                    st.save();
                }
                self.emit(Event::SmsChanged);
                self.refresh_unread();
                Response::Ok
            }
            Request::SmsDelete { ids } => {
                {
                    let mut st = self.store.lock().unwrap();
                    st.sms.retain(|m| !ids.contains(&m.id));
                    st.save();
                }
                self.emit(Event::SmsChanged);
                self.refresh_unread();
                Response::Ok
            }
            Request::Dial { number } => {
                let number: String = number.chars().filter(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#')).collect();
                if number.is_empty() {
                    bail!("пустой номер");
                }
                let m = self.modem()?;
                // Без радио или сети модем отвечает DeviceNotReady, а звонок застревал в «Вызов…»
                let (radio, registered) = {
                    let i = self.inner.lock().unwrap();
                    (i.status.radio, i.status.registration.registered())
                };
                let emergency = matches!(number.as_str(), "112" | "911" | "101" | "102" | "103" | "104" | "01" | "02" | "03");
                if !radio {
                    bail!("мобильная связь выключена — включите её (плитка «Мобильная связь» в шторке)");
                }
                if !registered && !emergency {
                    bail!("нет сети — звонок невозможен");
                }
                let clir = self.store.lock().unwrap().clir.clone();
                let number = manage::with_clir(&number, &clir);
                let r = m.voice.call(Message::new(0x20).tlv(0x01, number.into_bytes()), Duration::from_secs(30))?;
                let id = r.get(0x10).and_then(|b| b.first().copied()).unwrap_or(0);
                Response::CallId { id }
            }
            Request::Answer { id } => {
                self.modem()?.voice.call(Message::new(0x22).u8(0x01, id), T)?;
                Response::Ok
            }
            Request::Hangup { id } => {
                self.inner.lock().unwrap().rejected.insert(id);
                let m = self.modem()?;
                if let Err(e) = m.voice.call(Message::new(0x21).u8(0x01, id), T) {
                    // Модем такого звонка не знает (не готов, нет звонка) — убрать зависший из состояния
                    let gone = e.downcast_ref::<QmiError>().is_some_and(|q| matches!(q.0, 0x29 | 0x30 | 0x1A));
                    if !gone {
                        return Err(e);
                    }
                    tracing::warn!("отбой звонка {id}: {e:#} — убираю из состояния");
                    let rest: Vec<(u8, CallState, bool, String)> = self
                        .inner
                        .lock()
                        .unwrap()
                        .status
                        .calls
                        .iter()
                        .filter(|c| c.id != id)
                        .map(|c| (c.id, c.state, c.incoming, c.number.clone()))
                        .collect();
                    self.set_calls(rest);
                }
                Response::Ok
            }
            Request::Dtmf { id, digit } => {
                if !(digit.is_ascii_digit() || matches!(digit, '*' | '#')) {
                    bail!("недопустимый тон {digit}");
                }
                self.modem()?.voice.call(Message::new(0x28).tlv(0x01, vec![id, 1, digit as u8]), T)?;
                Response::Ok
            }
            Request::CallLog => Response::CallLog { calls: self.store.lock().unwrap().calls.clone() },
            Request::CallLogSeen => {
                {
                    let mut st = self.store.lock().unwrap();
                    st.calls.iter_mut().for_each(|c| c.new = false);
                    st.save();
                }
                self.emit(Event::CallLog);
                Response::Ok
            }
            Request::CallLogDelete { ids } => {
                {
                    let mut st = self.store.lock().unwrap();
                    st.calls.retain(|c| !ids.contains(&c.id));
                    st.save();
                }
                self.emit(Event::CallLog);
                Response::Ok
            }
            Request::Subscribe | Request::GnssWatch => bail!("поток обрабатывается соединением"),
            Request::Info => {
                let m = self.modem()?;
                Response::Info { info: manage::info(&m.dms, &m.uim) }
            }
            Request::Cell => {
                let m = self.modem()?;
                let (tech, plmn) = {
                    let i = self.inner.lock().unwrap();
                    (i.status.technology.clone(), i.status.plmn.clone())
                };
                Response::Cell { cell: manage::cell(&m.nas, &tech, &plmn) }
            }
            Request::EsimProfiles => {
                let m = self.modem()?;
                let ch = euicc::Channel::open(&m.uim, self.euicc_slot(&m)?)?;
                Response::EsimProfiles { profiles: euicc::profiles(&ch)? }
            }
            Request::EsimEnable { iccid } => {
                let m = self.modem()?;
                {
                    let ch = euicc::Channel::open(&m.uim, self.euicc_slot(&m)?)?;
                    euicc::enable(&ch, &iccid)?;
                }
                tracing::info!("eSIM: включён профиль {}…", &iccid[..iccid.len().min(6)]);
                self.after_profile_switch();
                Response::Ok
            }
            Request::EsimDisable { iccid } => {
                let m = self.modem()?;
                {
                    let ch = euicc::Channel::open(&m.uim, self.euicc_slot(&m)?)?;
                    euicc::disable(&ch, &iccid)?;
                }
                self.after_profile_switch();
                Response::Ok
            }
            Request::EsimDelete { iccid } => {
                let m = self.modem()?;
                let ch = euicc::Channel::open(&m.uim, self.euicc_slot(&m)?)?;
                euicc::delete(&ch, &iccid)?;
                Response::Ok
            }
            Request::EsimNickname { iccid, name } => {
                let m = self.modem()?;
                let ch = euicc::Channel::open(&m.uim, self.euicc_slot(&m)?)?;
                euicc::set_nickname(&ch, &iccid, &name)?;
                Response::Ok
            }
            Request::NetworkScan => Response::Networks { networks: manage::scan(&self.modem()?.nas)? },
            Request::NetworkSelect { plmn } => {
                manage::select(&self.modem()?.nas, plmn)?;
                Response::Ok
            }
            Request::Modes => {
                let mut modes = manage::modes(&self.modem()?.nas)?;
                modes.data_roaming = self.store.lock().unwrap().data_roaming;
                Response::Modes { modes }
            }
            Request::SetModes { allowed } => {
                manage::set_modes(&self.modem()?.nas, &allowed)?;
                Response::Ok
            }
            Request::SetDataRoaming { on } => {
                {
                    let mut st = self.store.lock().unwrap();
                    st.data_roaming = on;
                    st.save();
                }
                self.data_kick();
                Response::Ok
            }
            Request::ApnList => {
                let _m = self.modem()?;
                let wds = manage::temp_client(svc::WDS)?;
                Response::ApnList { apns: manage::apns(&wds)? }
            }
            Request::ApnSave { apn } => {
                let _m = self.modem()?;
                let wds = manage::temp_client(svc::WDS)?;
                manage::save_apn(&wds, &apn)?;
                Response::Ok
            }
            Request::ApnDelete { index } => {
                let _m = self.modem()?;
                let wds = manage::temp_client(svc::WDS)?;
                manage::delete_apn(&wds, index)?;
                Response::Ok
            }
            Request::PinEnable { on, pin } => {
                manage::pin_enable(&self.modem()?.uim, on, &pin)?;
                Response::Ok
            }
            Request::PinChange { old, new } => {
                manage::pin_change(&self.modem()?.uim, &old, &new)?;
                Response::Ok
            }
            Request::PinVerify { pin } => {
                manage::pin_verify(&self.modem()?.uim, &pin)?;
                Response::Ok
            }
            Request::PinUnblock { puk, new } => {
                manage::pin_unblock(&self.modem()?.uim, &puk, &new)?;
                Response::Ok
            }
            Request::Smsc => Response::Text { text: manage::smsc(&self.modem()?.wms)? },
            Request::SetSmsc { number } => {
                manage::set_smsc(&self.modem()?.wms, &number)?;
                Response::Ok
            }
            Request::Ussd { code } => {
                manage::ussd_start(&self.modem()?.voice, &code)?;
                Response::Ok
            }
            Request::UssdReply { text } => {
                manage::ussd_answer(&self.modem()?.voice, &text)?;
                Response::Ok
            }
            Request::UssdCancel => {
                manage::ussd_cancel(&self.modem()?.voice)?;
                Response::Ok
            }
            Request::CallServices => {
                let mut services = manage::call_services(&self.modem()?.voice);
                let clir = self.store.lock().unwrap().clir.clone();
                services.clir = if clir.is_empty() { "network".into() } else { clir };
                Response::CallServices { services }
            }
            Request::SetCallWaiting { on } => {
                manage::set_call_waiting(&self.modem()?.voice, on)?;
                Response::Ok
            }
            Request::SetClir { mode } => {
                if !matches!(mode.as_str(), "network" | "hide" | "show") {
                    bail!("неизвестный режим {mode}");
                }
                let mut st = self.store.lock().unwrap();
                st.clir = mode;
                st.save();
                Response::Ok
            }
            Request::SetForward { reason, number, timer } => {
                manage::set_forward(&self.modem()?.voice, &reason, &number, timer)?;
                Response::Ok
            }
            Request::Usage => Response::Usage { usage: self.store.lock().unwrap().usage.clone() },
            Request::UsageReset => {
                let mut st = self.store.lock().unwrap();
                st.usage.total_rx = 0;
                st.usage.total_tx = 0;
                st.usage.since = now();
                st.save();
                Response::Ok
            }
        })
    }
}

/// Имя сети по MCC/MNC (таблица модема и EONS SIM).
fn plmn_name(m: &Modem, mcc: u16, mnc: u16) -> Option<String> {
    let mut plmn = mcc.to_le_bytes().to_vec();
    plmn.extend_from_slice(&mnc.to_le_bytes());
    let r = m.nas.call(Message::new(0x44).tlv(0x01, plmn).u8(0x12, 1), T).ok()?;
    let mut rd = r.reader(0x10)?;
    let spn_enc = rd.u8()?;
    let spn = rd.arr8()?.to_vec();
    let (short_enc, _, short_spare) = (rd.u8()?, rd.u8()?, rd.u8()?);
    let short = rd.arr8()?.to_vec();
    let (long_enc, _, long_spare) = (rd.u8()?, rd.u8()?, rd.u8()?);
    let long = rd.arr8()?.to_vec();
    let dec = |enc: u8, b: &[u8], spare: u8| -> String {
        let s = match enc {
            4 => {
                // UCS-2: порядок байтов у модемов разный — латиница выдаёт его нулями
                let le = b.len() >= 2 && b[1] == 0;
                let units: Vec<u16> = b
                    .chunks_exact(2)
                    .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                    .collect();
                String::from_utf16_lossy(&units)
            }
            9 => pdu::gsm7_packed(b, spare),
            _ => String::from_utf8_lossy(b).into_owned(),
        };
        s.trim_matches(char::from(0)).trim().to_string()
    };
    [dec(long_enc, &long, long_spare), dec(short_enc, &short, short_spare), dec(spn_enc, &spn, 0)]
        .into_iter()
        .find(|s| !s.is_empty())
}

#[derive(Debug)]
struct App {
    kind: u8,
    state: u8,
    aid: Vec<u8>,
    pin1_state: u8,
    pin1_retries: u8,
}

#[derive(Debug)]
struct Card {
    state: u8,
    apps: Vec<App>,
}

#[derive(Debug)]
struct CardStatus {
    /// (карта, приложение) основной подписки 3GPP.
    primary_gw: Option<(usize, usize)>,
    cards: Vec<Card>,
}

impl CardStatus {
    fn primary_app(&self) -> Option<&App> {
        let (c, a) = self.primary_gw?;
        self.cards.get(c)?.apps.get(a)
    }

    fn sim_state(&self) -> SimState {
        if let Some(app) = self.primary_app() {
            return match app.state {
                7 => SimState::Ready,
                2 => SimState::PinRequired,
                3 => SimState::PukRequired,
                5 | 6 => SimState::Blocked,
                _ if app.pin1_state == 3 => SimState::PinRequired,
                _ => SimState::Initializing,
            };
        }
        if self.cards.iter().any(|c| c.state == 1) {
            SimState::Initializing
        } else if self.cards.iter().any(|c| c.state == 2) && !self.cards.iter().any(|c| c.state == 0) {
            SimState::Error
        } else {
            SimState::Absent
        }
    }
}

fn parse_card_status(r: &Message, tlv: u8) -> Option<CardStatus> {
    let mut rd = r.reader(tlv)?;
    let gw = rd.u16()?;
    rd.u16()?;
    rd.u16()?;
    rd.u16()?;
    let n = rd.u8()?;
    let mut cards = Vec::new();
    for _ in 0..n {
        let state = rd.u8()?;
        rd.bytes(4)?;
        let na = rd.u8()?;
        let mut apps = Vec::new();
        for _ in 0..na {
            let kind = rd.u8()?;
            let st = rd.u8()?;
            rd.bytes(4)?;
            let aid = rd.arr8()?.to_vec();
            rd.u8()?;
            let pin1_state = rd.u8()?;
            let pin1_retries = rd.u8()?;
            rd.bytes(4)?;
            apps.push(App { kind, state: st, aid, pin1_state, pin1_retries });
        }
        cards.push(Card { state, apps });
    }
    let primary_gw = (gw != 0xFFFF).then_some(((gw >> 8) as usize, (gw & 0xFF) as usize));
    Some(CardStatus { primary_gw, cards })
}

#[derive(Clone, Copy)]
struct Peer {
    trusted: bool,
}

fn peer_of(s: &UnixStream) -> Peer {
    let mut cred = libc::ucred { pid: 0, uid: u32::MAX, gid: u32::MAX };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    use std::os::fd::AsRawFd;
    let r = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    if r != 0 {
        return Peer { trusted: false };
    }
    Peer { trusted: cred.uid == 0 || in_group(cred.uid, cred.gid, &["wheel", "network"]) }
}

fn in_group(uid: u32, gid: u32, groups: &[&str]) -> bool {
    let user = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|s| s.lines().find(|l| l.split(':').nth(2) == Some(&uid.to_string())).map(|l| l.split(':').next().unwrap_or("").to_string()))
        .unwrap_or_default();
    std::fs::read_to_string("/etc/group").ok().is_some_and(|s| {
        s.lines().any(|l| {
            let f: Vec<&str> = l.split(':').collect();
            f.len() >= 4 && groups.contains(&f[0]) && (f[2] == gid.to_string() || f[3].split(',').any(|m| m == user))
        })
    })
}

fn serve(d: Arc<Daemon>, s: UnixStream) {
    let peer = peer_of(&s);
    let Ok(mut w) = s.try_clone() else { return };
    let send = |w: &mut UnixStream, v: &dyn erased::Json| -> bool {
        let mut out = v.json();
        out.push('\n');
        w.write_all(out.as_bytes()).is_ok()
    };
    for line in BufReader::new(s).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Request>(&line) {
            Ok(Request::Subscribe) => {
                if !peer.trusted {
                    send(&mut w, &Response::Error { message: t!("нет доступа к событиям").into() });
                    return;
                }
                let (tx, rx) = mpsc::channel();
                d.subs.lock().unwrap().push(tx);
                // Первое событие — текущее состояние
                let st = d.inner.lock().unwrap().status.clone();
                if !send(&mut w, &Event::Status { status: st }) {
                    return;
                }
                for ev in rx {
                    if !send(&mut w, &ev) {
                        return;
                    }
                }
                return;
            }
            Ok(Request::GnssWatch) => {
                if !peer.trusted {
                    send(&mut w, &Response::Error { message: t!("нет доступа к местоположению").into() });
                    return;
                }
                let Some(g) = d.gnss.get() else { return };
                let watch = g.watch();
                for fix in watch.rx.iter() {
                    if !send(&mut w, &fix) {
                        return;
                    }
                }
                return;
            }
            Ok(req) => {
                let resp = d.handle(req, peer).unwrap_or_else(|e| Response::Error { message: format!("{e:#}") });
                if !send(&mut w, &resp) {
                    break;
                }
            }
            Err(e) => {
                if !send(&mut w, &Response::Error { message: t!("запрос: {e}", e = e) }) {
                    break;
                }
            }
        }
    }
}

mod erased {
    pub trait Json {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}

pub fn run() -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        bail!("synmodemd запускается от root (synmodem.service)");
    }
    let dir = std::path::Path::new(api::SOCKET).parent().unwrap();
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(api::SOCKET);
    let l = UnixListener::bind(api::SOCKET).context(api::SOCKET)?;
    std::fs::set_permissions(api::SOCKET, std::fs::Permissions::from_mode(0o666))?;
    let d = Arc::new(Daemon {
        inner: Mutex::default(),
        modem: Mutex::default(),
        store: Mutex::new(Store::load()),
        subs: Mutex::default(),
        sms_ref: std::sync::atomic::AtomicU8::new((now() & 0xFF) as u8),
        data: Mutex::default(),
        data_busy: Default::default(),
        data_ready: Default::default(),
        ind_tx: Mutex::default(),
        gnss: Default::default(),
    });
    {
        let weak = Arc::downgrade(&d);
        let g = gnss::Engine::start(move |on| {
            if let Some(d) = weak.upgrade() {
                d.update(|s| s.gnss = on);
            }
        });
        let _ = d.gnss.set(g);
    }
    d.refresh_unread();
    d.usage_loop();
    {
        let on = d.store.lock().unwrap().data_on;
        d.update(|s| {
            s.data.enabled = on;
            s.data.state = if on { DataState::Waiting } else { DataState::Off };
        });
    }
    {
        let d = d.clone();
        std::thread::Builder::new().name("modem".into()).spawn(move || loop {
            if let Err(e) = d.modem_session() {
                tracing::debug!("модем недоступен: {e:#}");
                std::thread::sleep(Duration::from_secs(5));
            }
        })?;
    }
    tracing::info!("synmodemd слушает {}", api::SOCKET);
    for s in l.incoming() {
        match s {
            Ok(s) => {
                let d = d.clone();
                std::thread::spawn(move || serve(d, s));
            }
            Err(e) => tracing::warn!("accept: {e}"),
        }
    }
    Ok(())
}
