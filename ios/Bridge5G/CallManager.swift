import AVFoundation
import CallKit
import Network
import Observation

/// Presents bridge calls through CallKit (native incoming-call screen, lock screen, Recents)
/// and maps CallKit actions to bridge commands.
@Observable
final class CallManager: NSObject {
    private(set) var state: CallState = .idle
    private(set) var number = ""

    @ObservationIgnored var send: (([String: Any]) -> Void)?

    @ObservationIgnored private let provider: CXProvider
    @ObservationIgnored private let controller = CXCallController()
    @ObservationIgnored private let audio = VoiceAudio()
    @ObservationIgnored private var callID: UUID?
    @ObservationIgnored private var outgoing = false

    override init() {
        let config = CXProviderConfiguration()
        config.supportsVideo = false
        config.maximumCallGroups = 1
        config.maximumCallsPerCallGroup = 1
        config.supportedHandleTypes = [.phoneNumber]
        config.includesCallsInRecents = true
        provider = CXProvider(configuration: config)
        super.init()
        provider.setDelegate(self, queue: .main)
    }

    /// Where call audio is streamed: the Wi-Fi bridge or the USB-C dongle.
    func setAudioHost(_ host: String) {
        audio.host = NWEndpoint.Host(host)
    }

    // MARK: - Bridge -> CallKit

    func handle(_ msg: [String: Any]) {
        guard let raw = msg["state"] as? String, let newState = CallState(rawValue: raw) else { return }
        let num = msg["num"] as? String ?? ""
        state = newState
        if !num.isEmpty { number = num }

        switch newState {
        case .incoming:
            guard callID == nil else { return }
            let id = UUID()
            callID = id
            outgoing = false
            let update = CXCallUpdate()
            update.remoteHandle = CXHandle(type: .phoneNumber, value: num.isEmpty ? "Unknown" : num)
            update.hasVideo = false
            configureAudioSession()
            provider.reportNewIncomingCall(with: id, update: update) { [weak self] error in
                if error != nil {
                    // e.g. Do Not Disturb or blocked number: decline on the modem too
                    self?.callID = nil
                    self?.send?(["t": "hangup"])
                }
            }
        case .alerting:
            if let callID, outgoing { provider.reportOutgoingCall(with: callID, startedConnectingAt: Date()) }
        case .active:
            if let callID, outgoing { provider.reportOutgoingCall(with: callID, connectedAt: Date()) }
        case .idle:
            if let callID {
                provider.reportCall(with: callID, endedAt: Date(), reason: .remoteEnded)
                finish()
            }
        case .dialing, .held, .waiting:
            break
        }
    }

    // MARK: - UI -> CallKit

    func startCall(to number: String) {
        guard callID == nil else { return }
        let id = UUID()
        let action = CXStartCallAction(call: id, handle: CXHandle(type: .phoneNumber, value: number))
        controller.request(CXTransaction(action: action)) { error in
            if let error { print("Start call failed: \(error)") }
        }
    }

    func endCall() {
        guard let callID else { return }
        controller.request(CXTransaction(action: CXEndCallAction(call: callID))) { _ in }
    }

    private func configureAudioSession() {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .allowBluetoothA2DP])
    }

    private func finish() {
        callID = nil
        outgoing = false
        audio.stop()
    }
}

extension CallManager: CXProviderDelegate {
    func providerDidReset(_ provider: CXProvider) {
        finish()
    }

    func provider(_ provider: CXProvider, perform action: CXStartCallAction) {
        callID = action.callUUID
        outgoing = true
        number = action.handle.value
        configureAudioSession()
        send?(["t": "dial", "num": action.handle.value])
        provider.reportOutgoingCall(with: action.callUUID, startedConnectingAt: Date())
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXAnswerCallAction) {
        send?(["t": "answer"])
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXEndCallAction) {
        send?(["t": "hangup"])
        finish()
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXSetMutedCallAction) {
        audio.muted = action.isMuted
        action.fulfill()
    }

    func provider(_ provider: CXProvider, perform action: CXPlayDTMFCallAction) {
        for digit in action.digits {
            send?(["t": "dtmf", "d": String(digit)])
        }
        action.fulfill()
    }

    func provider(_ provider: CXProvider, didActivate audioSession: AVAudioSession) {
        audio.start()
    }

    func provider(_ provider: CXProvider, didDeactivate audioSession: AVAudioSession) {
        audio.stop()
    }
}
