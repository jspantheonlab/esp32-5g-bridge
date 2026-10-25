import SwiftUI
import UIKit

/// Edit the bridge hotspot's network name and password over the encrypted BLE link.
struct WifiSettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var ssid = ""
    @State private var pass = ""
    @State private var showPassword = false
    @State private var edited = false
    @State private var confirming = false

    private var draft: WifiCredentials { WifiCredentials(ssid: ssid, pass: pass) }
    private var problem: String? { WifiCredentials.problem(ssid: ssid, pass: pass) }
    private var canSave: Bool {
        model.link.state == .ready && problem == nil && draft != model.wifi && model.wifiSaveState != .saving
    }

    var body: some View {
        Form {
            Section {
                TextField("Network name", text: $ssid)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                HStack {
                    Group {
                        if showPassword {
                            TextField("Password", text: $pass)
                        } else {
                            SecureField("Password", text: $pass)
                        }
                    }
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    Button { showPassword.toggle() } label: {
                        Image(systemName: showPassword ? "eye.slash" : "eye")
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel(showPassword ? "Hide password" : "Show password")
                }
            } header: {
                Text("Hotspot")
            } footer: {
                if edited, let problem {
                    Text(problem).foregroundStyle(.red)
                } else {
                    Text("8–63 characters. Devices on the hotspot, including this iPhone, disconnect when you save and need the new password to rejoin.")
                }
            }

            Section {
                Button("Copy Password") { UIPasteboard.general.string = pass }
                    .disabled(pass.isEmpty)
            }

            switch model.wifiSaveState {
            case .saving:
                Section { HStack { ProgressView(); Text("Applying…") } }
            case .saved:
                Section {
                    Label("Saved. Rejoin “\(model.wifi?.ssid ?? ssid)” in Settings → Wi-Fi.", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                }
            case .failed(let message):
                Section { Label(message, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red) }
            case .idle:
                EmptyView()
            }
        }
        .navigationTitle("Hotspot")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            Button("Save") { confirming = true }.disabled(!canSave)
        }
        .confirmationDialog("Change hotspot settings?", isPresented: $confirming, titleVisibility: .visible) {
            Button("Change and Disconnect") { model.saveWifi(draft) }
        } message: {
            Text("Everything on the hotspot drops off. If you kept the same network name, iOS may ask you to forget the network before it accepts the new password.")
        }
        .onAppear {
            if let current = model.wifi {
                ssid = current.ssid
                pass = current.pass
            }
            model.requestWifi()
        }
        .onChange(of: model.wifi) { _, current in
            // Fill the form from the bridge unless the user has started editing.
            if let current, !edited || model.wifiSaveState == .saved {
                ssid = current.ssid
                pass = current.pass
                edited = false
            }
        }
        .onChange(of: ssid) { _, new in if new != model.wifi?.ssid { edited = true } }
        .onChange(of: pass) { _, new in if new != model.wifi?.pass { edited = true } }
    }
}
