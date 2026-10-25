//! Battery monitoring for the custom board (docs/board-design.md): MAX17048 fuel gauge and
//! BQ25895 charger on I2C. Disabled automatically when no gauge answers (e.g. a DevKit on USB).
//! The charger's register defaults (2 A, 4.208 V) match the board design, so it is only read.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bridge_core::power::{self, BQ25895_ADDR, BQ25895_REG_STATUS, MAX17048_ADDR, MAX17048_REG_SOC};
use bridge_core::protocol::{Event, Status};
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sys;
use log::{info, warn};

use crate::config;
use crate::modem::Modem;
use crate::Emit;

const POLL_INTERVAL: Duration = Duration::from_secs(30);

pub struct PowerMonitor {
    pub i2c: I2cDriver<'static>,
    pub status: Arc<Mutex<Status>>,
    pub modem: Arc<Modem>,
    pub emit: Emit,
}

pub fn spawn(monitor: PowerMonitor) -> anyhow::Result<()> {
    thread::Builder::new().name("power".into()).stack_size(4096).spawn(move || monitor.run())?;
    Ok(())
}

impl PowerMonitor {
    fn read_reg<const N: usize>(&mut self, addr: u8, reg: u8) -> Option<[u8; N]> {
        let mut buf = [0u8; N];
        let timeout = TickType::from(Duration::from_millis(50)).into();
        self.i2c.write_read(addr, &[reg], &mut buf, timeout).ok().map(|_| buf)
    }

    fn run(mut self) {
        if self.read_reg::<2>(MAX17048_ADDR, MAX17048_REG_SOC).is_none() {
            info!("No battery gauge found; battery reporting disabled");
            return;
        }
        info!("Battery gauge found");
        loop {
            let percent = self.read_reg::<2>(MAX17048_ADDR, MAX17048_REG_SOC).map(power::soc_percent);
            let charger = self.read_reg::<1>(BQ25895_ADDR, BQ25895_REG_STATUS).map(|r| r[0]);
            let charging = charger.map(|r| power::charge_state(r).is_charging());
            let usb = charger.map(power::input_power_good);

            let changed = {
                let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
                let changed = st.bat != percent || st.chg != charging || st.usb != usb;
                st.bat = percent;
                st.chg = charging;
                st.usb = usb;
                changed.then(|| st.clone())
            };
            if let Some(snapshot) = changed {
                (self.emit)(Event::Status(snapshot));
            }

            if let Some(p) = percent {
                if power::is_critical(p, usb.unwrap_or(false)) {
                    self.shutdown(p);
                }
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Powers the modem down cleanly, then deep-sleeps until the power button is pressed.
    fn shutdown(&mut self, percent: u8) -> ! {
        warn!("Battery critical ({percent}%), shutting down");
        (self.emit)(Event::error("power", format!("battery critical ({percent}%), shutting down")));
        let _ = self.modem.at("AT+QPOWD=1", 5000);
        thread::sleep(Duration::from_secs(3));
        unsafe {
            sys::esp_sleep_enable_ext0_wakeup(config::POWER_BUTTON_GPIO, 0);
            sys::esp_deep_sleep_start();
        }
    }
}
