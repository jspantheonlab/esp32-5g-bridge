//! Factory reset by holding the BOOT button (GPIO0, present on the DevKit and the custom board).
//! Hold 5 s for a warning, 10 s to reset: clears settings saved from the app (hotspot name and
//! password) and all Bluetooth pairings, then restarts. SIM contents and modem settings are kept.

use std::ffi::CStr;
use std::thread;
use std::time::{Duration, Instant};

use bridge_core::button::{LongPress, PressEvent, RESET_AFTER_MS, WARN_AFTER_MS};
use bridge_core::protocol::Event;
use esp_idf_svc::hal::gpio::{Input, PinDriver};
use esp_idf_svc::sys;
use log::warn;

use crate::Emit;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// NVS namespace holding everything set from the app (see hotspot.rs).
const SETTINGS_NAMESPACE: &CStr = c"bridge";

pub fn spawn(button: PinDriver<'static, Input>, emit: Emit) -> anyhow::Result<()> {
    thread::Builder::new().name("reset".into()).stack_size(4096).spawn(move || run(button, emit))?;
    Ok(())
}

fn run(button: PinDriver<'static, Input>, emit: Emit) {
    let start = Instant::now();
    let mut press = LongPress::new();
    loop {
        // The BOOT button pulls GPIO0 low when pressed.
        let now_ms = start.elapsed().as_millis() as u32;
        match press.update(button.is_low(), now_ms) {
            Some(PressEvent::Warning) => {
                let left = (RESET_AFTER_MS - WARN_AFTER_MS) / 1000;
                warn!("Factory reset in {left} s; release the button to cancel");
                emit(Event::Notice {
                    msg: format!("Factory reset in {left} s. Release the bridge's button to cancel."),
                });
            }
            Some(PressEvent::Cancelled) => {
                warn!("Factory reset cancelled");
                emit(Event::Notice { msg: "Factory reset cancelled.".into() });
            }
            Some(PressEvent::Reset) => factory_reset(&emit),
            None => {}
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn factory_reset(emit: &Emit) -> ! {
    warn!("Factory reset: clearing saved settings and Bluetooth pairings");
    emit(Event::Notice {
        msg: "Factory reset: the bridge is restarting. Forget \"5G-Bridge\" in iPhone Settings → Bluetooth, then pair again."
            .into(),
    });
    thread::sleep(Duration::from_millis(500)); // let the notice go out over BLE

    unsafe {
        let mut handle: sys::nvs_handle_t = 0;
        if sys::nvs_open(SETTINGS_NAMESPACE.as_ptr(), sys::nvs_open_mode_t_NVS_READWRITE, &mut handle) == sys::ESP_OK {
            sys::nvs_erase_all(handle);
            sys::nvs_commit(handle);
            sys::nvs_close(handle);
        }
        sys::ble_store_clear();
        sys::esp_restart()
    }
}
