import SwiftUI

struct MessagesView: View {
    @Environment(AppModel.self) private var model
    @State private var composingTo: String?
    @State private var showNew = false
    @State private var newNumber = ""

    var body: some View {
        NavigationStack {
            List {
                ForEach(model.store.conversations) { convo in
                    NavigationLink(value: convo.peer) {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Text(convo.peer).font(.headline)
                                Spacer()
                                if let date = convo.last?.date {
                                    Text(date, style: .relative).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            Text(convo.last?.body ?? "")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                    }
                }
                .onDelete { offsets in
                    let convos = model.store.conversations
                    offsets.forEach { model.store.delete(peer: convos[$0].peer) }
                }
            }
            .overlay {
                if model.store.messages.isEmpty {
                    ContentUnavailableView("No Messages", systemImage: "message",
                                           description: Text("SMS received by the bridge appear here."))
                }
            }
            .navigationTitle("Messages")
            .navigationDestination(for: String.self) { ConversationView(peer: $0) }
            .toolbar {
                Button { showNew = true } label: { Image(systemName: "square.and.pencil") }
            }
            .alert("New Message", isPresented: $showNew) {
                TextField("Phone number", text: $newNumber).keyboardType(.phonePad)
                Button("Cancel", role: .cancel) { newNumber = "" }
                Button("Next") {
                    composingTo = newNumber
                    newNumber = ""
                }
            }
            .navigationDestination(item: $composingTo) { ConversationView(peer: $0) }
        }
    }
}

struct ConversationView: View {
    @Environment(AppModel.self) private var model
    let peer: String
    @State private var draft = ""

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 8) {
                        ForEach(model.store.messages(with: peer)) { msg in
                            Bubble(message: msg).id(msg.id)
                        }
                    }
                    .padding()
                }
                .onChange(of: model.store.messages.count) {
                    if let last = model.store.messages(with: peer).last {
                        withAnimation { proxy.scrollTo(last.id, anchor: .bottom) }
                    }
                }
            }

            HStack(alignment: .bottom) {
                TextField("Text message", text: $draft, axis: .vertical)
                    .textFieldStyle(.roundedBorder)
                    .lineLimit(1...5)
                Button {
                    model.sendSMS(to: peer, body: draft)
                    draft = ""
                } label: {
                    Image(systemName: "arrow.up.circle.fill").font(.title)
                }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.link.state != .ready)
            }
            .padding()
            .background(.bar)
        }
        .navigationTitle(peer)
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct Bubble: View {
    let message: SMSMessage

    var body: some View {
        HStack {
            if message.outgoing { Spacer(minLength: 48) }
            VStack(alignment: message.outgoing ? .trailing : .leading, spacing: 2) {
                Text(message.body)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(message.outgoing ? Color.green : Color(.secondarySystemBackground))
                    .foregroundStyle(message.outgoing ? .white : .primary)
                    .clipShape(RoundedRectangle(cornerRadius: 18))
                if message.outgoing && message.status != .sent {
                    Text(message.status == .failed ? (message.error ?? "Not delivered") : "Sending…")
                        .font(.caption2)
                        .foregroundStyle(message.status == .failed ? .red : .secondary)
                }
            }
            if !message.outgoing { Spacer(minLength: 48) }
        }
    }
}
