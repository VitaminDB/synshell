//! Сохранение рекорда, статистики и текущей партии в простые текстовые файлы.
//!
//! Путь: `$XDG_DATA_HOME/syn-blocks/{stats,game,game_deluxe}.txt` или
//! `~/.local/share/...`. Без каталога данных (например, на Android без HOME) —
//! работает в памяти.

use crate::game::{Board, Cell, Difficulty, Family, Game, Material, Piece, Rng, Shape, SIZE};
use std::path::PathBuf;

/// Вид игры: одна и та же логика, разная визуальная часть. У каждого вида
/// своя сохранённая партия и свои рекорды.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Variant {
    #[default]
    Standard,
    Deluxe,
}

impl Variant {
    pub const ALL: [Variant; 2] = [Variant::Standard, Variant::Deluxe];

    pub fn index(self) -> usize {
        match self {
            Variant::Standard => 0,
            Variant::Deluxe => 1,
        }
    }

    pub fn from_index(i: usize) -> Variant {
        match i {
            1 => Variant::Deluxe,
            _ => Variant::Standard,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Variant::Standard => "Стандарт",
            Variant::Deluxe => "Люкс",
        }
    }

    pub fn is_deluxe(self) -> bool {
        matches!(self, Variant::Deluxe)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub best: i32,
    pub games: u32,
    pub best_combo: u32,
    pub total_lines: u32,
    pub total_score: i64,
    pub perfects: u32,
    pub sound: bool,
    pub music: bool,
    /// Голосовые реплики («Люкс»).
    pub voice: bool,
    /// Выбранная сложность (индекс `Difficulty`).
    pub difficulty: u8,
    /// Рекорды: вид игры × сложность (легко / норма / сложно).
    pub best_by: [[i32; 3]; 2],
    /// Выбранный трек.
    pub track: u8,
    /// Треки по кругу (после конца трека — следующий).
    pub loop_all: bool,
    /// Последний выбранный вид игры (индекс `Variant`).
    pub variant: u8,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            best: 0,
            games: 0,
            best_combo: 0,
            total_lines: 0,
            total_score: 0,
            perfects: 0,
            sound: true,
            music: true,
            voice: true,
            difficulty: 1,
            best_by: [[0; 3]; 2],
            track: 0,
            loop_all: false,
            variant: 0,
        }
    }
}

impl Stats {
    /// Рекорд вида игры на сложности.
    pub fn best_of(&self, v: Variant, d: Difficulty) -> i32 {
        self.best_by[v.index()][d.index()]
    }

    /// Лучший результат вида игры по всем сложностям.
    pub fn best_variant(&self, v: Variant) -> i32 {
        self.best_by[v.index()].iter().copied().max().unwrap_or(0)
    }

    pub fn to_text(&self) -> String {
        let b = &self.best_by;
        format!(
            "best={}\ngames={}\nbest_combo={}\ntotal_lines={}\ntotal_score={}\nperfects={}\nsound={}\nmusic={}\nvoice={}\ndifficulty={}\nbest_easy={}\nbest_normal={}\nbest_hard={}\ndeluxe_best_easy={}\ndeluxe_best_normal={}\ndeluxe_best_hard={}\ntrack={}\nloop_all={}\nvariant={}\n",
            self.best, self.games, self.best_combo, self.total_lines, self.total_score, self.perfects,
            self.sound as u8, self.music as u8, self.voice as u8, self.difficulty, b[0][0], b[0][1], b[0][2], b[1][0], b[1][1], b[1][2], self.track,
            self.loop_all as u8, self.variant
        )
    }

    pub fn from_text(text: &str) -> Self {
        let mut s = Stats::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            let k = k.trim();
            // старые ключи рекордов «Люкса» (до переименования) читаются как новые
            let legacy = k.strip_prefix(LEGACY_DELUXE).and_then(|rest| rest.strip_prefix("_best_"));
            let k = match legacy {
                Some("easy") => "deluxe_best_easy",
                Some("normal") => "deluxe_best_normal",
                Some("hard") => "deluxe_best_hard",
                _ => k,
            };
            match k {
                "best" => s.best = v.parse().unwrap_or(0),
                "games" => s.games = v.parse().unwrap_or(0),
                "best_combo" => s.best_combo = v.parse().unwrap_or(0),
                "total_lines" => s.total_lines = v.parse().unwrap_or(0),
                "total_score" => s.total_score = v.parse().unwrap_or(0),
                "perfects" => s.perfects = v.parse().unwrap_or(0),
                "sound" => s.sound = v != "0",
                "music" => s.music = v != "0",
                "voice" => s.voice = v != "0",
                "difficulty" => s.difficulty = v.parse::<u8>().unwrap_or(1).min(2),
                "best_easy" => s.best_by[0][0] = v.parse().unwrap_or(0),
                "best_normal" => s.best_by[0][1] = v.parse().unwrap_or(0),
                "best_hard" => s.best_by[0][2] = v.parse().unwrap_or(0),
                "deluxe_best_easy" => s.best_by[1][0] = v.parse().unwrap_or(0),
                "deluxe_best_normal" => s.best_by[1][1] = v.parse().unwrap_or(0),
                "deluxe_best_hard" => s.best_by[1][2] = v.parse().unwrap_or(0),
                "track" => s.track = v.parse().unwrap_or(0),
                "loop_all" => s.loop_all = v != "0",
                "variant" => s.variant = v.parse::<u8>().unwrap_or(0).min(1),
                _ => {}
            }
        }
        s
    }
}

fn data_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SYN_BLOCKS_DATA") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let dir = base.join("syn-blocks");
    // Игра называлась Syn Blocks: сохранения из прежнего каталога переезжают в новый (один раз).
    let old = base.join("elle_block");
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
    Some(dir)
}

pub fn path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("stats.txt"))
}

/// Прежнее английское имя вида «Люкс» (до переименования в `deluxe`).
/// Нужно только для чтения старых сохранений: ключей рекордов в `stats.txt`
/// и файла партии. Собрано из частей, чтобы имя не встречалось в исходниках.
const LEGACY_DELUXE: &str = concat!("el", "vira");

/// Переносит сохранения, записанные до переименования вида: старый файл
/// партии «Люкса» переименовывается в `game_deluxe.txt`, если нового ещё нет.
/// Идемпотентна, ошибки молча пропускаются (сохранение — не критичный путь).
fn migrate_legacy_files(dir: &std::path::Path) {
    let old = dir.join(format!("game_{LEGACY_DELUXE}.txt"));
    let new = dir.join("game_deluxe.txt");
    if old.is_file() && !new.exists() {
        let _ = std::fs::rename(&old, &new);
    }
}

/// Файл текущей партии вида игры.
pub fn game_path(v: Variant) -> Option<PathBuf> {
    let name = match v {
        Variant::Standard => "game.txt",
        Variant::Deluxe => "game_deluxe.txt",
    };
    data_dir().map(|d| d.join(name))
}

// ── текущая партия ─────────────────────────────────────────────────────────

fn family_name(f: Family) -> String {
    format!("{f:?}")
}

fn family_by_name(name: &str) -> Option<Family> {
    Family::ALL.iter().copied().find(|f| family_name(*f) == name)
}

/// Фигура: `семейство:цвет:материал:r.c;r.c;…:отделка` — по имени семейства,
/// а не индексу, чтобы сохранение переживало изменения каталога фигур.
/// Отделка — необязательное последнее поле (старые записи без него).
fn piece_to_text(p: &Piece) -> String {
    let cells: Vec<String> = p.shape.cells().iter().map(|(r, c)| format!("{r}.{c}")).collect();
    format!("{}:{}:{}:{}:{}", family_name(p.family), p.color, p.material.index(), cells.join(";"), p.finish)
}

fn piece_from_text(s: &str) -> Option<Piece> {
    let mut it = s.split(':');
    let family = family_by_name(it.next()?)?;
    let color: u8 = it.next()?.parse().ok()?;
    let material = Material::from_index(it.next()?.parse().ok()?);
    let cells: Vec<(u8, u8)> = it
        .next()?
        .split(';')
        .map(|rc| {
            let (r, c) = rc.split_once('.')?;
            Some((r.parse().ok()?, c.parse().ok()?))
        })
        .collect::<Option<_>>()?;
    if cells.is_empty() || cells.len() > crate::game::piece::MAX_CELLS || cells.iter().any(|&(r, c)| r as usize >= SIZE || c as usize >= SIZE) {
        return None;
    }
    let finish = it.next().and_then(|s| s.parse::<u8>().ok()).unwrap_or(0);
    let mut p = Piece::new(family, Shape::from_cells(&cells)).with_material(material).with_finish(finish);
    p.color = color.clamp(1, 7);
    Some(p)
}

pub fn game_to_text(g: &Game) -> String {
    let mut board = String::with_capacity(SIZE * SIZE * 4);
    for r in 0..SIZE {
        for c in 0..SIZE {
            board.push_str(&format!("{:04x}", g.board.get(r, c)));
        }
    }
    let tray: Vec<String> = g.tray.iter().map(|p| p.map(|p| piece_to_text(&p)).unwrap_or_else(|| "-".into())).collect();
    let sc = &g.scoring;
    format!(
        "v=2\nseed={}\nrng={}\ndifficulty={}\nover={}\nenergy={}\nabilities_used={}\nmode={}\nmode_moves={}\n\
         score={}\ncombo={}\nmisses={}\nbest_combo={}\nlines={}\nmoves={}\nperfects={}\ngrace={}\n\
         board={}\ntray0={}\ntray1={}\ntray2={}\n",
        g.seed,
        g.rng.state(),
        g.difficulty.index(),
        g.over as u8,
        g.energy,
        g.abilities_used,
        g.mode.map(|m| m.index() as i32).unwrap_or(-1),
        g.mode_moves,
        sc.score,
        sc.combo,
        sc.misses,
        sc.best_combo,
        sc.lines,
        sc.moves,
        sc.perfects,
        sc.grace,
        board,
        tray[0],
        tray[1],
        tray[2],
    )
}

/// Разбор сохранённой партии; `None`, если файл битый или чужой версии.
/// Версия 1 (клетка — 2 hex-символа, без отделки) читается тоже.
pub fn game_from_text(text: &str) -> Option<Game> {
    let kv: std::collections::HashMap<&str, &str> = text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
    if !matches!(kv.get("v"), Some(&"1") | Some(&"2")) {
        return None;
    }
    let num = |k: &str| -> Option<i64> { kv.get(k)?.parse().ok() };
    let seed = kv.get("seed")?.parse::<u64>().ok()?;
    let mut g = Game::with_difficulty(seed, Difficulty::from_index(num("difficulty")? as usize));
    g.rng = Rng::new(kv.get("rng")?.parse::<u64>().ok()?);
    g.over = num("over")? != 0;
    g.energy = num("energy")? as u32;
    g.abilities_used = num("abilities_used")? as u32;
    g.mode = match num("mode")? {
        m if m < 0 => None,
        m => Some(Material::from_index(m as u8)).filter(|m| *m != Material::Plain),
    };
    g.mode_moves = num("mode_moves").unwrap_or(0) as u32;
    let sc = &mut g.scoring;
    sc.score = num("score")? as i32;
    sc.combo = num("combo")? as u32;
    sc.misses = num("misses")? as u32;
    sc.best_combo = num("best_combo")? as u32;
    sc.lines = num("lines")? as u32;
    sc.moves = num("moves")? as u32;
    sc.perfects = num("perfects")? as u32;
    sc.grace = num("grace")? as u32;
    let hex = kv.get("board")?;
    let w = match hex.len() {
        n if n == SIZE * SIZE * 2 => 2,
        n if n == SIZE * SIZE * 4 => 4,
        _ => return None,
    };
    let mut board = Board::new();
    for i in 0..SIZE * SIZE {
        let v = Cell::from_str_radix(hex.get(i * w..i * w + w)?, 16).ok()?;
        board.set(i / SIZE, i % SIZE, v);
    }
    g.board = board;
    for (i, key) in ["tray0", "tray1", "tray2"].iter().enumerate() {
        let v = kv.get(key)?;
        g.tray[i] = if *v == "-" { None } else { Some(piece_from_text(v)?) };
    }
    // трей не может быть пустым посреди партии
    if !g.over && g.tray.iter().all(|p| p.is_none()) {
        return None;
    }
    Some(g)
}

pub fn load_game(v: Variant) -> Option<Game> {
    if let Some(dir) = data_dir() {
        migrate_legacy_files(&dir);
    }
    let text = std::fs::read_to_string(game_path(v)?).ok()?;
    game_from_text(&text)
}

pub fn save_game(v: Variant, g: &Game) {
    let Some(p) = game_path(v) else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // через временный файл: оборванная запись не испортит сохранение
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, game_to_text(g)).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

pub fn load() -> Stats {
    if let Some(dir) = data_dir() {
        migrate_legacy_files(&dir);
    }
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| Stats::from_text(&t))
        .unwrap_or_default()
}

pub fn save(stats: &Stats) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, stats.to_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Stats { best: 1234, games: 7, best_combo: 5, total_lines: 88, total_score: 9999, perfects: 2, sound: false, music: true, voice: false, difficulty: 2, best_by: [[10, 1234, 77], [5, 6, 7]], track: 3, loop_all: true, variant: 1 };
        assert_eq!(Stats::from_text(&s.to_text()), s);
        assert_eq!(Stats::from_text("garbage\nbest=x\n"), Stats::default());
        // старый файл без ключей Люкса/голоса
        let old = Stats::from_text("best=5\nbest_normal=5\n");
        assert_eq!(old.best_by, [[0, 5, 0], [0; 3]]);
        assert!(old.voice);
        assert_eq!(old.best_of(Variant::Standard, Difficulty::Normal), 5);
        assert_eq!(old.best_variant(Variant::Deluxe), 0);
    }

    #[test]
    fn game_roundtrip() {
        let mut g = Game::with_difficulty(42, Difficulty::Hard);
        g.board.set(3, 4, crate::game::make_cell(5, Material::Crystal));
        g.board.set(7, 0, 2);
        g.tray[1] = None;
        g.tray[2] = Some(Piece::of(Family::D3).with_material(Material::Gold).with_finish(21));
        g.board.set(0, 0, crate::game::make_cell_full(3, Material::Plain, 24));
        g.scoring.score = 4321;
        g.scoring.combo = 3;
        g.energy = 5;
        g.mode = Some(Material::Wood);
        g.mode_moves = 7;
        let back = game_from_text(&game_to_text(&g)).expect("разбор");
        assert_eq!(back, g);
        // генератор продолжает ту же последовательность
        let (mut a, mut b) = (g.clone(), back);
        assert_eq!(a.rng.next_u64(), b.rng.next_u64());
        assert!(game_from_text("v=1\nseed=1\n").is_none());
        assert!(game_from_text("v=3\nseed=1\n").is_none());
        assert!(game_from_text("мусор").is_none());
    }

    #[test]
    fn legacy_v1_game_loads_without_finish() {
        let mut g = Game::with_difficulty(7, Difficulty::Normal);
        g.board.set(2, 2, crate::game::make_cell(4, Material::Wood));
        for p in g.tray.iter_mut().flatten() {
            p.finish = 0;
        }
        // текст версии 1: 2 символа на клетку, фигуры без поля отделки
        let mut text = game_to_text(&g).replace("v=2", "v=1");
        let board: String = (0..SIZE * SIZE).map(|i| format!("{:02x}", g.board.get(i / SIZE, i % SIZE))).collect();
        text = text.lines().map(|l| if l.starts_with("board=") { format!("board={board}") } else { l.trim_end_matches(":0").to_string() }).collect::<Vec<_>>().join("\n");
        let back = game_from_text(&text).expect("разбор v1");
        assert_eq!(back.board, g.board);
        assert_eq!(back.tray, g.tray);
        assert_eq!(crate::game::cell_finish(back.board.get(2, 2)), 0);
    }

    #[test]
    fn legacy_deluxe_best_keys_are_read() {
        let text = format!("{0}_best_easy=11\n{0}_best_normal=22\n{0}_best_hard=33\n", LEGACY_DELUXE);
        let s = Stats::from_text(&text);
        assert_eq!(s.best_by[1], [11, 22, 33]);
        // запись — уже под новыми ключами, и она читается обратно
        let back = Stats::from_text(&s.to_text());
        assert_eq!(back.best_by[1], [11, 22, 33]);
        assert!(s.to_text().contains("deluxe_best_hard=33"));
    }

    #[test]
    fn legacy_deluxe_game_file_is_renamed() {
        let dir = std::env::temp_dir().join(format!("syn_blocks_migrate_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join(format!("game_{LEGACY_DELUXE}.txt"));
        std::fs::write(&old, "v=2\n").unwrap();
        migrate_legacy_files(&dir);
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(dir.join("game_deluxe.txt")).unwrap(), "v=2\n");
        // новый файл уже есть — старый не трогаем и не затираем новый
        std::fs::write(&old, "old\n").unwrap();
        migrate_legacy_files(&dir);
        assert!(old.exists());
        assert_eq!(std::fs::read_to_string(dir.join("game_deluxe.txt")).unwrap(), "v=2\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_paths_differ_by_variant() {
        std::env::set_var("SYN_BLOCKS_DATA", std::env::temp_dir().join("syn_blocks_paths_test"));
        let a = game_path(Variant::Standard).unwrap();
        let b = game_path(Variant::Deluxe).unwrap();
        assert_ne!(a, b);
        assert!(b.ends_with("game_deluxe.txt"));
    }
}
