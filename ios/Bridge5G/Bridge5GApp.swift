import SwiftUI

@main
struct Bridge5GApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(model)
        }
    }
}

struct ContentView: View {
    var body: some View {
        TabView {
            StatusView()
                .tabItem { Label("Bridge", systemImage: "antenna.radiowaves.left.and.right") }
            DialerView()
                .tabItem { Label("Keypad", systemImage: "circle.grid.3x3.fill") }
            MessagesView()
                .tabItem { Label("Messages", systemImage: "message.fill") }
        }
    }
}
