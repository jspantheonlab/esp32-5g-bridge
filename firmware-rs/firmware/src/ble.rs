//! BLE GATT control link to the iPhone app (newline-delimited JSON, passkey-bonded).
//!
//!   Service A5E10001-5C0E-4B6A-9D3E-6B2F0C5A1E01
//!   RX      A5E10002-…  write   (iPhone → bridge)
//!   TX      A5E10003-…  notify  (bridge → iPhone)

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bridge_core::framing::LineAssembler;
use esp32_nimble::enums::{AuthReq, SecurityIOCap};
use esp32_nimble::utilities::mutex::Mutex as NimbleMutex;
use esp32_nimble::{uuid128, BLEAdvertisementData, BLECharacteristic, BLEDevice, NimbleProperties};
use esp_idf_svc::sys;
use log::{info, warn};

use crate::config;

const SERVICE_UUID: &str = "a5e10001-5c0e-4b6a-9d3e-6b2f0c5a1e01";
const RX_UUID: &str = "a5e10002-5c0e-4b6a-9d3e-6b2f0c5a1e01";
const TX_UUID: &str = "a5e10003-5c0e-4b6a-9d3e-6b2f0c5a1e01";
const NOTIFY_RETRIES: u32 = 50;

pub enum LinkEvent {
    Line(String),
    /// Link encrypted and the phone subscribed to TX.
    Ready,
}

#[derive(Default)]
struct Peer {
    conn_handle: Option<u16>,
    encrypted: bool,
    subscribed: bool,
    ready: bool,
}

impl Peer {
    /// Returns true when the link just became ready.
    fn update_ready(&mut self) -> bool {
        let ready = self.conn_handle.is_some() && self.encrypted && self.subscribed;
        let became_ready = ready && !self.ready;
        self.ready = ready;
        became_ready
    }
}

pub struct BleLink {
    tx: Arc<NimbleMutex<BLECharacteristic>>,
    peer: Arc<Mutex<Peer>>,
    send_lock: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl BleLink {
    pub fn start(events: Sender<LinkEvent>) -> anyhow::Result<Arc<Self>> {
        let device = BLEDevice::take();
        BLEDevice::set_device_name(config::BLE_NAME)?;
        // Passkey pairing: the bridge "displays" a fixed passkey that the user types on the iPhone.
        device
            .security()
            .set_auth(AuthReq::Bond | AuthReq::Mitm | AuthReq::Sc)
            .set_passkey(config::BLE_PASSKEY)
            .set_io_cap(SecurityIOCap::DisplayOnly)
            .resolve_rpa();

        let peer = Arc::new(Mutex::new(Peer::default()));
        let server = device.get_server();
        server.advertise_on_disconnect(true);

        server.on_connect({
            let peer = peer.clone();
            move |_, desc| {
                info!("BLE connected ({})", desc.address());
                *lock(&peer) = Peer { conn_handle: Some(desc.conn_handle()), ..Default::default() };
            }
        });
        server.on_disconnect({
            let peer = peer.clone();
            move |_, reason| {
                info!("BLE disconnected ({reason:?})");
                *lock(&peer) = Peer::default();
            }
        });
        server.on_authentication_complete({
            let peer = peer.clone();
            let events = events.clone();
            move |server, desc, result| {
                if let Err(e) = result {
                    warn!("BLE pairing failed: {e:?}");
                    let _ = server.disconnect(desc.conn_handle());
                    return;
                }
                let mut p = lock(&peer);
                p.encrypted = desc.encrypted();
                if p.update_ready() {
                    let _ = events.send(LinkEvent::Ready);
                }
            }
        });

        let service = server.create_service(uuid128!(SERVICE_UUID));
        let rx = service.lock().create_characteristic(
            uuid128!(RX_UUID),
            NimbleProperties::WRITE | NimbleProperties::WRITE_NO_RSP | NimbleProperties::WRITE_ENC,
        );
        let tx = service.lock().create_characteristic(
            uuid128!(TX_UUID),
            NimbleProperties::READ | NimbleProperties::READ_ENC | NimbleProperties::NOTIFY,
        );

        let lines = Mutex::new(LineAssembler::new(1024, false));
        rx.lock().on_write({
            let events = events.clone();
            move |args| {
                for line in lock(&lines).feed(args.recv_data()) {
                    let _ = events.send(LinkEvent::Line(line));
                }
            }
        });
        tx.lock().on_subscribe({
            let peer = peer.clone();
            move |_, desc, sub| {
                let mut p = lock(&peer);
                p.subscribed = !sub.is_empty();
                p.encrypted = desc.encrypted();
                if p.update_ready() {
                    let _ = events.send(LinkEvent::Ready);
                }
            }
        });

        let advertising = device.get_advertising();
        advertising
            .lock()
            .set_data(BLEAdvertisementData::new().name(config::BLE_NAME).add_service_uuid(uuid128!(SERVICE_UUID)))?;
        advertising.lock().start()?;
        info!("BLE advertising as \"{}\"", config::BLE_NAME);

        Ok(Arc::new(Self { tx, peer, send_lock: Mutex::new(()) }))
    }

    /// Sends one line, fragmented to the ATT MTU. Dropped if the phone isn't connected.
    pub fn send(&self, line: &str) {
        let _guard = lock(&self.send_lock);
        let conn = match &*lock(&self.peer) {
            Peer { conn_handle: Some(h), ready: true, .. } => *h,
            _ => return,
        };
        let mtu = unsafe { sys::ble_att_mtu(conn) } as usize;
        let chunk_len = mtu.saturating_sub(3).max(20);

        for chunk in line.as_bytes().chunks(chunk_len) {
            let mut attempt = 0;
            loop {
                match self.tx.lock().notify_with(chunk, conn) {
                    Ok(()) => break,
                    Err(e)
                        if attempt < NOTIFY_RETRIES && matches!(e.code(), sys::BLE_HS_ENOMEM | sys::BLE_HS_EBUSY) =>
                    {
                        attempt += 1;
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(e) => {
                        warn!("BLE notify failed: {e:?}");
                        return;
                    }
                }
            }
        }
    }
}
