// Метка сборки для приветствия агента (видно в syn-vm --agent-status, какой агент поднялся).
fn main() {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = t / 86400;
    // дата UTC из дней (Говард Хиннант)
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let hm = (t % 86400) / 60;
    println!("cargo:rustc-env=SYN_BUILD={y:04}-{m:02}-{d:02} {:02}:{:02}", hm / 60, hm % 60);
    println!("cargo:rerun-if-changed=src");
}
