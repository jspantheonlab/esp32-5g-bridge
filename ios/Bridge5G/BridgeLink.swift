import CoreBluetooth
import Foundation
import Observation

/// BLE control link to the ESP32. Newline-delimited JSON over two GATT characteristics.
/// Uses `bluetooth-central` background mode + state restoration so incoming calls/SMS
/// wake the app even when it is suspended.
@Observable
final class BridgeLink: NSObject {
    enum State: String { case poweredOff, scanning, connecting, connected, ready }

    static let serviceUUID = CBUUID(string: "A5E10001-5C0E-4B6A-9D3E-6B2F0C5A1E01")
    static let rxUUID = CBUUID(string: "A5E10002-5C0E-4B6A-9D3E-6B2F0C5A1E01")
    static let txUUID = CBUUID(string: "A5E10003-5C0E-4B6A-9D3E-6B2F0C5A1E01")
    private static let restoreID = "bridge5g.central"
    private static let knownPeripheralKey = "bridge5g.peripheral"

    private(set) var state: State = .poweredOff
    /// The bridge no longer knows this iPhone (e.g. after a factory reset). iOS keeps its old
    /// pairing keys until the user forgets the device in Settings → Bluetooth.
    private(set) var pairingLost = false

    @ObservationIgnored var onMessage: (([String: Any]) -> Void)?
    @ObservationIgnored var onReady: (() -> Void)?

    @ObservationIgnored private var central: CBCentralManager!
    @ObservationIgnored private var peripheral: CBPeripheral?
    @ObservationIgnored private var rx: CBCharacteristic?
    @ObservationIgnored private var inbound = Data()

    override init() {
        super.init()
        central = CBCentralManager(
            delegate: self, queue: .main,
            options: [CBCentralManagerOptionRestoreIdentifierKey: Self.restoreID])
    }

    func send(_ message: [String: Any]) {
        guard state == .ready, let peripheral, let rx,
              var data = try? JSONSerialization.data(withJSONObject: message) else { return }
        data.append(0x0A)
        let chunk = peripheral.maximumWriteValueLength(for: .withResponse)
        var offset = 0
        while offset < data.count {
            let end = min(offset + chunk, data.count)
            peripheral.writeValue(data.subdata(in: offset..<end), for: rx, type: .withResponse)
            offset = end
        }
    }

    func forgetDevice() {
        UserDefaults.standard.removeObject(forKey: Self.knownPeripheralKey)
        if let peripheral { central.cancelPeripheralConnection(peripheral) }
    }

    private func connectOrScan() {
        guard central.state == .poweredOn else { return }
        if let idString = UserDefaults.standard.string(forKey: Self.knownPeripheralKey),
           let id = UUID(uuidString: idString),
           let known = central.retrievePeripherals(withIdentifiers: [id]).first {
            connect(known)
        } else {
            state = .scanning
            central.scanForPeripherals(withServices: [Self.serviceUUID])
        }
    }

    private func connect(_ p: CBPeripheral) {
        peripheral = p
        p.delegate = self
        state = .connecting
        // A pending connect never times out, so this also reconnects when the bridge comes back in range.
        central.connect(p, options: [CBConnectPeripheralOptionNotifyOnDisconnectionKey: true])
    }

    private func handle(_ data: Data) {
        inbound.append(data)
        while let newline = inbound.firstIndex(of: 0x0A) {
            let line = inbound[inbound.startIndex..<newline]
            inbound.removeSubrange(inbound.startIndex...newline)
            if let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
                onMessage?(obj)
            }
        }
    }
}

extension BridgeLink: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn {
            if let peripheral, peripheral.state == .connected {
                peripheral.discoverServices([Self.serviceUUID])
            } else {
                connectOrScan()
            }
        } else {
            state = .poweredOff
        }
    }

    func centralManager(_ central: CBCentralManager, willRestoreState dict: [String: Any]) {
        if let restored = (dict[CBCentralManagerRestoredStatePeripheralsKey] as? [CBPeripheral])?.first {
            peripheral = restored
            restored.delegate = self
        }
    }

    func centralManager(_ central: CBCentralManager, didDiscover p: CBPeripheral,
                        advertisementData: [String: Any], rssi RSSI: NSNumber) {
        central.stopScan()
        connect(p)
    }

    func centralManager(_ central: CBCentralManager, didConnect p: CBPeripheral) {
        state = .connected
        UserDefaults.standard.set(p.identifier.uuidString, forKey: Self.knownPeripheralKey)
        p.discoverServices([Self.serviceUUID])
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect p: CBPeripheral, error: Error?) {
        notePairingError(error)
        connectOrScan()
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral p: CBPeripheral, error: Error?) {
        rx = nil
        inbound.removeAll()
        notePairingError(error)
        connectOrScan()
    }

    private func notePairingError(_ error: Error?) {
        if (error as? CBError)?.code == .peerRemovedPairingInformation {
            pairingLost = true
        }
    }
}

extension BridgeLink: CBPeripheralDelegate {
    func peripheral(_ p: CBPeripheral, didDiscoverServices error: Error?) {
        guard let service = p.services?.first(where: { $0.uuid == Self.serviceUUID }) else { return }
        p.discoverCharacteristics([Self.rxUUID, Self.txUUID], for: service)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        for c in service.characteristics ?? [] {
            if c.uuid == Self.rxUUID { rx = c }
            // Subscribing to the encrypted characteristic triggers iOS passkey pairing on first use.
            if c.uuid == Self.txUUID { p.setNotifyValue(true, for: c) }
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateNotificationStateFor c: CBCharacteristic, error: Error?) {
        if c.uuid == Self.txUUID, c.isNotifying, error == nil {
            state = .ready
            pairingLost = false
            onReady?()
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor c: CBCharacteristic, error: Error?) {
        if c.uuid == Self.txUUID, let value = c.value { handle(value) }
    }
}
