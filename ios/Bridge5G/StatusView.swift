import SwiftUI
import UIKit

struct StatusView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        NavigationStack {
            List {
                if model.link.pairingLost {
                    Section {
                        Label("The bridge was reset and no longer recognises this iPhone.", systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.orange)
                        Text("Open Settings → Bluetooth, tap ⓘ next to 5G-Bridge, choose Forget This Device, then come back here and enter the passkey when asked.")
                            .font(.footnote)
                        Button("Open Settings") {
                            if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
                        }
                    }
                }

                if let notice = model.notice {
                    Section {
                        Label(notice, systemImage: "info.circle")
                    }
                }

                Section("Bluetooth link") {
                    LabeledContent("State", value: model.link.state.rawValue.capitalized)
                    if model.link.state == .scanning {
                        Text("Looking for your 5G Bridge… When prompted, enter the passkey printed on the device.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }

                if let battery = model.status.battery {
                    Section("Battery") {
                        LabeledContent("Level") {
                            HStack(spacing: 6) {
                                Image(systemName: model.status.batterySymbol)
                                if model.status.charging == true { Image(systemName: "bolt.fill") }
                                Text("\(battery)%")
                            }
                        }
                        if let usb = model.status.usbPower {
                            LabeledContent("Power", value: model.status.charging == true ? "Charging"
                                           : usb ? "USB (charged)" : "Battery")
                        }
                    }
                }

                Section("Cellular") {
                    LabeledContent("SIM", value: model.status.simReady ? "Ready" : "Not ready")
                    LabeledContent("Operator", value: model.status.operatorName.isEmpty ? "—" : model.status.operatorName)
                    LabeledContent("Network", value: model.status.rat)
                    LabeledContent("Signal") {
                        HStack(spacing: 6) {
                            Image(systemName: "cellularbars", variableValue: Double(model.status.bars) / 4)
                            if let rssi = model.status.rssi { Text("\(rssi) dBm") }
                        }
                    }
                }

                Section {
                    LabeledContent("Data", value: model.status.dataUp ? "Connected" : "Down")
                    if !model.status.ip.isEmpty {
                        LabeledContent("WAN IP", value: model.status.ip)
                    }
                    LabeledContent("Wi-Fi", value: model.status.ssid.isEmpty ? "—" : model.status.ssid)
                    NavigationLink("Network name & password") { WifiSettingsView() }
                        .disabled(model.link.state != .ready)
                } header: {
                    Text("Hotspot")
                } footer: {
                    Text("Join this Wi-Fi network in Settings to use the bridge's cellular data. Call audio also travels over it.")
                }

                if let error = model.lastError {
                    Section("Last error") { Text(error).foregroundStyle(.red) }
                }

                Section {
                    Button("Refresh") { model.link.send(["t": "status"]) }
                    Button("Forget device", role: .destructive) { model.link.forgetDevice() }
                }
            }
            .navigationTitle("5G Bridge")
        }
    }
}
