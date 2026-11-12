//! Системная служба нейросетей: сервер — `synaptixd::server`, тот же, что
//! программы поднимают у себя без службы (`server::embedded`).
fn main() -> anyhow::Result<()> {
    synaptixd::server::main()
}
