import Foundation
import Observation

/// Local SMS history (the bridge deletes messages from the SIM once the app acknowledges them).
@Observable
final class MessageStore {
    private(set) var messages: [SMSMessage] = []

    @ObservationIgnored private let fileURL = URL.documentsDirectory.appending(path: "messages.json")

    init() {
        if let data = try? Data(contentsOf: fileURL),
           let saved = try? JSONDecoder().decode([SMSMessage].self, from: data) {
            messages = saved
        }
    }

    var conversations: [Conversation] {
        Dictionary(grouping: messages, by: \.peer)
            .map { Conversation(peer: $0.key, messages: $0.value.sorted { $0.date < $1.date }) }
            .sorted { ($0.last?.date ?? .distantPast) > ($1.last?.date ?? .distantPast) }
    }

    func messages(with peer: String) -> [SMSMessage] {
        messages.filter { $0.peer == peer }.sorted { $0.date < $1.date }
    }

    func add(_ message: SMSMessage) {
        messages.append(message)
        save()
    }

    func update(id: UUID, status: SMSMessage.Status, error: String? = nil) {
        guard let i = messages.firstIndex(where: { $0.id == id }) else { return }
        messages[i].status = status
        messages[i].error = error
        save()
    }

    func delete(peer: String) {
        messages.removeAll { $0.peer == peer }
        save()
    }

    private func save() {
        if let data = try? JSONEncoder().encode(messages) {
            try? data.write(to: fileURL, options: [.atomic, .completeFileProtection])
        }
    }
}
