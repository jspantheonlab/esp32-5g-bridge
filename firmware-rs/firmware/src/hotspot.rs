//! Wi-Fi access point, NAT-routed (lwIP NAPT) onto the modem's PPP uplink.
//! Credentials can be changed from the phone; they are stored in NVS and override the
//! build-time defaults in `config.rs`.

use anyhow::Context;
use bridge_core::wifi;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem as RadioModem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sys::{self, esp, EspError};
use esp_idf_svc::wifi::{AccessPointConfiguration, AuthMethod, BlockingWifi, Configuration, EspWifi};
use log::{info, warn};

use crate::config;

const DHCPS_OFFER_DNS: u8 = 0x02;
const NVS_NAMESPACE: &str = "bridge";
const NVS_SSID: &str = "ap_ssid";
const NVS_PASS: &str = "ap_pass";

/// Handle to the AP netif, used by the modem thread to (un)route clients.
#[derive(Clone, Copy)]
pub struct ApRouter(*mut sys::esp_netif_t);

// SAFETY: esp_netif calls are thread-safe; the AP netif lives as long as the Wi-Fi driver (forever).
unsafe impl Send for ApRouter {}
unsafe impl Sync for ApRouter {}

impl ApRouter {
    /// Hands out the uplink's DNS server over DHCP and enables NAPT from the AP to the uplink.
    pub fn set_uplink(&self, uplink: *mut sys::esp_netif_t) -> Result<(), EspError> {
        unsafe {
            let mut dns: sys::esp_netif_dns_info_t = core::mem::zeroed();
            if sys::esp_netif_get_dns_info(uplink, sys::esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN, &mut dns)
                == sys::ESP_OK
            {
                let mut offer = DHCPS_OFFER_DNS;
                sys::esp_netif_dhcps_stop(self.0);
                sys::esp_netif_dhcps_option(
                    self.0,
                    sys::esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
                    sys::esp_netif_dhcp_option_id_t_ESP_NETIF_DOMAIN_NAME_SERVER,
                    &mut offer as *mut u8 as *mut core::ffi::c_void,
                    1,
                );
                sys::esp_netif_set_dns_info(self.0, sys::esp_netif_dns_type_t_ESP_NETIF_DNS_MAIN, &mut dns);
                sys::esp_netif_dhcps_start(self.0);
            }
            esp!(sys::esp_netif_set_default_netif(uplink))?;
            esp!(sys::esp_netif_napt_enable(self.0))?;
        }
        info!("NAPT enabled: hotspot clients routed over 5G");
        Ok(())
    }

    pub fn clear_uplink(&self) {
        unsafe { sys::esp_netif_napt_disable(self.0) };
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub ssid: String,
    pub pass: String,
}

pub struct Hotspot {
    wifi: BlockingWifi<EspWifi<'static>>,
    nvs: EspNvs<NvsDefault>,
    router: ApRouter,
    creds: Credentials,
}

impl Hotspot {
    pub fn start(
        radio: RadioModem<'static>,
        sysloop: EspSystemEventLoop,
        nvs: EspDefaultNvsPartition,
    ) -> anyhow::Result<Self> {
        let store = EspNvs::new(nvs.clone(), NVS_NAMESPACE, true)?;
        let creds = load_credentials(&store);
        let wifi = BlockingWifi::wrap(EspWifi::new(radio, sysloop.clone(), Some(nvs))?, sysloop)?;
        let router = ApRouter(wifi.wifi().ap_netif().handle());
        let mut hotspot = Self { wifi, nvs: store, router, creds };
        hotspot.apply().context("starting hotspot")?;
        Ok(hotspot)
    }

    pub fn router(&self) -> ApRouter {
        self.router
    }

    pub fn credentials(&self) -> &Credentials {
        &self.creds
    }

    /// Validates, applies (connected clients drop and must rejoin) and persists new credentials.
    /// The caller should re-enable routing afterwards (`modem::reapply_uplink`).
    pub fn set_credentials(&mut self, ssid: &str, pass: &str) -> anyhow::Result<()> {
        wifi::validate(ssid, pass)?;
        let previous = std::mem::replace(&mut self.creds, Credentials { ssid: ssid.into(), pass: pass.into() });
        if let Err(e) = self.apply() {
            warn!("Applying new hotspot settings failed ({e}); restoring previous settings");
            self.creds = previous;
            self.apply()?;
            return Err(e.into());
        }
        self.nvs.set_str(NVS_SSID, ssid)?;
        self.nvs.set_str(NVS_PASS, pass)?;
        info!("Hotspot settings changed (SSID \"{ssid}\")");
        Ok(())
    }

    fn apply(&mut self) -> Result<(), EspError> {
        let config = Configuration::AccessPoint(AccessPointConfiguration {
            // Lengths are guaranteed by wifi::validate / load_credentials.
            ssid: self.creds.ssid.as_str().try_into().unwrap_or_default(),
            password: self.creds.pass.as_str().try_into().unwrap_or_default(),
            channel: config::AP_CHANNEL,
            auth_method: AuthMethod::WPA2WPA3Personal,
            max_connections: config::AP_MAX_CONN,
            ..Default::default()
        });
        if self.wifi.is_started()? {
            self.wifi.stop()?;
        }
        self.wifi.set_configuration(&config)?;
        self.wifi.start()?;
        info!("Hotspot \"{}\" up on channel {}", self.creds.ssid, config::AP_CHANNEL);
        Ok(())
    }
}

/// Saved credentials if valid, otherwise the build-time defaults.
fn load_credentials(store: &EspNvs<NvsDefault>) -> Credentials {
    let mut ssid_buf = [0u8; wifi::MAX_SSID_BYTES + 1];
    let mut pass_buf = [0u8; wifi::MAX_PASS_LEN + 1];
    let saved = match (store.get_str(NVS_SSID, &mut ssid_buf), store.get_str(NVS_PASS, &mut pass_buf)) {
        (Ok(Some(ssid)), Ok(Some(pass))) if wifi::validate(ssid, pass).is_ok() => {
            Some(Credentials { ssid: ssid.into(), pass: pass.into() })
        }
        _ => None,
    };
    saved.unwrap_or_else(|| Credentials { ssid: config::AP_SSID.into(), pass: config::AP_PASSWORD.into() })
}
