import Foundation

enum CallState: String {
    case idle, incoming, dialing, alerting, active, held, waiting
}

struct ModemStatus: Equatable {
    var simReady = false
    var registered = false
    var dataUp = false
    var operatorName = ""
    var rat = "—"
    var rssi: Int?
    var ip = ""
    var ssid = ""
    /// nil on boards without a battery gauge.
    var battery: Int?
    var charging: Bool?
    var usbPower: Bool?

    init() {}

    init(_ msg: [String: Any]) {
        simReady = msg["sim"] as? Bool ?? false
        registered = msg["reg"] as? Bool ?? false
        dataUp = msg["data"] as? Bool ?? false
        operatorName = msg["op"] as? String ?? ""
        rat = msg["rat"] as? String ?? "—"
        rssi = msg["rssi"] as? Int
        ip = msg["ip"] as? String ?? ""
        ssid = msg["ssid"] as? String ?? ""
        battery = msg["bat"] as? Int
        charging = msg["chg"] as? Bool
        usbPower = msg["usb"] as? Bool
    }

    var batterySymbol: String {
        guard let battery else { return "battery.0percent" }
        let level = battery >= 88 ? 100 : battery >= 63 ? 75 : battery >= 38 ? 50 : battery >= 13 ? 25 : 0
        return "battery.\(level)percent"
    }

    /// 0...4 bars from RSSI in dBm.
    var bars: Int {
        guard let rssi else { return 0 }
        switch rssi {
        case ..<(-105): return 1
        case ..<(-95): return 2
        case ..<(-85): return 3
        default: return 4
        }
    }
}

struct SMSMessage: Codable, Identifiable, Hashable {
    enum Status: String, Codable { case received, sending, sent, failed }

    var id = UUID()
    var peer: String
    var body: String
    var date: Date
    var outgoing: Bool
    var status: Status
    var error: String?
}

struct Conversation: Identifiable {
    var peer: String
    var messages: [SMSMessage]
    var id: String { peer }
    var last: SMSMessage? { messages.last }
}

struct WifiCredentials: Equatable {
    var ssid: String
    var pass: String

    /// Same rules as the firmware (bridge-core/src/wifi.rs). Returns nil when valid.
    static func problem(ssid: String, pass: String) -> String? {
        if ssid.isEmpty { return "Enter a network name." }
        if ssid.utf8.count > 32 { return "Network name is longer than 32 bytes." }
        if !(8...63).contains(pass.utf8.count) { return "Password must be 8 to 63 characters." }
        if !pass.unicodeScalars.allSatisfy({ (0x20...0x7E).contains($0.value) }) {
            return "Password may only use letters, digits, spaces and standard symbols (no emoji or accents)."
        }
        return nil
    }
}

enum WifiSaveState: Equatable {
    case idle, saving, saved, failed(String)
}
