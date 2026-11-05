//! ИК-передатчик для Android — `syndroidd __ir`: HIDL-сервис `android.hardware.ir@1.0::IConsumerIr/default`
//! по hwbinder контейнера (Android 13: ConsumerIrService берёт HIDL). Передаёт программа платформы
//! `/usr/lib/syndroid/ir-transmit ЧАСТОТА импульс пауза …` (мкс, как `pattern` Android). Объявление HAL в VINTF и
//! функцию `android.hardware.consumerir` кладёт в слой vendor сборка ОС устройства вместе с программой: без
//! HAL при объявленной функции system_server падает при загрузке.

use std::process::Command;
use std::sync::Arc;

use anyhow::{bail, Context, Result};

use crate::hwbinder::{self, HwParcel, HwReader, Service};

pub const TRANSMIT: &str = "/usr/lib/syndroid/ir-transmit";
const ICONSUMERIR: &str = "android.hardware.ir@1.0::IConsumerIr";
/// Частоты несущей, которые умеет светодиод (Гц).
const FREQ_MIN: u32 = 30_000;
const FREQ_MAX: u32 = 60_000;

struct ConsumerIr;

impl Service for ConsumerIr {
    fn chain(&self) -> &'static [&'static str] {
        &[ICONSUMERIR]
    }

    fn call(&self, code: u32, r: &mut HwReader, reply: &mut HwParcel) -> Result<()> {
        match code {
            // transmit(int32 carrierFreq, vec<int32> pattern) generates (bool success)
            1 => {
                let freq = r.i32()?;
                let pattern = r.vec_i32()?;
                let ok = transmit(freq, &pattern);
                reply.ok();
                reply.i32(ok as i32);
            }
            // getCarrierFreqs() generates (bool success, vec<ConsumerIrFreqRange{uint32 min, max}> ranges)
            2 => {
                reply.ok();
                reply.i32(1);
                let mut b = FREQ_MIN.to_le_bytes().to_vec();
                b.extend_from_slice(&FREQ_MAX.to_le_bytes());
                reply.vec(b, 1);
            }
            _ => bail!("неизвестный метод IConsumerIr {code}"),
        }
        Ok(())
    }
}

fn transmit(freq: i32, pattern: &[i32]) -> bool {
    if pattern.is_empty() || pattern.iter().any(|&v| v <= 0) || !(FREQ_MIN as i32..=FREQ_MAX as i32).contains(&freq) {
        tracing::warn!("ИК: неверный сигнал ({freq} Гц, {} отрезков)", pattern.len());
        return false;
    }
    let mut cmd = Command::new(TRANSMIT);
    cmd.arg(freq.to_string()).args(pattern.iter().map(|v| v.to_string()));
    match cmd.status() {
        Ok(s) if s.success() => {
            tracing::info!("ИК: {freq} Гц, {} отрезков", pattern.len());
            true
        }
        Ok(s) => {
            tracing::warn!("ИК: {TRANSMIT}: {s}");
            false
        }
        Err(e) => {
            tracing::warn!("ИК: {TRANSMIT}: {e}");
            false
        }
    }
}

/// `syndroidd __ir <узел hwbinder>`
pub fn ir_main(args: &[String]) -> ! {
    let dev = args.first().cloned().unwrap_or_default();
    if let Err(e) = hwbinder::serve(&dev, "default", Arc::new(ConsumerIr)).context("IConsumerIr") {
        eprintln!("syndroid: ИК: {e:#}");
        std::process::exit(1);
    }
    std::process::exit(0)
}
