//! Синхронизация с соединёнными устройствами через synlink.
//!
//! Файлы устройства смонтированы демоном synlink (Проводник → Устройства);
//! хранилище там лежит по тому же пути в домашнем каталоге
//! ([`vault::VAULT_IN_HOME`]). Синхронизация — слияние по записям в обе
//! стороны: чужие правки вливаются сюда, итог записывается и туда (своим
//! ключом — после смены мастер-пароля копия там перешифровывается).
//! Не смонтировано — просим демон смонтировать.

use std::path::PathBuf;
use std::time::Duration;

use synshell_common::link::{self, PeerInfo, Request, Response};

use crate::vault::{self, Envelope, Session};

/// Итог синхронизации с одним устройством.
#[derive(Debug, Clone, PartialEq)]
pub enum PeerState {
    /// Записи совпадают.
    Synced,
    /// Там хранилища не было — записали своё.
    Copied,
    /// Там другой мастер-пароль.
    OtherPassword,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerSync {
    pub id: String,
    pub name: String,
    pub kind: link::DeviceKind,
    pub state: PeerState,
}

/// Соединённые устройства (быстро: демон не отвечает — пусто).
pub fn connected() -> Vec<PeerInfo> {
    link::status_quick(Duration::from_millis(800)).map(|s| s.connected().cloned().collect()).unwrap_or_default()
}

/// Где хранилище устройства в смонтированных файлах.
fn peer_vault(p: &PeerInfo) -> Option<PathBuf> {
    let mount = match &p.mount {
        Some(m) => m.clone(),
        None => match link::request(&Request::Mount { device: p.id.clone(), mount: true }) {
            Ok(Response::Mount { path: Some(m) }) => m,
            _ => return None,
        },
    };
    let home = p.home.clone().unwrap_or_else(|| "/".into());
    Some(PathBuf::from(format!("{}{}", mount.trim_end_matches('/'), home)).join(vault::VAULT_IN_HOME))
}

/// Конверт с устройства, если там есть хранилище.
pub fn peer_envelope(p: &PeerInfo) -> Option<Envelope> {
    let path = peer_vault(p)?;
    Envelope::read(&path).ok().flatten()
}

/// Слить со всеми соединёнными. `changed` — у нас появились изменения
/// (уже записаны в свой файл не здесь — вызывающий сохраняет).
pub fn sync_all(s: &mut Session) -> (Vec<PeerSync>, bool) {
    let mut out = Vec::new();
    let mut changed = false;
    let peers = connected();
    for p in &peers {
        let state = match sync_one(s, p) {
            Ok((st, ch)) => {
                changed |= ch;
                st
            }
            Err(e) => PeerState::Error(format!("{e:#}")),
        };
        out.push(PeerSync { id: p.id.clone(), name: p.name.clone(), kind: p.kind, state });
    }
    // Влитое с одного устройства — и остальным.
    if changed && peers.len() > 1 {
        for (p, ps) in peers.iter().zip(out.iter_mut()) {
            if matches!(ps.state, PeerState::Synced | PeerState::Copied) {
                if let Err(e) = sync_one(s, p) {
                    ps.state = PeerState::Error(format!("{e:#}"));
                }
            }
        }
    }
    (out, changed)
}

fn sync_one(s: &mut Session, p: &PeerInfo) -> anyhow::Result<(PeerState, bool)> {
    let path = peer_vault(p).ok_or_else(|| anyhow::anyhow!("файлы устройства недоступны"))?;
    let Some(env) = Envelope::read(&path)? else {
        s.seal()?.write(&path)?;
        return Ok((PeerState::Copied, false));
    };
    let Some(theirs) = s.open_other(&env)? else {
        return Ok((PeerState::OtherPassword, false));
    };
    let changed = s.merge(&theirs);
    // Там не то же самое или другой ключ (сменили пароль) — записать итог.
    if !vault::same(&s.data, &theirs) || !s.same_keys(&env) {
        s.seal()?.write(&path)?;
    }
    Ok((PeerState::Synced, changed))
}
