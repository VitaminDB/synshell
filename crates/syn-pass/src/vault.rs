//! Хранилище паролей: записи в JSON, зашифрованные мастер-паролем.
//!
//! Файл (`~/.local/share/synpass/vault.json`) — конверт [`Envelope`]: параметры
//! Argon2id и соль, nonce и шифротекст XChaCha20-Poly1305. Ключ — Argon2id от
//! мастер-пароля с солью конверта; расшифровка проверяет пароль (тег AEAD).
//!
//! Синхронизация между устройствами — слияние по записям ([`merge`]): у
//! каждой записи постоянный `id` и время изменения; удалённая остаётся
//! «надгробием» без данных, чтобы удаление тоже доходило до других.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};
use syngui::n_;

const AAD: &[u8] = b"synpass/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    pub favorite: bool,
    /// Миллисекунды UNIX.
    pub created: i64,
    pub modified: i64,
    /// Последнее копирование (для «Недавних»), только локально важно.
    pub used: i64,
    pub deleted: bool,
}

impl Entry {
    pub fn new() -> Self {
        let t = now_ms();
        Self { id: new_id(), created: t, modified: t, ..Default::default() }
    }

    /// Надгробие: удалено, данные стёрты.
    pub fn tombstone(&mut self) {
        let (id, created) = (std::mem::take(&mut self.id), self.created);
        self.zeroize_fields();
        *self = Entry { id, created, modified: now_ms(), deleted: true, ..Default::default() };
    }

    fn zeroize_fields(&mut self) {
        self.password.zeroize();
        self.notes.zeroize();
        self.username.zeroize();
    }

    /// Строка для поиска.
    pub fn matches(&self, q: &str) -> bool {
        let q = q.trim().to_lowercase();
        q.is_empty()
            || self.title.to_lowercase().contains(&q)
            || self.username.to_lowercase().contains(&q)
            || self.url.to_lowercase().contains(&q)
            || self.notes.to_lowercase().contains(&q)
    }

    /// Хост из адреса сайта (для подписи).
    pub fn host(&self) -> String {
        let u = self.url.trim();
        let u = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
        let h = u.split(['/', '?', '#']).next().unwrap_or("");
        h.trim_start_matches("www.").to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Data {
    pub entries: Vec<Entry>,
}

impl Data {
    pub fn live(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| !e.deleted)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Kdf {
    /// Соль, base64.
    pub salt: String,
    /// Память, КиБ.
    pub m: u32,
    pub t: u32,
    pub p: u32,
}

impl Kdf {
    /// Новые параметры: 64 МиБ, 3 прохода — на телефоне около полсекунды.
    pub fn fresh() -> Self {
        let mut salt = [0u8; 16];
        getrandom::getrandom(&mut salt).expect("getrandom");
        Self { salt: B64.encode(salt), m: 64 * 1024, t: 3, p: 1 }
    }

    pub fn derive(&self, password: &str) -> Result<Key> {
        let salt = B64.decode(&self.salt).context("соль")?;
        let params = argon2::Params::new(self.m, self.t, self.p, Some(32)).map_err(|e| anyhow::anyhow!("параметры Argon2: {e}"))?;
        let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = Zeroizing::new([0u8; 32]);
        a.hash_password_into(password.as_bytes(), &salt, key.as_mut()).map_err(|e| anyhow::anyhow!("Argon2: {e}"))?;
        Ok(Key(key))
    }
}

#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

impl Key {
    fn random() -> Self {
        let mut k = Zeroizing::new([0u8; 32]);
        getrandom::getrandom(k.as_mut()).expect("getrandom");
        Key(k)
    }

    fn encrypt(&self, plain: &[u8]) -> Result<(String, String)> {
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
        let c = XChaCha20Poly1305::new(self.0.as_ref().into());
        let ct = c.encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: AAD }).map_err(|_| anyhow::anyhow!("шифрование"))?;
        Ok((B64.encode(nonce), B64.encode(ct)))
    }

    /// `None` — ключ не подходит.
    fn decrypt(&self, nonce: &str, data: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let nonce = B64.decode(nonce).context("nonce")?;
        let ct = B64.decode(data).context("данные")?;
        if nonce.len() != 24 {
            bail!("повреждённое хранилище");
        }
        let c = XChaCha20Poly1305::new(self.0.as_ref().into());
        Ok(c.decrypt(XNonce::from_slice(&nonce), Payload { msg: &ct, aad: AAD }).ok().map(Zeroizing::new))
    }
}

/// Ответ на секретный вопрос без разницы в регистре, пробелах и «ё».
pub fn normalize_answer(a: &str) -> String {
    a.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().replace('ё', "е")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WrapKind {
    /// Под мастер-паролем.
    Password,
    /// Под ответом на секретный вопрос.
    Recovery,
}

/// Ключ хранилища, зашифрованный ключом из пароля или ответа.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Wrap {
    pub kind: WrapKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    pub kdf: Kdf,
    pub nonce: String,
    pub data: String,
}

impl Wrap {
    fn new(kind: WrapKind, question: Option<String>, secret: &str, dek: &Key) -> Result<(Self, Key)> {
        let kdf = Kdf::fresh();
        let kek = kdf.derive(secret)?;
        let (nonce, data) = kek.encrypt(dek.0.as_ref())?;
        Ok((Self { kind, question, kdf, nonce, data }, kek))
    }

    fn unwrap_with(&self, kek: &Key) -> Result<Option<Key>> {
        let Some(raw) = kek.decrypt(&self.nonce, &self.data)? else { return Ok(None) };
        if raw.len() != 32 {
            bail!("повреждённый ключ хранилища");
        }
        let mut k = Zeroizing::new([0u8; 32]);
        k.copy_from_slice(&raw);
        Ok(Some(Key(k)))
    }

    /// Ключ хранилища по паролю (ответу). `None` — не подходит.
    fn unwrap(&self, secret: &str) -> Result<Option<Key>> {
        self.unwrap_with(&self.kdf.derive(secret)?)
    }
}

/// Файл хранилища: записи зашифрованы случайным ключом хранилища, а сам он
/// лежит в обёртках — под мастер-паролем и (если задан секретный вопрос)
/// под ответом. Сменить мастер-пароль можно, зная ответ.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub version: u32,
    /// Какой ключ хранилища (у копий одного хранилища — один).
    pub key_id: String,
    /// Когда менялись обёртки (пароль, вопрос), мс.
    pub meta_modified: i64,
    pub wraps: Vec<Wrap>,
    pub nonce: String,
    pub data: String,
}

impl Envelope {
    fn check(&self) -> Result<()> {
        if self.format != "synpass" || self.version != 2 {
            bail!("неизвестный формат хранилища");
        }
        Ok(())
    }

    fn wrap(&self, kind: WrapKind) -> Option<&Wrap> {
        self.wraps.iter().find(|w| w.kind == kind)
    }

    /// Секретный вопрос, если задан (виден и без мастер-пароля).
    pub fn question(&self) -> Option<String> {
        self.wrap(WrapKind::Recovery).and_then(|w| w.question.clone())
    }

    fn open_data(&self, dek: &Key) -> Result<Option<Data>> {
        self.check()?;
        let Some(plain) = dek.decrypt(&self.nonce, &self.data)? else { return Ok(None) };
        Ok(Some(serde_json::from_slice(&plain).context("содержимое хранилища")?))
    }

    pub fn read(path: &Path) -> Result<Option<Self>> {
        match std::fs::read(path) {
            Ok(b) => Ok(Some(serde_json::from_slice(&b).with_context(|| format!("{}", path.display()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("{}", path.display())),
        }
    }

    /// Записать атомарно (временный файл + rename), права 0600.
    pub fn write(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = path.parent().context("каталог")?;
        std::fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
        let tmp = dir.join(format!(".vault.{}.tmp", std::process::id()));
        {
            let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp).with_context(|| format!("{}", tmp.display()))?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            let _ = f.sync_all();
        }
        std::fs::rename(&tmp, path).with_context(|| format!("{}", path.display()))?;
        Ok(())
    }
}

/// Открытое хранилище: ключ, обёртки и записи.
///
/// Копии хранилища на устройствах сходятся к одному ключу: из двух разных
/// побеждает меньший `key_id` (если к нему подходит наш мастер-пароль), из
/// обёрток одного ключа — более новые (пароль сменили на другом устройстве:
/// открытая здесь сессия подхватывает его, не спрашивая).
#[derive(Clone)]
pub struct Session {
    dek: Key,
    pub key_id: String,
    pub meta_modified: i64,
    wraps: Vec<Wrap>,
    pub data: Data,
    /// Мастер-пароль, если известен (сменили на другом устройстве — нет):
    /// им открываются копии с другим ключом.
    password: Option<Zeroizing<String>>,
    /// Ключи хранилищ, уже открытых паролем, по `key_id`.
    keys: HashMap<String, Key>,
    /// Обёртки, к которым пароль не подошёл (key_id, соль), — не выводить
    /// ключ заново при каждой синхронизации.
    foreign: std::collections::HashSet<(String, String)>,
}

impl Session {
    pub fn create(password: &str) -> Result<Self> {
        let dek = Key::random();
        let (w, _) = Wrap::new(WrapKind::Password, None, password, &dek)?;
        Ok(Self {
            dek,
            key_id: new_id(),
            meta_modified: now_ms(),
            wraps: vec![w],
            data: Data::default(),
            password: Some(Zeroizing::new(password.to_string())),
            keys: HashMap::new(),
            foreign: Default::default(),
        })
    }

    fn from_env(env: &Envelope, dek: Key, password: Option<&str>) -> Result<Option<Self>> {
        let Some(data) = env.open_data(&dek)? else { return Ok(None) };
        Ok(Some(Self {
            dek,
            key_id: env.key_id.clone(),
            meta_modified: env.meta_modified,
            wraps: env.wraps.clone(),
            data,
            password: password.map(|p| Zeroizing::new(p.to_string())),
            keys: HashMap::new(),
            foreign: Default::default(),
        }))
    }

    /// Открыть мастер-паролем. `Ok(None)` — пароль неверный.
    pub fn unlock(env: &Envelope, password: &str) -> Result<Option<Self>> {
        env.check()?;
        let w = env.wrap(WrapKind::Password).context("в хранилище нет мастер-пароля")?;
        let Some(dek) = w.unwrap(password)? else { return Ok(None) };
        Self::from_env(env, dek, Some(password))
    }

    /// Открыть ответом на секретный вопрос и сразу задать новый мастер-пароль.
    /// `Ok(None)` — ответ неверный.
    pub fn recover(env: &Envelope, answer: &str, new_password: &str) -> Result<Option<Self>> {
        env.check()?;
        let w = env.wrap(WrapKind::Recovery).context("секретный вопрос не задан")?;
        let Some(dek) = w.unwrap(&normalize_answer(answer))? else { return Ok(None) };
        let Some(mut s) = Self::from_env(env, dek, None)? else { return Ok(None) };
        s.change_password(new_password)?;
        Ok(Some(s))
    }

    pub fn seal(&self) -> Result<Envelope> {
        let plain = Zeroizing::new(serde_json::to_vec(&self.data)?);
        let (nonce, data) = self.dek.encrypt(&plain)?;
        Ok(Envelope {
            format: "synpass".into(),
            version: 2,
            key_id: self.key_id.clone(),
            meta_modified: self.meta_modified,
            wraps: self.wraps.clone(),
            nonce,
            data,
        })
    }

    fn wrap(&self, kind: WrapKind) -> Option<&Wrap> {
        self.wraps.iter().find(|w| w.kind == kind)
    }

    pub fn question(&self) -> Option<String> {
        self.wrap(WrapKind::Recovery).and_then(|w| w.question.clone())
    }

    /// Это мастер-пароль или ответ на секретный вопрос (для смены пароля
    /// в открытом хранилище). Ответ проверяется Argon2 — вызывать не из
    /// потока интерфейса.
    pub fn check_secret(&self, secret: &str) -> bool {
        if self.password.as_ref().is_some_and(|p| p.as_str() == secret) {
            return true;
        }
        let check = |kind: WrapKind, s: &str| self.wrap(kind).is_some_and(|w| matches!(w.unwrap(s), Ok(Some(_))));
        // Пароль, сменённый на другом устройстве, нам не известен — сверить с обёрткой.
        (self.password.is_none() && check(WrapKind::Password, secret)) || check(WrapKind::Recovery, &normalize_answer(secret))
    }

    fn set_wrap(&mut self, w: Wrap) {
        self.wraps.retain(|x| x.kind != w.kind);
        self.wraps.push(w);
        self.meta_modified = now_ms().max(self.meta_modified + 1);
    }

    pub fn change_password(&mut self, password: &str) -> Result<()> {
        let (w, _) = Wrap::new(WrapKind::Password, None, password, &self.dek)?;
        self.set_wrap(w);
        self.password = Some(Zeroizing::new(password.to_string()));
        // Новым паролем может открыться и то, что не открывалось.
        self.foreign.clear();
        Ok(())
    }

    pub fn set_recovery(&mut self, question: &str, answer: &str) -> Result<()> {
        let (w, _) = Wrap::new(WrapKind::Recovery, Some(question.trim().to_string()), &normalize_answer(answer), &self.dek)?;
        self.set_wrap(w);
        Ok(())
    }

    pub fn remove_recovery(&mut self) {
        if self.wrap(WrapKind::Recovery).is_some() {
            self.wraps.retain(|x| x.kind != WrapKind::Recovery);
            self.meta_modified = now_ms().max(self.meta_modified + 1);
        }
    }

    /// Взять ключ и обёртки чужого конверта (его ключ — `dek`).
    fn adopt(&mut self, env: &Envelope, dek: Key) {
        // Пароль остаётся известным, только если обёртка пароля та же.
        if self.wrap(WrapKind::Password) != env.wrap(WrapKind::Password) && self.key_id == env.key_id {
            self.password = None;
        }
        self.keys.insert(self.key_id.clone(), self.dek.clone());
        self.dek = dek;
        self.key_id = env.key_id.clone();
        self.meta_modified = env.meta_modified;
        self.wraps = env.wraps.clone();
    }

    /// Открыть копию хранилища (с другого устройства или изменённый файл):
    /// своим ключом или — копия с другим ключом — мастер-паролем. Заодно
    /// перенять у неё ключ или обёртки, если её «победа» (см. [`Session`]).
    /// `None` — там хранилище с другим мастер-паролем.
    pub fn open_other(&mut self, env: &Envelope) -> Result<Option<Data>> {
        env.check()?;
        if env.key_id == self.key_id {
            let d = env.open_data(&self.dek)?;
            if d.is_some() && env.meta_modified > self.meta_modified {
                let dek = self.dek.clone();
                self.adopt(env, dek);
            }
            return Ok(d);
        }
        let dek = match self.keys.get(&env.key_id) {
            Some(k) => Some(k.clone()),
            None => {
                let Some(w) = env.wrap(WrapKind::Password) else { return Ok(None) };
                let tag = (env.key_id.clone(), w.kdf.salt.clone());
                let Some(pw) = self.password.clone() else { return Ok(None) };
                if self.foreign.contains(&tag) {
                    return Ok(None);
                }
                match w.unwrap(&pw)? {
                    Some(k) => {
                        self.keys.insert(env.key_id.clone(), k.clone());
                        Some(k)
                    }
                    None => {
                        self.foreign.insert(tag);
                        None
                    }
                }
            }
        };
        let Some(dek) = dek else { return Ok(None) };
        let d = env.open_data(&dek)?;
        if d.is_some() && env.key_id < self.key_id {
            self.adopt(env, dek);
        }
        Ok(d)
    }

    /// Копия совпадает с нами по ключу и обёрткам.
    pub fn same_keys(&self, env: &Envelope) -> bool {
        env.key_id == self.key_id && env.meta_modified == self.meta_modified
    }

    /// Перенять у копии сессии (синхронизация идёт на копии) найденные ключи
    /// и отказы, а если она взяла другой ключ или новые обёртки — их.
    pub fn absorb_keys(&mut self, other: &Session) {
        for (k, v) in &other.keys {
            self.keys.entry(k.clone()).or_insert_with(|| v.clone());
        }
        self.foreign.extend(other.foreign.iter().cloned());
        let newer = if other.key_id == self.key_id { other.meta_modified > self.meta_modified } else { other.key_id < self.key_id };
        if newer {
            self.keys.insert(self.key_id.clone(), self.dek.clone());
            self.dek = other.dek.clone();
            self.key_id = other.key_id.clone();
            self.meta_modified = other.meta_modified;
            self.wraps = other.wraps.clone();
            self.password = other.password.clone();
        }
    }

    /// Влить чужие записи. `true` — у нас что-то изменилось.
    pub fn merge(&mut self, other: &Data) -> bool {
        merge(&mut self.data, other)
    }
}

/// Слить `other` в `into`: запись с более поздним изменением побеждает.
pub fn merge(into: &mut Data, other: &Data) -> bool {
    let mut changed = false;
    let pos: HashMap<String, usize> = into.entries.iter().enumerate().map(|(i, e)| (e.id.clone(), i)).collect();
    for e in &other.entries {
        match pos.get(&e.id) {
            Some(&i) => {
                let mine = &mut into.entries[i];
                if e.modified > mine.modified && e != mine {
                    let used = mine.used.max(e.used);
                    *mine = e.clone();
                    mine.used = used;
                    changed = true;
                }
            }
            None => {
                into.entries.push(e.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Записи совпадают (без учёта порядка и `used`).
pub fn same(a: &Data, b: &Data) -> bool {
    if a.entries.len() != b.entries.len() {
        return false;
    }
    let m: HashMap<&str, (i64, bool)> = b.entries.iter().map(|e| (e.id.as_str(), (e.modified, e.deleted))).collect();
    a.entries.iter().all(|e| m.get(e.id.as_str()) == Some(&(e.modified, e.deleted)))
}

pub fn vault_path() -> PathBuf {
    if let Ok(p) = std::env::var("SYNPASS_VAULT") {
        return PathBuf::from(p);
    }
    data_home().join("synpass/vault.json")
}

/// Путь хранилища относительно домашнего каталога (тот же на других устройствах).
pub const VAULT_IN_HOME: &str = ".local/share/synpass/vault.json";

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn new_id() -> String {
    let mut b = [0u8; 12];
    getrandom::getrandom(&mut b).expect("getrandom");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ─── генератор и оценка ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenOpts {
    pub length: usize,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Без похожих символов (0/O, 1/l/I) — удобно переписывать с экрана.
    pub no_similar: bool,
}

impl Default for GenOpts {
    fn default() -> Self {
        Self { length: 20, upper: true, digits: true, symbols: true, no_similar: true }
    }
}

pub fn generate(o: &GenOpts) -> String {
    const SIMILAR: &str = "0O1lIo";
    let filt = |s: &'static str| -> Vec<char> { s.chars().filter(|c| !o.no_similar || !SIMILAR.contains(*c)).collect() };
    let mut sets = vec![filt("abcdefghijklmnopqrstuvwxyz")];
    if o.upper {
        sets.push(filt("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    }
    if o.digits {
        sets.push(filt("0123456789"));
    }
    if o.symbols {
        sets.push("!@#$%^&*-_=+?.:;~".chars().collect());
    }
    let all: Vec<char> = sets.iter().flatten().copied().collect();
    let len = o.length.clamp(4, 128);
    loop {
        let mut out: Vec<char> = (0..len).map(|_| all[rand_below(all.len())]).collect();
        // По символу из каждого выбранного набора — на случайных местах.
        if len >= sets.len() {
            let mut places: Vec<usize> = (0..len).collect();
            for set in &sets {
                let i = rand_below(places.len());
                let p = places.swap_remove(i);
                out[p] = set[rand_below(set.len())];
            }
        }
        let s: String = out.into_iter().collect();
        if !s.is_empty() {
            return s;
        }
    }
}

fn rand_below(n: usize) -> usize {
    // Без смещения: отбрасываем хвост диапазона.
    let n = n as u32;
    let zone = u32::MAX - (u32::MAX % n);
    loop {
        let mut b = [0u8; 4];
        getrandom::getrandom(&mut b).expect("getrandom");
        let x = u32::from_le_bytes(b);
        if x < zone {
            return (x % n) as usize;
        }
    }
}

/// Сила пароля: 0 — очень слабый … 4 — отличный; и подпись.
pub fn strength(p: &str) -> (u8, &'static str) {
    if p.is_empty() {
        return (0, "");
    }
    let mut pool = 0u32;
    if p.chars().any(|c| c.is_lowercase()) {
        pool += 26;
    }
    if p.chars().any(|c| c.is_uppercase()) {
        pool += 26;
    }
    if p.chars().any(|c| c.is_ascii_digit()) {
        pool += 10;
    }
    if p.chars().any(|c| !c.is_alphanumeric()) {
        pool += 20;
    }
    if p.chars().any(|c| !c.is_ascii()) {
        pool += 33;
    }
    let distinct = {
        let mut v: Vec<char> = p.chars().collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    let len = p.chars().count().min(distinct * 2);
    let bits = len as f64 * (pool.max(1) as f64).log2();
    match bits as u32 {
        0..=27 => (0, n_!("Очень слабый")),
        28..=40 => (1, n_!("Слабый")),
        41..=59 => (2, n_!("Средний")),
        60..=89 => (3, n_!("Надёжный")),
        _ => (4, n_!("Отличный")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_password() {
        let mut s = Session::create("пароль").unwrap();
        let mut e = Entry::new();
        e.title = "Почта".into();
        e.password = "секрет".into();
        s.data.entries.push(e);
        let env = s.seal().unwrap();
        assert!(Session::unlock(&env, "не тот").unwrap().is_none());
        let s2 = Session::unlock(&env, "пароль").unwrap().unwrap();
        assert_eq!(s2.data.entries[0].password, "секрет");
    }

    #[test]
    fn merge_newer_wins_and_tombstones() {
        let mut a = Data::default();
        let mut e = Entry::new();
        e.title = "x".into();
        a.entries.push(e.clone());
        let mut b = a.clone();
        b.entries[0].title = "y".into();
        b.entries[0].modified += 10;
        assert!(merge(&mut a, &b));
        assert_eq!(a.entries[0].title, "y");
        let mut c = a.clone();
        c.entries[0].tombstone();
        c.entries[0].modified = a.entries[0].modified + 5;
        assert!(merge(&mut a, &c));
        assert!(a.entries[0].deleted);
        assert!(!merge(&mut a, &b));
    }

    #[test]
    fn copies_converge_and_password_change_spreads() {
        let mut phone = Session::create("общий").unwrap();
        let mut desk = Session::create("общий").unwrap();
        let (pk, dk) = (phone.key_id.clone(), desk.key_id.clone());
        // Разные ключи: обе стороны открывают паролем, побеждает меньший key_id.
        assert!(phone.open_other(&desk.seal().unwrap()).unwrap().is_some());
        assert!(desk.open_other(&phone.seal().unwrap()).unwrap().is_some());
        let win = pk.min(dk);
        assert_eq!(phone.key_id, win);
        assert_eq!(desk.key_id, win);
        // Сменили пароль на компьютере — телефон подхватывает без пароля.
        desk.change_password("новый").unwrap();
        assert!(phone.open_other(&desk.seal().unwrap()).unwrap().is_some());
        assert!(phone.same_keys(&desk.seal().unwrap()));
        assert!(Session::unlock(&phone.seal().unwrap(), "новый").unwrap().is_some());
        assert!(Session::unlock(&phone.seal().unwrap(), "общий").unwrap().is_none());
        // Чужое хранилище с другим паролем не открывается.
        let other = Session::create("другой").unwrap();
        assert!(phone.open_other(&other.seal().unwrap()).unwrap().is_none());
    }

    #[test]
    fn recovery_question() {
        let mut s = Session::create("забуду").unwrap();
        let mut e = Entry::new();
        e.title = "Банк".into();
        s.data.entries.push(e);
        s.set_recovery("Кличка первого питомца", "  Бармалей Ёжиков ").unwrap();
        let env = s.seal().unwrap();
        assert_eq!(env.question().as_deref(), Some("Кличка первого питомца"));
        assert!(Session::recover(&env, "мурзик", "новый-пароль").unwrap().is_none());
        let r = Session::recover(&env, "бармалей   ежиков", "новый-пароль").unwrap().unwrap();
        assert_eq!(r.data.entries[0].title, "Банк");
        let env2 = r.seal().unwrap();
        assert!(Session::unlock(&env2, "новый-пароль").unwrap().is_some());
        assert!(Session::unlock(&env2, "забуду").unwrap().is_none());
        // Вопрос остаётся и после восстановления.
        assert!(env2.question().is_some());
        assert!(r.check_secret("Бармалей Ежиков"));
        assert!(r.check_secret("новый-пароль"));
        assert!(!r.check_secret("забуду"));
    }

    #[test]
    fn generator() {
        let o = GenOpts { length: 16, ..Default::default() };
        let p = generate(&o);
        assert_eq!(p.chars().count(), 16);
        assert!(p.chars().any(|c| c.is_ascii_digit()));
        assert!(strength(&p).0 >= 3);
    }
}
