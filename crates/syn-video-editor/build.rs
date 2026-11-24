//! Номер сборки для чипа в заголовке: число коммитов (как pkgrel пакета) и короткий хеш.

fn main() {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    println!("cargo:rustc-env=SVE_BUILD={}", git(&["rev-list", "--count", "HEAD"]).unwrap_or_else(|| "0".into()));
    println!("cargo:rustc-env=SVE_COMMIT={}", git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default());
    // журнал HEAD меняется при каждом коммите: пересобрать номер
    if let Some(p) = git(&["rev-parse", "--path-format=absolute", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={p}");
    }
}
