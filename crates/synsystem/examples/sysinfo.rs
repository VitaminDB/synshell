//! Снимок системы в терминал — проверка читателей на живом устройстве:
//! `cargo run -p synsystem --example sysinfo`.

use synsystem::*;

fn main() {
    let sys = Sys::host();
    println!("CPU: {:?} ({:?})", cpu::model(&sys), cpu::device_model(&sys));
    let mut s = cpu::CpuSampler::new(sys.clone());
    for c in s.clusters() {
        println!("  кластер {}: ядра {:?}, {:?}–{:?} МГц, {:?}", c.policy, c.cpus, c.min_mhz, c.max_mhz, c.governor);
    }
    s.sample();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let snap = s.sample();
    println!("  загрузка {:.0}%", snap.usage);
    for c in &snap.cores {
        println!("  cpu{} {:>5.1}% {:?} МГц кластер {} класс {}", c.index, c.usage, c.cur_mhz, c.cluster, c.tier);
    }
    if let Some(m) = memory::read(&sys) {
        println!("Память: {} из {} ({:.0}%)", memory::human_kb(m.used_kb()), memory::human_kb(m.total_kb), m.used_percent());
    }
    println!("Батарея: {:?}", battery::read(&sys));
    for b in battery::batteries(&sys) {
        println!("  {b:?}");
    }
    let t = thermal::sensors(&sys);
    println!("Температуры: {} датчиков, CPU {:?} °C, GPU {:?} °C", t.len(), thermal::cpu_celsius(&t), thermal::gpu_celsius(&t));
    println!("GPU: {:?}", gpu::read(&sys));
    println!("Подсветка: {:?}", backlight::list(&sys));
    println!("Сеть: {:?}, трафик {:?}", network::read(&sys), network::traffic(&sys));
    let mut p = procs::ProcSampler::new(sys.clone());
    let me = std::process::id() as i32;
    p.sample(&[me, 1]);
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("Процессы: {:?}", p.sample(&[me, 1]));
    for b in ["iwd", "networkmanager"] {
        match wifi::backend(b) {
            Some(w) => match w.state() {
                Ok(st) => println!("Wi-Fi {}: включён {} подключено {:?} ip {:?} сетей {} (первая {:?})", w.name(), st.powered, st.connected, st.ip, st.networks.len(), st.networks.first()),
                Err(e) => println!("Wi-Fi {}: {e}", w.name()),
            },
            None => println!("Wi-Fi {b}: нет"),
        }
    }
    match bluetooth::Bluetooth::new().map(|b| b.state()) {
        Some(Ok(st)) => println!("Bluetooth: {:?} включён {} устройств {} {:?}", st.adapter, st.powered, st.devices.len(), st.devices.iter().map(|d| (&d.name, d.paired, d.connected)).collect::<Vec<_>>()),
        Some(Err(e)) => println!("Bluetooth: {e}"),
        None => println!("Bluetooth: нет bluez"),
    }
}
