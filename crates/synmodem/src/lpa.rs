//! Загрузка профиля eSIM (SGP.22: ES9+ с сервером SM-DP+ оператора по TLS, ES10b на карте) —
//! это LPA целиком; её делает **lpac** (synmobile `hw/modem`, `/usr/lib/synmodem/lpac`, драйвер
//! APDU `qmi_qrtr` — QMI UIM по QRTR, как у synmodemd). Демон запускает его от root и
//! пересказывает ход загрузки событиями.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use synshell_tr::{n_, t};

pub const LPAC_DIR: &str = "/usr/lib/synmodem/lpac";

/// Код активации из QR оператора: `LPA:1$сервер$код[$OID[$1]]` (последнее `1` — нужен код
/// подтверждения). Возвращает (сервер, код сопоставления, нужен ли код подтверждения).
pub fn parse_code(code: &str) -> Option<(String, String, bool)> {
    let c = code.trim();
    let rest = c.strip_prefix("LPA:").or_else(|| c.strip_prefix("lpa:"))?;
    let mut it = rest.split('$');
    if it.next()? != "1" {
        return None;
    }
    let server = it.next()?.trim().to_string();
    if server.is_empty() || !server.contains('.') {
        return None;
    }
    let matching = it.next().unwrap_or("").to_string();
    let _oid = it.next();
    let conf = it.next() == Some("1");
    Some((server, matching, conf))
}

/// Шаг lpac → подпись для человека.
fn step_label(msg: &str) -> String {
    let s = match msg {
        m if m.starts_with("es10b_get_euicc_challenge") || m.starts_with("es10b_get_euicc_info") => n_!("Связь с eSIM"),
        m if m.starts_with("es9p_initiate_authentication") => n_!("Связь с сервером оператора"),
        m if m.starts_with("es10b_authenticate_server") => n_!("Проверка сервера оператора"),
        m if m.starts_with("es9p_authenticate_client") => n_!("Проверка eSIM сервером"),
        m if m.starts_with("es10b_prepare_download") => n_!("Подготовка загрузки"),
        m if m.starts_with("es9p_get_bound_profile_package") => n_!("Загрузка профиля"),
        m if m.starts_with("es10b_load_bound_profile_package") => n_!("Установка профиля"),
        m if m.starts_with("es8p_meta") => n_!("Сведения о профиле"),
        _ => return msg.replace('_', " "),
    };
    t!(s)
}

/// Загрузить профиль: `on_step` — подпись каждого шага.
pub fn download(slot: u8, code: &str, confirmation: &str, mut on_step: impl FnMut(String)) -> Result<()> {
    if parse_code(code).is_none() {
        bail!("{}", t!("неверный код активации: ожидается LPA:1$сервер$код"));
    }
    let lpac = std::path::Path::new(LPAC_DIR).join("lpac");
    if !lpac.exists() {
        bail!("{}", t!("нет lpac ({p}) — synmobile hw/modem/install.sh", p = lpac.display()));
    }
    let mut cmd = Command::new(&lpac);
    cmd.current_dir(LPAC_DIR)
        .env("LPAC_APDU", "qmi_qrtr")
        .env("LPAC_APDU_QMI_UIM_SLOT", slot.to_string())
        .args(["profile", "download", "-a", code.trim()]);
    if !confirmation.trim().is_empty() {
        cmd.args(["-c", confirmation.trim()]);
    }
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().with_context(|| lpac.display().to_string())?;
    let out = child.stdout.take().ok_or_else(|| anyhow!("lpac: нет stdout"))?;
    let mut result: Option<Result<()>> = None;
    for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        let p = &v["payload"];
        let msg = p["message"].as_str().unwrap_or("");
        match v["type"].as_str() {
            Some("progress") => on_step(step_label(msg)),
            Some("lpa") => {
                let code = p["code"].as_i64().unwrap_or(-1);
                result = Some(if code == 0 {
                    Ok(())
                } else {
                    let data = p["data"].as_str().map(String::from).unwrap_or_else(|| p["data"].to_string());
                    Err(anyhow!("{}", t!("lpac: {msg} ({data})", msg = msg, data = data)))
                });
            }
            _ => {}
        }
    }
    let status = child.wait()?;
    let err = child.stderr.take().map(|e| BufReader::new(e).lines().map_while(|l| l.ok()).collect::<Vec<_>>().join("; ")).unwrap_or_default();
    match result {
        Some(r) => r?,
        None if status.success() => {}
        None => bail!("{}", t!("lpac завершился с кодом {c}: {err}", c = status.code().unwrap_or(-1), err = err)),
    }
    // уведомления оператору о загрузке (SGP.22: «профиль установлен»)
    let _ = Command::new(&lpac)
        .current_dir(LPAC_DIR)
        .env("LPAC_APDU", "qmi_qrtr")
        .env("LPAC_APDU_QMI_UIM_SLOT", slot.to_string())
        .args(["notification", "process", "-a", "-r"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_codes() {
        assert_eq!(parse_code("LPA:1$smdp.example.com$ABC-123"), Some(("smdp.example.com".into(), "ABC-123".into(), false)));
        assert_eq!(parse_code(" LPA:1$rsp.oper.ru$X$1.2.3$1 "), Some(("rsp.oper.ru".into(), "X".into(), true)));
        assert_eq!(parse_code("LPA:1$sm-dp.net$"), Some(("sm-dp.net".into(), "".into(), false)));
        assert_eq!(parse_code("LPA:2$a.b$c"), None);
        assert_eq!(parse_code("https://example.com"), None);
        assert_eq!(parse_code("LPA:1$$c"), None);
        assert_eq!(step_label("es9p_get_bound_profile_package"), "Загрузка профиля");
    }
}
