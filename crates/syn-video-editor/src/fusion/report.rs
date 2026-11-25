//! Отчёт покрытия библиотеки шаблонов Fusion (`.setting`):
//! параллельный разбор всех шаблонов (`rayon`), статистика по разделам,
//! ошибки парсинга и рейтинг недостающих инструментов (с подсчётом
//! `sole_blocker` — сколько шаблонов станут на 100% поддерживаемыми,
//! если добавить именно этот инструмент).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use rayon::prelude::*;
use serde_json::{json, Value};

use super::{is_supported_tool, parse, scan, Comp};

#[derive(Clone, Debug, PartialEq)]
pub struct SectionStat {
    pub name: String,
    pub total: usize,
    pub parsed: usize,
    pub full: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MissingTool {
    pub ty: String,
    /// В скольких шаблонах встречается.
    pub templates: usize,
    /// Для скольких шаблонов это единственный недостающий инструмент.
    pub sole_blocker: usize,
    /// Пример шаблона, где используется.
    pub sample: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoverageReport {
    pub root: String,
    pub query: String,
    pub total: usize,
    pub parsed: usize,
    pub full: usize,
    pub partial: usize,
    pub total_tool_types: usize,
    pub supported_tool_types: usize,
    pub by_section: Vec<SectionStat>,
    pub missing_tools: Vec<MissingTool>,
    pub parse_errors: Vec<(String, String)>,
}

/// Проанализировать все `.setting` в `root` (опционально с фильтром по разделу/имени).
pub fn analyze(root: &Path, query: Option<&str>) -> CoverageReport {
    let q = query.unwrap_or("").trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let files: Vec<_> = scan(root)
        .into_iter()
        .filter(|(cat, name, _)| {
            if words.is_empty() {
                return true;
            }
            let hay = format!("{cat} {name}").to_lowercase().replace('_', " ");
            words.iter().all(|w| hay.contains(w))
        })
        .collect();

    struct One {
        section: String,
        label: String,
        parsed: Result<(Vec<String>, Vec<String>), String>,
    }

    let results: Vec<One> = files
        .par_iter()
        .map(|(cat, name, path)| {
            let section = cat.split('/').next().filter(|s| !s.is_empty()).unwrap_or("Корень").to_string();
            let label = if cat.is_empty() { name.clone() } else { format!("{cat}/{name}") };
            let parsed = std::fs::read_to_string(path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse(&text))
                .map(|lua| {
                    let comp = Comp::from_lua(name.as_str(), &lua);
                    let types = comp.tool_types();
                    let unsup = comp.unsupported_tools();
                    (types, unsup)
                });
            One { section, label, parsed }
        })
        .collect();

    let mut sections: BTreeMap<String, SectionStat> = BTreeMap::new();
    let mut missing: HashMap<String, (usize, usize, String)> = HashMap::new();
    let mut all_types: HashSet<String> = HashSet::new();
    let mut parse_errors: Vec<(String, String)> = Vec::new();
    let mut parsed_n = 0usize;
    let mut full_n = 0usize;

    for r in results {
        let sec = sections.entry(r.section.clone()).or_insert_with(|| SectionStat {
            name: r.section,
            total: 0,
            parsed: 0,
            full: 0,
        });
        sec.total += 1;
        match r.parsed {
            Ok((types, unsup)) => {
                parsed_n += 1;
                sec.parsed += 1;
                for t in types {
                    all_types.insert(t);
                }
                if unsup.is_empty() {
                    full_n += 1;
                    sec.full += 1;
                } else {
                    let sole = unsup.len() == 1;
                    for u in unsup {
                        let e = missing.entry(u).or_insert_with(|| (0, 0, r.label.clone()));
                        e.0 += 1;
                        if sole {
                            e.1 += 1;
                        }
                    }
                }
            }
            Err(err) => parse_errors.push((r.label, err)),
        }
    }

    let mut by_section: Vec<SectionStat> = sections.into_values().collect();
    by_section.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.name.cmp(&b.name)));

    let mut missing_tools: Vec<MissingTool> = missing
        .into_iter()
        .map(|(ty, (templates, sole_blocker, sample))| MissingTool { ty, templates, sole_blocker, sample })
        .collect();
    missing_tools.sort_by(|a, b| b.templates.cmp(&a.templates).then_with(|| b.sole_blocker.cmp(&a.sole_blocker)).then_with(|| a.ty.cmp(&b.ty)));

    let supported_tool_types = all_types.iter().filter(|t| is_supported_tool(t)).count();

    CoverageReport {
        root: root.display().to_string(),
        query: q,
        total: files.len(),
        parsed: parsed_n,
        full: full_n,
        partial: parsed_n.saturating_sub(full_n),
        total_tool_types: all_types.len(),
        supported_tool_types,
        by_section,
        missing_tools,
        parse_errors,
    }
}

impl CoverageReport {
    pub fn full_pct(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.full as f64 * 100.0 / self.total as f64 }
    }

    pub fn parsed_pct(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.parsed as f64 * 100.0 / self.total as f64 }
    }

    pub fn format(&self, top: usize) -> String {
        let mut lines = Vec::new();
        let scope = if self.query.is_empty() { String::new() } else { format!(" (фильтр: «{}»)", self.query) };
        lines.push(format!(
            "Покрытие библиотеки Fusion{scope}: {} шаблонов — разобрано {} ({:.1}%), полностью поддерживаются {} ({:.1}%), частично {} (ошибок разбора: {})",
            self.total,
            self.parsed,
            self.parsed_pct(),
            self.full,
            self.full_pct(),
            self.partial,
            self.parse_errors.len()
        ));
        lines.push(format!("Типов инструментов в библиотеке: {} (поддерживаются: {})", self.total_tool_types, self.supported_tool_types));
        if !self.by_section.is_empty() {
            lines.push("По разделам:".into());
            for s in &self.by_section {
                let pct = if s.total == 0 { 0.0 } else { s.full as f64 * 100.0 / s.total as f64 };
                lines.push(format!("  {} — {}/{} ({:.1}%), разобрано {}", s.name, s.full, s.total, pct, s.parsed));
            }
        }
        if !self.missing_tools.is_empty() && top > 0 {
            lines.push(format!("Топ недостающих инструментов (из {}):", self.missing_tools.len()));
            for m in self.missing_tools.iter().take(top) {
                let sole = if m.sole_blocker > 0 { format!(", +{} до 100%", m.sole_blocker) } else { String::new() };
                lines.push(format!("  {} — {} шаблонов{} (пример: {})", m.ty, m.templates, sole, m.sample));
            }
        }
        if !self.parse_errors.is_empty() {
            lines.push(format!("Ошибки разбора (первые {}):", self.parse_errors.len().min(10)));
            for (p, e) in self.parse_errors.iter().take(10) {
                lines.push(format!("  {p}: {e}"));
            }
        }
        lines.join("\n")
    }

    pub fn to_json(&self, top: usize) -> Value {
        json!({
            "root": self.root,
            "query": self.query,
            "total": self.total,
            "parsed": self.parsed,
            "full": self.full,
            "partial": self.partial,
            "full_pct": (self.full_pct() * 10.0).round() / 10.0,
            "parsed_pct": (self.parsed_pct() * 10.0).round() / 10.0,
            "total_tool_types": self.total_tool_types,
            "supported_tool_types": self.supported_tool_types,
            "by_section": self.by_section.iter().map(|s| json!({
                "name": s.name,
                "total": s.total,
                "parsed": s.parsed,
                "full": s.full,
            })).collect::<Vec<_>>(),
            "missing_tools": self.missing_tools.iter().take(top).map(|m| json!({
                "type": m.ty,
                "templates": m.templates,
                "sole_blocker": m.sole_blocker,
                "sample": m.sample,
            })).collect::<Vec<_>>(),
            "parse_errors": self.parse_errors.iter().take(20).map(|(p, e)| json!({"path": p, "error": e})).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzes_synthetic_templates() {
        let dir = tempfile::tempdir().unwrap();
        let titles = dir.path().join("Титры");
        let fx = dir.path().join("Эффекты");
        std::fs::create_dir_all(&titles).unwrap();
        std::fs::create_dir_all(&fx).unwrap();
        std::fs::write(
            titles.join("ok.setting"),
            r#"{ Tools = ordered() { B = Background {}, T = TextPlus {}, M = Merge { Inputs = { Background = Input { SourceOp = "B" }, Foreground = Input { SourceOp = "T" } } } } }"#,
        )
        .unwrap();
        std::fs::write(
            fx.join("partial.setting"),
            r#"{ Tools = ordered() { B = Background {}, R = Renderer3D { Inputs = { Input = Input { SourceOp = "B" } } } } }"#,
        )
        .unwrap();
        let rep = analyze(dir.path(), None);
        assert_eq!(rep.total, 2);
        assert_eq!(rep.parsed, 2);
        assert_eq!(rep.full, 1);
        assert_eq!(rep.partial, 1);
        assert_eq!(rep.missing_tools.len(), 1);
        assert_eq!(rep.missing_tools[0].ty, "Renderer3D");
        assert_eq!(rep.missing_tools[0].sole_blocker, 1);
        assert!(rep.format(10).contains("Renderer3D"));
    }
}
