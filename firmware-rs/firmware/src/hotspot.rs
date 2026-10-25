//! Wi-Fi access point, NAT-routed (lwIP NAPT) onto the modem's PPP uplink.

use anyhow::bail;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem as RadioModem;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::{self, esp, EspError};
use esp_idf_svc::wifi::{AccessPointConfiguration, AuthMethod, BlockingWifi, Configuration, EspWifi};
use log::info;

use crate::config;

const DHCPS_OFFER_DNS: u8 = 0x02;

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

pub fn start(
    radio: RadioModem<'static>,
    sysloop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<(BlockingWifi<EspWifi<'static>>, ApRouter)> {
    if config::AP_PASSWORD.len() < 8 {
        bail!("hotspot password must be at least 8 characters");
    }
    let mut wifi = BlockingWifi::wrap(EspWifi::new(radio, sysloop.clone(), Some(nvs))?, sysloop)?;
    wifi.set_configuration(&Configuration::AccessPoint(AccessPointConfiguration {
        ssid: config::AP_SSID.try_into().map_err(|_| anyhow::anyhow!("SSID too long"))?,
        password: config::AP_PASSWORD.try_into().map_err(|_| anyhow::anyhow!("password too long"))?,
        channel: config::AP_CHANNEL,
        auth_method: AuthMethod::WPA2WPA3Personal,
        max_connections: config::AP_MAX_CONN,
        ..Default::default()
    }))?;
    wifi.start()?;
    let ap = wifi.wifi().ap_netif().handle();
    info!("Hotspot \"{}\" up on channel {}", config::AP_SSID, config::AP_CHANNEL);
    Ok((wifi, ApRouter(ap)))
}
