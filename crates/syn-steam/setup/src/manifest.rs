//! Манифест обновлений клиента Steam (`steam_client_linuxarm64` на CDN Valve, формат KeyValues/VDF):
//! версия и компоненты — архивы zip с именем файла на CDN, размером и sha256.

use anyhow::{bail, Context, Result};

/// Каналы клиента: манифест на CDN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(s: &str) -> Option<Channel> {
        match s {
            "stable" => Some(Channel::Stable),
            "publicbeta" | "beta" => Some(Channel::Beta),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "publicbeta",
        }
    }

    /// Имя манифеста на CDN.
    pub fn manifest(self) -> &'static str {
        match self {
            Channel::Stable => "steam_client_linuxarm64",
            Channel::Beta => "steam_client_publicbeta_linuxarm64",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// «bins_linuxarm64_linuxarm64»
    pub name: String,
    /// Имя файла на CDN: «bins_linuxarm64_linuxarm64.zip.<sha1>»
    pub file: String,
    pub size: u64,
    /// sha256 файла (поле «sha2»), строчными.
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// Платформа (ключ верхнего уровня): «linuxarm64».
    pub platform: String,
    pub version: String,
    pub components: Vec<Component>,
}

impl Manifest {
    /// Компоненты нативного клиента: общие (`*_all`) и для этой платформы (`*_<платформа>_<платформа>`). Прочие
    /// (`bins_linuxarm64`, `*_steamrt_*`, `bins_androidarm64_linuxarm64`…) клиент при надобности берёт сам — их
    /// не ставил и прежний пакет steam-arm64.
    pub fn selected(&self) -> Vec<&Component> {
        let own = format!("_{p}_{p}", p = self.platform);
        self.components
            .iter()
            .filter(|c| c.name.ends_with("_all") || c.name.ends_with(&own))
            .collect()
    }

    /// Сколько скачать (байт) — выбранные компоненты.
    pub fn download_size(&self) -> u64 {
        self.selected().iter().map(|c| c.size).sum()
    }
}

/// Узел KeyValues: строка или вложенный блок.
#[derive(Debug, PartialEq)]
enum Node {
    Str(String),
    Obj(Vec<(String, Node)>),
}

fn tokens(text: &str) -> Result<Vec<Tok>> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut s = String::new();
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(x) => s.push(x),
                            None => bail!("строка не закрыта"),
                        },
                        Some(x) => s.push(x),
                        None => bail!("строка не закрыта"),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for x in it.by_ref() {
                    if x == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                // ключ без кавычек
                let mut s = String::from(c);
                while let Some(&x) = it.peek() {
                    if x.is_whitespace() || x == '{' || x == '}' || x == '"' {
                        break;
                    }
                    s.push(x);
                    it.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn block(t: &[Tok], i: &mut usize, top: bool) -> Result<Vec<(String, Node)>> {
    let mut v = Vec::new();
    loop {
        match t.get(*i) {
            None if top => return Ok(v),
            None => bail!("блок не закрыт"),
            Some(Tok::Close) if !top => {
                *i += 1;
                return Ok(v);
            }
            Some(Tok::Close) => bail!("лишняя «}}»"),
            Some(Tok::Open) => bail!("«{{» без ключа"),
            Some(Tok::Str(k)) => {
                *i += 1;
                match t.get(*i) {
                    Some(Tok::Str(s)) => {
                        *i += 1;
                        v.push((k.clone(), Node::Str(s.clone())));
                    }
                    Some(Tok::Open) => {
                        *i += 1;
                        let b = block(t, i, false)?;
                        v.push((k.clone(), Node::Obj(b)));
                    }
                    _ => bail!("у ключа «{k}» нет значения"),
                }
            }
        }
    }
}

fn get<'a>(o: &'a [(String, Node)], k: &str) -> Option<&'a str> {
    o.iter().find_map(|(n, v)| match v {
        Node::Str(s) if n == k => Some(s.as_str()),
        _ => None,
    })
}

/// Разбор манифеста. Принимаются только компоненты с файлом, размером и sha256; имя файла — без путей.
pub fn parse(text: &str) -> Result<Manifest> {
    let t = tokens(text)?;
    let mut i = 0;
    let top = block(&t, &mut i, true)?;
    let (platform, body) = top
        .iter()
        .find_map(|(k, v)| match v {
            Node::Obj(o) if get(o, "version").is_some() => Some((k.clone(), o)),
            _ => None,
        })
        .context("в манифесте нет блока с версией")?;
    let version = get(body, "version").unwrap_or_default().to_string();
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        bail!("версия манифеста «{version}» — не число");
    }
    let mut components = Vec::new();
    for (name, v) in body {
        let Node::Obj(o) = v else { continue };
        let (Some(file), Some(size), Some(sha)) = (get(o, "file"), get(o, "size"), get(o, "sha2"))
        else {
            continue;
        };
        let size: u64 = size
            .parse()
            .with_context(|| format!("{name}: размер «{size}»"))?;
        let sha = sha.to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("{name}: sha256 «{sha}»");
        }
        if file.is_empty() || file.contains(['/', '\\']) || file.starts_with('.') {
            bail!("{name}: имя файла «{file}»");
        }
        components.push(Component {
            name: name.clone(),
            file: file.to_string(),
            size,
            sha256: sha,
        });
    }
    if components.is_empty() {
        bail!("в манифесте нет компонентов");
    }
    Ok(Manifest {
        platform,
        version,
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#""linuxarm64"
{
	"version"		"1792618285"
	"tenfoot_images_all"
	{
		"file"		"tenfoot_images_all.zip.86419c7a"
		"size"		"6582204"
		"sha2"		"990676BC29D01736477C6B571B5048258A6156284EF242503112FC68567CBF01"
		"zipvz"		"tenfoot_images_all.zip.vz.193cb8c4_5572671"
		"sha2vz"		"e1fee3beffa9d08a415a35ddc7d3141af6c44269c67e67aa0a3eea6e56889bd7"
	}
	"bins_linuxarm64"
	{
		"file"		"bins_linuxarm64.zip.f86d2980"
		"size"		"76110471"
		"sha2"		"c2343282a34fe3da7400f39d1c26a57cdb3a8d320016b103092013246fd9ae65"
	}
	"steam_linuxarm64"
	{
		"file"		"steam_linuxarm64.zip.2d79dcdc"
		"size"		"4552071"
		"sha2"		"12fb90991a1ec79e3912cec1c06a23959cec21a18ccdab652a716048bce991a7"
		"IsBootstrapperPackage"		"1"
	}
	"bins_androidarm64_linuxarm64"
	{
		"file"		"bins_androidarm64_linuxarm64.zip.c4ba0833"
		"size"		"17941294"
		"sha2"		"ad6b9a0653302f8818c43fe8da4f5ff68e2ead6994069e5d93c4021ce2281d91"
	}
	"bins_linuxarm64_linuxarm64"
	{
		"file"		"bins_linuxarm64_linuxarm64.zip.dc817d33"
		"size"		"1000"
		"sha2"		"c2d7c0ecb8de7e209504cd657b785127cd1ac3d905668c296e87c360b2b54163"
	}
	"bins_steamrt_linuxarm64"
	{
		"file"		"bins_steamrt_linuxarm64.zip.05ace2dd"
		"sha2vz"		"e043d05b4e7bcc43e22cc2be3e8872ed1132e79e989cbbe181eb3a5d3f849df7"
	}
}
"kvsign2"
{
	"linuxarm64"		"63cf09"
}
"#;

    #[test]
    fn parses_version_and_components() {
        let m = parse(SAMPLE).unwrap();
        assert_eq!(m.platform, "linuxarm64");
        assert_eq!(m.version, "1792618285");
        // блок без file/size/sha2 пропускается
        assert_eq!(m.components.len(), 5);
        let t = &m.components[0];
        assert_eq!(t.name, "tenfoot_images_all");
        assert_eq!(t.file, "tenfoot_images_all.zip.86419c7a");
        assert_eq!(t.size, 6582204);
        assert_eq!(
            t.sha256,
            "990676bc29d01736477c6b571b5048258a6156284ef242503112fc68567cbf01"
        );
    }

    #[test]
    fn selects_all_and_own_platform() {
        let m = parse(SAMPLE).unwrap();
        let names: Vec<&str> = m.selected().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["tenfoot_images_all", "bins_linuxarm64_linuxarm64"]);
        assert_eq!(m.download_size(), 6582204 + 1000);
    }

    #[test]
    fn rejects_bad_manifests() {
        assert!(parse("").is_err());
        assert!(parse(
            r#""linuxarm64" { "version" "abc" "x_all" { "file" "x" "size" "1" "sha2" "00" } }"#
        )
        .is_err());
        let sha = "a".repeat(64);
        // путь в имени файла
        assert!(parse(&format!(
            r#""p" {{ "version" "1" "x_all" {{ "file" "../x" "size" "1" "sha2" "{sha}" }} }}"#
        ))
        .is_err());
        // нет закрывающей скобки
        assert!(parse(&format!(
            r#""p" {{ "version" "1" "x_all" {{ "file" "x" "size" "1" "sha2" "{sha}" }}"#
        ))
        .is_err());
        assert!(parse(&format!(
            r#""p" {{ "version" "1" "x_all" {{ "file" "x" "size" "1" "sha2" "{sha}" }} }}"#
        ))
        .is_ok());
    }
}
