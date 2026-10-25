import AVFoundation
import Network

/// Streams call audio to/from the ESP32 over UDP on the bridge's Wi-Fi.
/// Wire format (matches firmware/main/audio_bridge.h):
///   8-byte header {'5','G', seq u16 LE, ts u32 LE} + 20 ms of mono int16 LE PCM.
final class VoiceAudio {
    static let bridgePort: NWEndpoint.Port = 5004

    /// 192.168.4.1 for the Wi-Fi bridge (ESP32), 192.168.7.1 for the USB-C dongle.
    var host: NWEndpoint.Host = "192.168.4.1"

    private let sampleRate: Double
    private let frameSamples: Int
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let netFormat: AVAudioFormat
    private let queue = DispatchQueue(label: "bridge5g.audio")

    private var connection: NWConnection?
    private var converter: AVAudioConverter?
    private var pending: [Int16] = []
    private var seq: UInt16 = 0
    private var timestamp: UInt32 = 0
    private var queuedBuffers = 0
    private var running = false

    var muted = false

    init(sampleRate: Double = 8000) {
        self.sampleRate = sampleRate
        frameSamples = Int(sampleRate / 50)
        netFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: sampleRate,
                                  channels: 1, interleaved: false)!
        engine.attach(player)
    }

    /// Call from CXProviderDelegate.provider(_:didActivate:).
    func start() {
        guard !running else { return }
        running = true

        let conn = NWConnection(host: host, port: Self.bridgePort, using: .udp)
        conn.start(queue: queue)
        connection = conn
        receive(on: conn)
        sendPacket(samples: [])   // lets the bridge learn our address before audio flows

        let input = engine.inputNode
        try? input.setVoiceProcessingEnabled(true)   // echo cancellation for speakerphone
        let inFormat = input.outputFormat(forBus: 0)
        converter = AVAudioConverter(from: inFormat, to: netFormat)
        engine.connect(player, to: engine.mainMixerNode, format: netFormat)
        input.installTap(onBus: 0, bufferSize: 1024, format: inFormat) { [weak self] buffer, _ in
            self?.capture(buffer)
        }

        do {
            try engine.start()
            player.play()
        } catch {
            print("Audio engine failed: \(error)")
        }
    }

    /// Call from provider(_:didDeactivate:) or when the call ends.
    func stop() {
        guard running else { return }
        running = false
        engine.inputNode.removeTap(onBus: 0)
        player.stop()
        engine.stop()
        connection?.cancel()
        connection = nil
        queue.async { [weak self] in
            self?.pending.removeAll()
            self?.queuedBuffers = 0
        }
    }

    // MARK: - Uplink (mic -> bridge)

    private func capture(_ buffer: AVAudioPCMBuffer) {
        guard let converter else { return }
        let ratio = netFormat.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 16
        guard let out = AVAudioPCMBuffer(pcmFormat: netFormat, frameCapacity: capacity) else { return }

        var consumed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if consumed {
                status.pointee = .noDataNow
                return nil
            }
            consumed = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let ch = out.floatChannelData?[0] else { return }

        let muted = self.muted
        let samples = (0..<Int(out.frameLength)).map { i -> Int16 in
            muted ? 0 : Int16(max(-1, min(1, ch[i])) * Float(Int16.max))
        }
        queue.async { [weak self] in
            guard let self else { return }
            pending.append(contentsOf: samples)
            while pending.count >= frameSamples {
                sendPacket(samples: Array(pending.prefix(frameSamples)))
                pending.removeFirst(frameSamples)
            }
        }
    }

    private func sendPacket(samples: [Int16]) {
        var data = Data(capacity: 8 + samples.count * 2)
        data.append(contentsOf: [UInt8(ascii: "5"), UInt8(ascii: "G")])
        withUnsafeBytes(of: seq.littleEndian) { data.append(contentsOf: $0) }
        withUnsafeBytes(of: timestamp.littleEndian) { data.append(contentsOf: $0) }
        samples.withUnsafeBufferPointer { ptr in
            for s in ptr { withUnsafeBytes(of: s.littleEndian) { data.append(contentsOf: $0) } }
        }
        seq &+= 1
        timestamp &+= UInt32(samples.count)
        connection?.send(content: data, completion: .idempotent)
    }

    // MARK: - Downlink (bridge -> speaker)

    private func receive(on conn: NWConnection) {
        conn.receiveMessage { [weak self] data, _, _, error in
            guard let self, error == nil else { return }
            if let data, data.count == 8 + frameSamples * 2,
               data[data.startIndex] == UInt8(ascii: "5"), data[data.startIndex + 1] == UInt8(ascii: "G") {
                play(data.dropFirst(8))
            }
            if running { receive(on: conn) }
        }
    }

    private func play(_ payload: Data) {
        // Drop audio if playback falls behind (clock drift / burst) to keep latency bounded.
        guard queuedBuffers < 8,
              let buffer = AVAudioPCMBuffer(pcmFormat: netFormat, frameCapacity: AVAudioFrameCount(frameSamples)),
              let ch = buffer.floatChannelData?[0] else { return }
        buffer.frameLength = AVAudioFrameCount(frameSamples)
        payload.withUnsafeBytes { raw in
            for i in 0..<frameSamples {
                let s = Int16(littleEndian: raw.loadUnaligned(fromByteOffset: i * 2, as: Int16.self))
                ch[i] = Float(s) / Float(Int16.max)
            }
        }
        queuedBuffers += 1
        player.scheduleBuffer(buffer) { [weak self] in
            self?.queue.async { self?.queuedBuffers -= 1 }
        }
    }
}
