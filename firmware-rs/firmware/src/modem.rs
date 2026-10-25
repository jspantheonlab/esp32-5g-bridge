//! Modem ownership: AT channel, URC delivery, and the supervisor thread that powers the
//! module on, registers, brings up CMUX + PPP, polls status and recovers from drops.

use std::ffi::{c_char, CStr, CString};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use bridge_core::at;
use bridge_core::framing::UrcReader;
use bridge_core::protocol::{Event, Status};
use esp_idf_svc::hal::gpio::{Output, PinDriver};
use esp_idf_svc::sys::{self, esp, modem_shim as shim, EspError};
use log::{info, warn};

use crate::config;
use crate::hotspot::ApRouter;
use crate::Emit;

/// Serializes access to the modem's AT channel.
pub struct Modem {
    lock: Mutex<()>,
}

/// Holds the AT channel for a multi-command sequence (e.g. switching the character set for SMS).
pub struct AtSession<'a> {
    _guard: MutexGuard<'a, ()>,
}

static URC_TX: OnceLock<SyncSender<String>> = OnceLock::new();
static URC_READER: Mutex<UrcReader> = Mutex::new(UrcReader::new(256));

/// Runs in the esp_modem DTE task: only split lines and hand them off.
/// esp_modem re-delivers its unconsumed buffer on each call; `UrcReader` keeps only new bytes.
unsafe extern "C" fn urc_rx(data: *const u8, len: usize) {
    if data.is_null() || len == 0 {
        return;
    }
    let bytes = std::slice::from_raw_parts(data, len);
    let lines = URC_READER.lock().map(|mut r| r.feed(bytes)).unwrap_or_default();
    if let Some(tx) = URC_TX.get() {
        for line in lines {
            let _ = tx.try_send(line);
        }
    }
}

fn invalid_arg() -> EspError {
    EspError::from(sys::ESP_ERR_INVALID_ARG as sys::esp_err_t).expect("non-zero error code")
}

impl Modem {
    pub fn init(urc_tx: SyncSender<String>) -> anyhow::Result<Arc<Self>> {
        URC_TX.set(urc_tx).map_err(|_| anyhow!("modem already initialized"))?;
        let apn = CString::new(config::APN)?;
        let cfg = shim::modem_shim_config_t {
            uart_port: config::MODEM_UART_PORT,
            tx_gpio: config::MODEM_TX_GPIO,
            rx_gpio: config::MODEM_RX_GPIO,
            rts_gpio: config::MODEM_RTS_GPIO,
            cts_gpio: config::MODEM_CTS_GPIO,
            baud_rate: config::MODEM_BAUD,
            apn: apn.as_ptr(),
        };
        esp!(unsafe { shim::modem_shim_init(&cfg) }).context("modem_shim_init")?;
        esp!(unsafe { shim::modem_shim_set_urc_handler(Some(urc_rx)) }).context("URC handler")?;
        Ok(Arc::new(Self { lock: Mutex::new(()) }))
    }

    pub fn session(&self) -> AtSession<'_> {
        AtSession { _guard: self.lock.lock().unwrap_or_else(|e| e.into_inner()) }
    }

    /// Sends `cmd\r` and waits for OK / ERROR. Returns the full response.
    pub fn at(&self, cmd: &str, timeout_ms: u32) -> Result<String, EspError> {
        self.session().at(cmd, timeout_ms)
    }
}

impl AtSession<'_> {
    pub fn at(&self, cmd: &str, timeout_ms: u32) -> Result<String, EspError> {
        self.raw(&format!("{cmd}\r"), "OK", "ERROR", timeout_ms)
    }

    /// Sends raw bytes (caller adds terminators) and waits for `pass` / `fail`.
    pub fn raw(&self, data: &str, pass: &str, fail: &str, timeout_ms: u32) -> Result<String, EspError> {
        let data = CString::new(data).map_err(|_| invalid_arg())?;
        let pass = CString::new(pass).map_err(|_| invalid_arg())?;
        let fail = CString::new(fail).map_err(|_| invalid_arg())?;
        let mut out = vec![0u8; unsafe { shim::modem_shim_resp_max() }];
        esp!(unsafe {
            shim::modem_shim_at_raw(
                data.as_ptr(),
                pass.as_ptr(),
                fail.as_ptr(),
                timeout_ms as i32,
                out.as_mut_ptr() as *mut c_char,
                out.len(),
            )
        })?;
        Ok(CStr::from_bytes_until_nul(&out).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
    }
}

fn ppp_ip() -> Option<String> {
    let mut buf = [0u8; 16];
    let up = unsafe { shim::modem_shim_ppp_ip(buf.as_mut_ptr() as *mut c_char, buf.len()) };
    up.then(|| CStr::from_bytes_until_nul(&buf).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}

pub struct Supervisor {
    pub modem: Arc<Modem>,
    pub pwrkey: PinDriver<'static, Output>,
    pub status: Arc<Mutex<Status>>,
    pub router: ApRouter,
    pub emit: Emit,
}

pub fn spawn_supervisor(sup: Supervisor) -> anyhow::Result<()> {
    thread::Builder::new().name("modem".into()).stack_size(8192).spawn(move || sup.run())?;
    Ok(())
}

impl Supervisor {
    fn run(mut self) {
        self.set_pwrkey(false);
        loop {
            match self.bring_up() {
                Ok(()) => {
                    self.monitor();
                    warn!("Data session lost, restarting");
                }
                Err(e) => warn!("Modem bring-up failed: {e:#}"),
            }
            unsafe { shim::modem_shim_enter_command() };
            thread::sleep(Duration::from_secs(10));
        }
    }

    fn set_pwrkey(&mut self, pressed: bool) {
        let _ = if pressed == config::PWRKEY_ACTIVE_HIGH { self.pwrkey.set_high() } else { self.pwrkey.set_low() };
    }

    fn pulse_pwrkey(&mut self) {
        info!("Pulsing PWRKEY");
        self.set_pwrkey(true);
        thread::sleep(Duration::from_millis(600));
        self.set_pwrkey(false);
    }

    fn sync_at(baud: i32, attempts: u32) -> bool {
        unsafe { shim::modem_shim_set_local_baud(baud) };
        (0..attempts).any(|_| {
            let ok = unsafe { shim::modem_shim_sync() } == sys::ESP_OK;
            if !ok {
                thread::sleep(Duration::from_millis(500));
            }
            ok
        })
    }

    /// Returns the baud rate the modem answered on.
    fn find_modem(attempts: u32) -> Option<i32> {
        [config::MODEM_BAUD, 115_200].into_iter().find(|&baud| Self::sync_at(baud, attempts))
    }

    fn power_and_sync(&mut self) -> anyhow::Result<()> {
        let mut baud = Self::find_modem(2);
        if baud.is_none() {
            self.pulse_pwrkey();
            baud = (0..10).find_map(|_| Self::find_modem(3));
        }
        let baud = baud.ok_or_else(|| anyhow!("modem not responding"))?;
        if baud != config::MODEM_BAUD {
            let _ = self.modem.at(&format!("AT+IPR={}", config::MODEM_BAUD), 1000);
            thread::sleep(Duration::from_millis(100));
            if !Self::sync_at(config::MODEM_BAUD, 6) {
                bail!("modem lost after baud switch");
            }
            let _ = self.modem.at("AT&W", 1000);
        }
        info!("Modem responding at {} baud", config::MODEM_BAUD);
        Ok(())
    }

    fn wait_sim_ready(&self) -> anyhow::Result<()> {
        for _ in 0..20 {
            if let Ok(resp) = self.modem.at("AT+CPIN?", 2000) {
                match at::parse_cpin(&resp) {
                    Some("READY") => return Ok(()),
                    Some(code) if code.contains("PUK") => bail!("SIM is PUK-locked"),
                    Some("SIM PIN") if !config::SIM_PIN.is_empty() => {
                        let _ = self.modem.at(&format!("AT+CPIN=\"{}\"", config::SIM_PIN), 5000);
                    }
                    _ => {}
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
        bail!("SIM not ready")
    }

    fn wait_registered(&self, timeout: Duration) -> anyhow::Result<()> {
        const QUERIES: [(&str, &str); 2] = [("AT+C5GREG?", "+C5GREG:"), ("AT+CEREG?", "+CEREG:")];
        let attempts = timeout.as_secs() / 2;
        for _ in 0..attempts {
            for (cmd, prefix) in QUERIES {
                let stat = self.modem.at(cmd, 2000).ok().and_then(|r| at::parse_registration(&r, prefix));
                if let Some(stat) = stat.filter(|s| at::is_registered(*s)) {
                    info!("Registered ({prefix} stat={stat})");
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
        bail!("network registration timed out")
    }

    fn bring_up(&mut self) -> anyhow::Result<()> {
        self.power_and_sync()?;

        for cmd in ["ATE0", "AT+CMEE=2", "AT+CSCS=\"IRA\"", "AT+COPS=3,0"] {
            let _ = self.modem.at(cmd, 1000);
        }
        self.wait_sim_ready()?;

        if config::ENABLE_IMS {
            let _ = self.modem.at("AT+QCFG=\"ims\",1", 2000);
        }
        if !config::PCM_DAI_CMD.is_empty() {
            let _ = self.modem.at(config::PCM_DAI_CMD, 2000);
        }
        for cmd in ["AT+CLIP=1", "AT+CMGF=1", "AT+CNMI=2,1,0,0,0"] {
            let _ = self.modem.at(cmd, 1000);
        }

        self.wait_registered(Duration::from_secs(180))?;
        self.refresh_status();

        // CMUX: one virtual channel carries PPP, another keeps AT available for calls and SMS.
        esp!(unsafe { shim::modem_shim_enter_cmux() }).context("enter CMUX/PPP")?;
        for _ in 0..30 {
            if ppp_ip().is_some() {
                self.on_data_up();
                return Ok(());
            }
            thread::sleep(Duration::from_secs(1));
        }
        bail!("PPP did not come up")
    }

    fn on_data_up(&self) {
        // Same C type, generated in a separate bindings module.
        let ppp = unsafe { shim::modem_shim_ppp_netif() }.cast();
        if let Err(e) = self.router.set_uplink(ppp) {
            warn!("Hotspot routing failed: {e}");
        }
        self.refresh_status();
    }

    fn monitor(&self) {
        let mut seconds = 0u32;
        loop {
            thread::sleep(Duration::from_secs(1));
            if ppp_ip().is_none() {
                self.router.clear_uplink();
                self.refresh_status();
                return;
            }
            seconds += 1;
            if seconds % 10 == 0 {
                self.refresh_status();
            }
        }
    }

    fn refresh_status(&self) {
        let sim = self.modem.at("AT+CPIN?", 2000).ok().is_some_and(|r| at::parse_cpin(&r) == Some("READY"));
        let rssi = self.modem.at("AT+CSQ", 2000).ok().and_then(|r| at::parse_csq(&r));
        let operator = self.modem.at("AT+COPS?", 3000).ok().and_then(|r| at::parse_cops(&r));
        let ip = ppp_ip();

        let snapshot = {
            let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
            st.sim = sim;
            st.rssi = rssi;
            st.reg = operator.as_ref().is_some_and(|o| !o.name.is_empty());
            st.rat = at::rat_name(operator.as_ref().and_then(|o| o.act)).into();
            st.op = operator.map(|o| o.name).unwrap_or_default();
            st.data = ip.is_some();
            st.ip = ip.unwrap_or_default();
            st.clone()
        };
        (self.emit)(Event::Status(snapshot));
    }
}
