import Foundation
import Observation
import UIKit
import UserNotifications

/// Wires the BLE link, CallKit and the message store together.
@Observable
final class AppModel {
    let link = BridgeLink()
    let calls = CallManager()
    let store = MessageStore()
    private(set) var status = ModemStatus()
    private(set) var lastError: String?

    init() {
        calls.send = { [weak self] in self?.link.send($0) }
        link.onMessage = { [weak self] in self?.handle($0) }
        link.onReady = { [weak self] in self?.link.send(["t": "status"]) }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
    }

    func sendSMS(to peer: String, body: String) {
        let msg = SMSMessage(peer: peer, body: body, date: .now, outgoing: true, status: .sending)
        store.add(msg)
        link.send(["t": "sms_send", "ref": msg.id.uuidString, "to": peer, "body": body])
    }

    private func handle(_ msg: [String: Any]) {
        switch msg["t"] as? String {
        case "status":
            let previous = status
            status = ModemStatus(msg)
            warnIfBatteryLow(previous: previous)
        case "call":
            calls.handle(msg)
        case "sms":
            receiveSMS(msg)
        case "sms_sent":
            if let ref = msg["ref"] as? String, let id = UUID(uuidString: ref) {
                let ok = msg["ok"] as? Bool ?? false
                store.update(id: id, status: ok ? .sent : .failed, error: msg["err"] as? String)
            }
        case "err":
            lastError = "\(msg["ctx"] as? String ?? ""): \(msg["msg"] as? String ?? "")"
        default:
            break
        }
    }

    private func receiveSMS(_ msg: [String: Any]) {
        guard let simIndex = msg["id"] as? Int,
              let from = msg["from"] as? String,
              let body = msg["body"] as? String else { return }
        let date = Self.parseTimestamp(msg["ts"] as? String) ?? .now

        // Messages stay on the SIM until acknowledged, so a re-sync can deliver duplicates.
        let isDuplicate = store.messages.contains {
            !$0.outgoing && $0.peer == from && $0.body == body && abs($0.date.timeIntervalSince(date)) < 1
        }
        if !isDuplicate {
            store.add(SMSMessage(peer: from, body: body, date: date, outgoing: false, status: .received))
            notify(from: from, body: body)
        }
        link.send(["t": "sms_ack", "id": simIndex])
    }

    /// Notifies once when the bridge battery drops to 15% or below while not charging.
    private func warnIfBatteryLow(previous: ModemStatus) {
        guard let level = status.battery, level <= 15, status.charging != true,
              (previous.battery ?? 100) > 15 else { return }
        let content = UNMutableNotificationContent()
        content.title = "5G Bridge battery low"
        content.body = "\(level)% remaining. Connect USB-C to keep calls and data working."
        content.sound = .default
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: "battery-low", content: content, trigger: nil))
    }

    private func notify(from: String, body: String) {
        guard UIApplication.shared.applicationState != .active else { return }
        let content = UNMutableNotificationContent()
        content.title = from
        content.body = body
        content.sound = .default
        UNUserNotificationCenter.current().add(
            UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }

    /// SCTS format "yy/MM/dd,HH:mm:ss±zz" where zz is the offset in quarter hours.
    static func parseTimestamp(_ ts: String?) -> Date? {
        guard let ts, ts.count >= 20 else { return nil }
        let base = String(ts.prefix(17))
        let signIndex = ts.index(ts.startIndex, offsetBy: 17)
        guard let quarters = Int(ts[ts.index(after: signIndex)...]) else { return nil }
        let offset = (ts[signIndex] == "-" ? -1 : 1) * quarters * 15 * 60
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yy/MM/dd,HH:mm:ss"
        f.timeZone = TimeZone(secondsFromGMT: offset)
        return f.date(from: base)
    }
}
