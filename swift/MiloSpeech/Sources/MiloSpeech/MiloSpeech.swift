@preconcurrency import AVFoundation
import FluidAudio
import Foundation

public typealias MiloEventCallback = @convention(c) (UnsafeMutableRawPointer?, Int32, UnsafePointer<CChar>?, Int64) -> Void

private let eventPartial: Int32 = 0
private let eventEou: Int32 = 1
private let eventWarning: Int32 = 2
private let eventInfo: Int32 = 3
private let sampleRate = 16000
private let queueWarnSamples: UInt64 = 8000

struct MiloError: LocalizedError {
    let errorDescription: String?
    init(_ message: String) { errorDescription = message }
}

struct EventSink: @unchecked Sendable {
    let callback: MiloEventCallback
    let context: UnsafeMutableRawPointer?

    func emit(_ kind: Int32, _ text: String, _ ms: Int64 = -1) {
        text.withCString { callback(context, kind, $0, ms) }
    }
}

final class Session: @unchecked Sendable {
    private let asr: StreamingEouAsrManager
    private let vqe: LocalVqeStream?
    private let refiner: AsrManager?
    private let sink: EventSink
    private let format: AVAudioFormat
    private let lock = NSLock()
    private let done = DispatchSemaphore(value: 0)
    private let wake: AsyncStream<Void>
    private let wakeContinuation: AsyncStream<Void>.Continuation

    private var mic: [Float] = []
    private var reference: [Float] = []
    private var pendingRearm: Bool?
    private var pushed: UInt64 = 0
    private var consumed: UInt64 = 0
    private var overflowing = false
    private var stopped = false
    private var eouTokenCount = 0
    private var latched = false

    private init(asr: StreamingEouAsrManager, vqe: LocalVqeStream?, refiner: AsrManager?, sink: EventSink) {
        self.asr = asr
        self.vqe = vqe
        self.refiner = refiner
        self.sink = sink
        format = AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: Double(sampleRate), channels: 1, interleaved: false)!
        (wake, wakeContinuation) = AsyncStream.makeStream(of: Void.self, bufferingPolicy: .bufferingNewest(1))
    }

    static func load(
        chunkMs: UInt32, debounceMs: UInt32, aec: Bool, secondPass: Bool, sink: EventSink
    ) async throws -> Session {
        let chunk: StreamingChunkSize
        switch chunkMs {
        case 160: chunk = .ms160
        case 320: chunk = .ms320
        case 1280: chunk = .ms1280
        default: throw MiloError("chunk_ms must be 160, 320 or 1280, got \(chunkMs)")
        }
        let asr = StreamingEouAsrManager(chunkSize: chunk, eouDebounceMs: Int(debounceMs))
        async let vqe = loadVqe(aec)
        if secondPass {
            sink.emit(eventInfo, "loading the second-pass transcription model (downloads and compiles on first run)")
        }
        async let refiner = loadRefiner(secondPass)
        try await asr.loadModels(to: nil, configuration: nil, progressHandler: nil)
        let session = Session(asr: asr, vqe: try await vqe, refiner: try await refiner, sink: sink)
        try await session.warm()
        await session.installCallbacks()
        Task.detached { await session.run() }
        return session
    }

    private static func loadVqe(_ enabled: Bool) async throws -> LocalVqeStream? {
        guard enabled else { return nil }
        let manager = try await LocalVqeManager(
            config: LocalVqeConfig(variant: .v13, chunk: .realtime16ms, computeUnits: .cpuOnly))
        return try await manager.makeStream()
    }

    private static func loadRefiner(_ enabled: Bool) async throws -> AsrManager? {
        guard enabled else { return nil }
        let models = try await AsrModels.downloadAndLoad(version: .v2)
        let manager = AsrManager(config: .default)
        try await manager.loadModels(models)
        return manager
    }

    func transcribe(_ samples: [Float]) async -> String? {
        guard let refiner else { return nil }
        var audio = samples
        let minimum = sampleRate * 3 / 2
        if audio.count < minimum {
            audio.append(contentsOf: repeatElement(0, count: minimum - audio.count))
        }
        do {
            var state = TdtDecoderState.make()
            return try await refiner.transcribe(audio, decoderState: &state).text
        } catch {
            sink.emit(eventWarning, "second-pass transcription failed: \(error.localizedDescription)")
            return nil
        }
    }

    private func warm() async throws {
        let silence = [Float](repeating: 0, count: sampleRate)
        _ = await transcribe(silence)
        if let vqe {
            _ = try await vqe.enhance(mic: silence, reference: silence)
            try await vqe.reset()
        }
        if let buffer = makeBuffer(silence) {
            _ = try await asr.process(audioBuffer: buffer)
        }
        await asr.reset()
    }

    private func installCallbacks() async {
        let sink = self.sink
        await asr.setPartialCallback { [weak self, weak asr] text in
            guard let self, let asr else { return }
            let stamps = asr.assumeIsolated { $0.getTokenTimestampsMs() }
            if self.takeLatch() { self.rearm(keepTranscript: true) }
            sink.emit(eventPartial, text, Int64(stamps.last ?? -1))
        }
        await asr.setEouCallback { [weak self, weak asr] text in
            guard let self, let asr else { return }
            let stamps = asr.assumeIsolated { $0.getTokenTimestampsMs() }
            guard self.claimEou(tokenCount: stamps.count) else { return }
            sink.emit(eventEou, text, Int64(stamps.last ?? -1))
        }
    }

    private func claimEou(tokenCount: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard tokenCount > eouTokenCount else {
            latched = true
            return false
        }
        eouTokenCount = tokenCount
        return true
    }

    private func takeLatch() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let was = latched
        latched = false
        return was
    }

    func push(mic micPtr: UnsafePointer<Float>, count: Int, reference refPtr: UnsafePointer<Float>?, refCount: Int) {
        let usesReference = vqe != nil
        lock.lock()
        if stopped {
            lock.unlock()
            return
        }
        mic.append(contentsOf: UnsafeBufferPointer(start: micPtr, count: count))
        if usesReference {
            let n = refPtr == nil ? 0 : min(refCount, count)
            if let refPtr, n > 0 {
                reference.append(contentsOf: UnsafeBufferPointer(start: refPtr, count: n))
            }
            if n < count {
                reference.append(contentsOf: repeatElement(0, count: count - n))
            }
        }
        pushed += UInt64(count)
        lock.unlock()
        wakeContinuation.yield()
    }

    func rearm(keepTranscript: Bool) {
        lock.lock()
        pendingRearm = keepTranscript && (pendingRearm ?? true)
        latched = false
        lock.unlock()
        wakeContinuation.yield()
    }

    func shutdown() {
        lock.lock()
        stopped = true
        lock.unlock()
        wakeContinuation.finish()
        done.wait()
    }

    private struct Batch {
        var rearm: Bool?
        var mic: [Float]
        var reference: [Float]
        var queued: UInt64
    }

    private func take() -> Batch? {
        lock.lock()
        defer { lock.unlock() }
        if stopped || (pendingRearm == nil && mic.isEmpty) { return nil }
        let batch = Batch(rearm: pendingRearm, mic: mic, reference: reference, queued: pushed - consumed)
        pendingRearm = nil
        mic.removeAll(keepingCapacity: true)
        reference.removeAll(keepingCapacity: true)
        if batch.rearm == false { eouTokenCount = 0 }
        return batch
    }

    private func markConsumed(_ count: Int) {
        lock.lock()
        consumed += UInt64(count)
        lock.unlock()
    }

    private func isStopped() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return stopped
    }

    private func run() async {
        for await _ in wake {
            while let batch = take() {
                await handle(batch)
            }
            if isStopped() { break }
        }
        await asr.setPartialCallback { _ in }
        await asr.setEouCallback { _ in }
        done.signal()
    }

    private func handle(_ batch: Batch) async {
        if batch.queued > queueWarnSamples, !overflowing {
            overflowing = true
            sink.emit(eventWarning, "speech recognition is \(batch.queued * 1000 / UInt64(sampleRate)) ms behind")
        } else if overflowing, batch.queued < queueWarnSamples / 2 {
            overflowing = false
            sink.emit(eventInfo, "speech recognition caught up")
        }
        if let keep = batch.rearm {
            do {
                try await asr.rearm(keepTranscript: keep)
            } catch {
                sink.emit(eventWarning, "speech rearm failed: \(error.localizedDescription)")
            }
        }
        guard !batch.mic.isEmpty else { return }
        defer { markConsumed(batch.mic.count) }
        do {
            var audio = batch.mic
            if let vqe {
                audio = try await vqe.enhance(mic: batch.mic, reference: batch.reference)
            }
            guard !audio.isEmpty, let buffer = makeBuffer(audio) else { return }
            _ = try await asr.process(audioBuffer: buffer)
        } catch {
            sink.emit(eventWarning, "speech processing failed: \(error.localizedDescription)")
        }
    }

    private func makeBuffer(_ samples: [Float]) -> AVAudioPCMBuffer? {
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)),
            let channel = buffer.floatChannelData?[0]
        else { return nil }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { channel.update(from: $0.baseAddress!, count: samples.count) }
        return buffer
    }
}

private final class StartResult: @unchecked Sendable {
    var value: Result<Session, Error>?
}

@_cdecl("milo_speech_start")
public func milo_speech_start(
    _ chunkMs: UInt32,
    _ eouDebounceMs: UInt32,
    _ aec: Bool,
    _ secondPass: Bool,
    _ callback: MiloEventCallback,
    _ context: UnsafeMutableRawPointer?,
    _ errorOut: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>?
) -> UnsafeMutableRawPointer? {
    let sink = EventSink(callback: callback, context: context)
    let result = StartResult()
    let ready = DispatchSemaphore(value: 0)
    Task.detached {
        do {
            result.value = .success(
                try await Session.load(
                    chunkMs: chunkMs, debounceMs: eouDebounceMs, aec: aec, secondPass: secondPass, sink: sink))
        } catch {
            result.value = .failure(error)
        }
        ready.signal()
    }
    ready.wait()
    switch result.value {
    case .success(let session):
        return Unmanaged.passRetained(session).toOpaque()
    case .failure(let failure):
        errorOut?.pointee = strdup(failure.localizedDescription)
        return nil
    case .none:
        errorOut?.pointee = strdup("milo_speech_start: no result")
        return nil
    }
}

@_cdecl("milo_speech_push")
public func milo_speech_push(
    _ handle: UnsafeMutableRawPointer,
    _ mic: UnsafePointer<Float>?,
    _ micCount: Int,
    _ reference: UnsafePointer<Float>?,
    _ refCount: Int
) {
    guard let mic, micCount > 0 else { return }
    Unmanaged<Session>.fromOpaque(handle).takeUnretainedValue()
        .push(mic: mic, count: micCount, reference: reference, refCount: refCount)
}

private final class TextResult: @unchecked Sendable {
    var value: String?
}

@_cdecl("milo_speech_transcribe")
public func milo_speech_transcribe(
    _ handle: UnsafeMutableRawPointer,
    _ samples: UnsafePointer<Float>?,
    _ count: Int
) -> UnsafeMutablePointer<CChar>? {
    guard let samples, count > 0 else { return nil }
    let session = Unmanaged<Session>.fromOpaque(handle).takeUnretainedValue()
    let audio = Array(UnsafeBufferPointer(start: samples, count: count))
    let result = TextResult()
    let ready = DispatchSemaphore(value: 0)
    Task.detached {
        result.value = await session.transcribe(audio)
        ready.signal()
    }
    ready.wait()
    return result.value.flatMap { strdup($0) }
}

@_cdecl("milo_speech_rearm")
public func milo_speech_rearm(_ handle: UnsafeMutableRawPointer, _ keepTranscript: Bool) {
    Unmanaged<Session>.fromOpaque(handle).takeUnretainedValue().rearm(keepTranscript: keepTranscript)
}

@_cdecl("milo_speech_destroy")
public func milo_speech_destroy(_ handle: UnsafeMutableRawPointer) {
    Unmanaged<Session>.fromOpaque(handle).takeRetainedValue().shutdown()
}

@_cdecl("milo_speech_free_string")
public func milo_speech_free_string(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}
