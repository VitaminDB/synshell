//! Журнал в SQLite. Одинаковые изменения (путь, программа, что сделано)
//! склеиваются: в памяти — между сбросами, в базе — если прошлая такая же
//! запись моложе [`MERGE_WINDOW`] (тогда растут `count` и `last`).

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use synfsd::{Kind, Record};

/// Склеивать с записью в базе, если она обновлялась не раньше (мс).
const MERGE_WINDOW: i64 = 60_000;

type Key = (String, Option<String>, Kind, String);

fn key(r: &Record) -> Key {
    (r.path.clone(), r.old_path.clone(), r.kind, r.exe.clone())
}

/// Ещё не записанное.
#[derive(Default)]
pub struct Pending {
    map: HashMap<Key, Record>,
}

impl Pending {
    pub fn add(&mut self, r: Record) {
        match self.map.get_mut(&key(&r)) {
            Some(e) => {
                e.count += 1;
                e.last = r.last;
                e.pid = r.pid;
            }
            None => {
                self.map.insert(key(&r), r);
            }
        }
    }

    pub fn take(&mut self) -> Vec<Record> {
        // Новая таблица, а не drain: после всплеска память не держится.
        let mut v: Vec<Record> = std::mem::take(&mut self.map).into_values().collect();
        v.sort_by_key(|r| r.first);
        v
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
}

pub struct Store {
    db: Connection,
    /// Недавние записи базы: ключ → (rowid, last).
    recent: HashMap<Key, (i64, i64)>,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
            // Журнал читают только через службу (она фильтрует по пользователю).
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700))?;
        }
        let db = Connection::open(path)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS events (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL,
                 old_path TEXT,
                 kind INTEGER NOT NULL,
                 is_dir INTEGER NOT NULL,
                 exe TEXT NOT NULL,
                 comm TEXT NOT NULL,
                 pid INTEGER NOT NULL,
                 uid INTEGER NOT NULL,
                 first INTEGER NOT NULL,
                 last INTEGER NOT NULL,
                 count INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS events_path ON events(path);
             CREATE INDEX IF NOT EXISTS events_old ON events(old_path);
             CREATE INDEX IF NOT EXISTS events_last ON events(last);",
        )?;
        Ok(Store { db, recent: HashMap::new() })
    }

    pub fn write(&mut self, batch: Vec<Record>) -> anyhow::Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let now = batch.iter().map(|r| r.last).max().unwrap_or(0);
        self.recent.retain(|_, (_, last)| now - *last < MERGE_WINDOW);
        // Всплеск (тысячи разных файлов) — склеивать не с чем, не держать в памяти.
        if self.recent.len() > 50_000 {
            self.recent.clear();
            self.recent.shrink_to_fit();
        }
        let tx = self.db.transaction()?;
        {
            let mut upd = tx.prepare_cached("UPDATE events SET count = count + ?1, last = ?2, pid = ?3 WHERE id = ?4")?;
            let mut ins = tx.prepare_cached(
                "INSERT INTO events (path, old_path, kind, is_dir, exe, comm, pid, uid, first, last, count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for r in batch {
                let k = key(&r);
                match self.recent.get_mut(&k) {
                    Some((id, last)) if r.first - *last < MERGE_WINDOW => {
                        upd.execute(params![r.count as i64, r.last, r.pid, *id])?;
                        *last = r.last;
                    }
                    _ => {
                        ins.execute(params![
                            r.path,
                            r.old_path,
                            r.kind.id(),
                            r.is_dir,
                            r.exe,
                            r.comm,
                            r.pid,
                            r.uid,
                            r.first,
                            r.last,
                            r.count as i64
                        ])?;
                        self.recent.insert(k, (tx.last_insert_rowid(), r.last));
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Удалить записи старше `before` (мс) и сверх `max` (самые старые).
    pub fn prune(&mut self, before: i64, max: u64) -> anyhow::Result<usize> {
        self.recent.clear();
        let mut n = self.db.execute("DELETE FROM events WHERE last < ?1", params![before])?;
        n += self.db.execute(
            "DELETE FROM events WHERE id <= (SELECT id FROM events ORDER BY id DESC LIMIT 1 OFFSET ?1)",
            params![max as i64],
        )?;
        if n > 0 {
            self.db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        Ok(n)
    }

    pub fn count(&self) -> u64 {
        self.db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0)).optional().ok().flatten().unwrap_or(0) as u64
    }

    /// История пути (и всего внутри, если `children`). `home` — если задан,
    /// только записи этого пользователя (`uid`) и пути внутри его папки.
    pub fn history(&self, path: &str, children: bool, limit: usize, only: Option<(u32, String)>) -> anyhow::Result<Vec<Record>> {
        let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
        // Всё внутри папки — диапазон строк [path/, path0): «0» идёт сразу за «/».
        let (lo, hi) = if path == "/" { ("/".to_string(), "0".to_string()) } else { (format!("{path}/"), format!("{path}0")) };
        let (uid, hlo, hhi) = match &only {
            Some((uid, home)) => (*uid as i64, format!("{home}/"), format!("{home}0")),
            None => (-1, String::new(), String::new()),
        };
        // ?8 — с содержимым папки; ?4 < 0 — без ограничения по пользователю.
        let mut st = self.db.prepare_cached(
            "SELECT path, old_path, kind, is_dir, exe, comm, pid, uid, first, last, count FROM events
             WHERE (path = ?1 OR old_path = ?1
                    OR (?8 AND ((path >= ?2 AND path < ?3) OR (old_path >= ?2 AND old_path < ?3))))
               AND (?4 < 0 OR uid = ?4 OR (path >= ?5 AND path < ?6))
             ORDER BY last DESC LIMIT ?7",
        )?;
        let rows = st.query_map(
            params![path, lo, hi, uid, hlo, hhi, limit.min(5000) as i64, children],
            |r| {
                Ok(Record {
                    path: r.get(0)?,
                    old_path: r.get(1)?,
                    kind: Kind::from_id(r.get(2)?),
                    is_dir: r.get(3)?,
                    exe: r.get(4)?,
                    comm: r.get(5)?,
                    pid: r.get(6)?,
                    uid: r.get(7)?,
                    first: r.get(8)?,
                    last: r.get(9)?,
                    count: r.get::<_, i64>(10)? as u64,
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &str, kind: Kind, exe: &str, t: i64) -> Record {
        Record {
            path: path.into(),
            old_path: None,
            kind,
            is_dir: false,
            exe: exe.into(),
            comm: String::new(),
            pid: 1,
            uid: 1000,
            first: t,
            last: t,
            count: 1,
        }
    }

    #[test]
    fn merges_and_queries() {
        let dir = std::env::temp_dir().join(format!("synfsd-test-{}", std::process::id()));
        let mut s = Store::open(&dir.join("a.db")).unwrap();
        let mut p = Pending::default();
        p.add(rec("/home/u/a.txt", Kind::Modified, "/usr/bin/vim", 1000));
        p.add(rec("/home/u/a.txt", Kind::Modified, "/usr/bin/vim", 2000));
        p.add(rec("/home/u/dir/b", Kind::Created, "/usr/bin/cp", 1500));
        p.add(rec("/home/ux", Kind::Created, "/usr/bin/cp", 1500));
        s.write(p.take()).unwrap();
        s.write(vec![rec("/home/u/a.txt", Kind::Modified, "/usr/bin/vim", 30_000)]).unwrap();
        let a = s.history("/home/u/a.txt", false, 10, None).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].count, a[0].first, a[0].last), (3, 1000, 30_000));
        // Всё внутри /home/u, но не соседняя /home/ux.
        let all = s.history("/home/u", true, 10, None).unwrap();
        assert_eq!(all.len(), 2);
        let other = s.history("/home/u", true, 10, Some((1, "/root".into()))).unwrap();
        assert!(other.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
