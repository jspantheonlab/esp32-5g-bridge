import SwiftUI

struct DialerView: View {
    @Environment(AppModel.self) private var model
    @State private var number = ""

    private let keys: [[String]] = [
        ["1", "2", "3"],
        ["4", "5", "6"],
        ["7", "8", "9"],
        ["*", "0", "#"],
    ]

    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Text(number.isEmpty ? " " : number)
                .font(.system(size: 36, weight: .light, design: .rounded))
                .lineLimit(1)
                .minimumScaleFactor(0.5)
                .padding(.horizontal)

            if model.calls.state != .idle {
                Text("\(model.calls.state.rawValue.capitalized) · \(model.calls.number)")
                    .foregroundStyle(.secondary)
            }

            VStack(spacing: 16) {
                ForEach(keys, id: \.self) { row in
                    HStack(spacing: 24) {
                        ForEach(row, id: \.self) { key in
                            Button { number.append(key) } label: {
                                Text(key)
                                    .font(.system(size: 32, weight: .regular, design: .rounded))
                                    .frame(width: 76, height: 76)
                                    .background(Circle().fill(Color(.secondarySystemBackground)))
                            }
                            .buttonStyle(.plain)
                            .simultaneousGesture(LongPressGesture().onEnded { _ in
                                if key == "0" { number.append("+") }
                            })
                        }
                    }
                }
            }

            HStack(spacing: 24) {
                Color.clear.frame(width: 76, height: 76)
                callButton
                Button { if !number.isEmpty { number.removeLast() } } label: {
                    Image(systemName: "delete.left").font(.title2).frame(width: 76, height: 76)
                }
                .buttonStyle(.plain)
                .opacity(number.isEmpty ? 0 : 1)
            }
            Spacer()
        }
        .disabled(!model.isConnected)
    }

    @ViewBuilder private var callButton: some View {
        if model.calls.state == .idle {
            Button { model.calls.startCall(to: number) } label: {
                Image(systemName: "phone.fill")
                    .font(.title)
                    .foregroundStyle(.white)
                    .frame(width: 76, height: 76)
                    .background(Circle().fill(.green))
            }
            .disabled(number.isEmpty)
        } else {
            Button { model.calls.endCall() } label: {
                Image(systemName: "phone.down.fill")
                    .font(.title)
                    .foregroundStyle(.white)
                    .frame(width: 76, height: 76)
                    .background(Circle().fill(.red))
            }
        }
    }
}
