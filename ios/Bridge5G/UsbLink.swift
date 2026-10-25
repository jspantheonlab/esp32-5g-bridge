import Foundation
import Network
import Observation

/// Control link to the Raspberry Pi USB-C dongle: the same newline-delimited JSON protocol as
/// the BLE link, over TCP on the wired USB network (the dongle appears as an Ethernet adapter).
@Observable
final class UsbLink {
    static let host: NWEndpoint.Host = "192.168.7.1"
    static let port: NWEndpoint.Port = 5005

    private(set) var isReady = false

    @ObservationIgnored var onMessage: (([String: Any]) -> Void)?
    @ObservationIgnored var onReadyChange: ((Bool) -> Void)?

    @ObservationIgnored private var connection: NWConnection?
    @ObservationIgnored private var inbound = Data()
    @ObservationIgnored private let queue = DispatchQueue(label: "bridge5g.usb")

    init() {
        connect()
    }

    func send(_ message: [String: Any]) {
        guard isReady, var data = try? JSONSerialization.data(withJSONObject: message) else { return }
        data.append(0x0A)
        connection?.send(content: data, completion: .contentProcessed { _ in })
    }

    private func connect() {
        let params = NWParameters.tcp
        // Only use the USB-C cable, never Wi-Fi or cellular, for this address.
        params.requiredInterfaceType = .wiredEthernet
        let conn = NWConnection(host: Self.host, port: Self.port, using: params)
        connection = conn
        conn.stateUpdateHandler = { [weak self] state in
            DispatchQueue.main.async { self?.handle(state, of: conn) }
        }
        conn.start(queue: queue)
    }

    private func handle(_ state: NWConnection.State, of conn: NWConnection) {
        guard conn === connection else { return }
        switch state {
        case .ready:
            setReady(true)
            receive(on: conn)
        case .failed, .cancelled, .waiting:
            setReady(false)
            conn.cancel()
            connection = nil
            inbound.removeAll()
            // No dongle (or it was unplugged): try again shortly.
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                if self?.connection == nil { self?.connect() }
            }
        default:
            break
        }
    }

    private func setReady(_ ready: Bool) {
        guard ready != isReady else { return }
        isReady = ready
        onReadyChange?(ready)
    }

    private func receive(on conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 64 * 1024) { [weak self] data, _, isComplete, error in
            DispatchQueue.main.async {
                guard let self, conn === self.connection else { return }
                if let data { self.handle(data) }
                if isComplete || error != nil {
                    self.handle(.failed(error ?? NWError.posix(.ECONNRESET)), of: conn)
                } else {
                    self.receive(on: conn)
                }
            }
        }
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
