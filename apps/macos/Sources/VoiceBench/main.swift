// End-to-end latency on this Mac: committed fixture audio, fed at real time, through the
// on-device recognizer, the real session service, FoundationModels and the system voice,
// into the turn-fenced player rendered offline at real time. Prints one JSON object.
//
//   swift run -c release VoiceBench --rounds 3 [--cold] [--unpaced]
//
// Each round speaks all six fixtures, then runs an interrupt drill. Nothing is played on the
// speakers. Live device latency (output buffer, Bluetooth) is not included: see DRILLS.md.

import Darwin
import Foundation
import VoiceCore

let args = CommandLine.arguments

/// Resident memory of this process, in MB (for soak runs).
func residentMB() -> Double {
    var info = mach_task_basic_info()
    var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size / MemoryLayout<natural_t>.size)
    let kr = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) { task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count) }
    }
    return kr == KERN_SUCCESS ? Double(info.resident_size) / 1_048_576 : -1
}

/// Resident memory of another process, in MB, via ps.
func residentMB(pid: Int32) -> Double {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/bin/ps")
    p.arguments = ["-o", "rss=", "-p", "\(pid)"]
    let out = Pipe()
    p.standardOutput = out
    try? p.run()
    p.waitUntilExit()
    let text = String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    return (Double(text) ?? -1024) / 1024
}

/// `player-storm --cycles N`: N interrupt cycles through the turn-fenced player, offline.
/// Each cycle schedules a chunk for turn k, renders a little, stops with fence k+1, then
/// offers a late chunk of turn k, which must be refused. Times every stop().
if args.dropFirst().first == "player-storm" {
    let cycles = args.firstIndex(of: "--cycles").flatMap { Int(args[$0 + 1]) } ?? 100_000
    let format = Audio.monoFloat(22_050)
    let player = try TurnFencedPlayer(format: format, offline: true)
    let tone = Audio.tone(seconds: 0.05, format: format)
    var stops: [Double] = []
    stops.reserveCapacity(cycles)
    var accepted = 0
    let memBefore = residentMB()
    let clock = ContinuousClock()
    let t0 = clock.now
    var memSamples: [Double] = []
    for k in 1...cycles {
        // A pool per cycle, as a run loop would drain one per event: without it, objects
        // AVFoundation autoreleases pile up for the whole loop and look like a leak.
        try autoreleasepool {
            let turn = TurnID(UInt64(k))
            player.enqueue(AudioChunk(turn: turn, seq: 0, pcm: tone))
            _ = try player.render(frames: 256)
            let d = player.stop(fence: TurnID(UInt64(k + 1)))
            stops.append(Double(d.components.seconds) * 1e6 + Double(d.components.attoseconds) / 1e12)
            if player.enqueue(AudioChunk(turn: turn, seq: 1, pcm: tone)) != .droppedStale { accepted += 1 }
        }
        if k % (cycles / 10) == 0 { memSamples.append(residentMB()) }
    }
    let elapsed = clock.now - t0
    let silentAfter = Audio.peak(try player.render(frames: 22_050)) < 1e-4
    stops.sort()
    func p(_ q: Double) -> Double { stops[min(stops.count - 1, Int(Double(stops.count - 1) * q))] }
    let out: [String: Any] = [
        "scenario": "player-storm", "cycles": cycles, "elapsed_s": Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18,
        "late_chunks_accepted": accepted, "silent_after_storm": silentAfter,
        "stop_call_us": ["p50": p(0.5), "p99": p(0.99), "p999": p(0.999), "max": stops.last ?? 0],
        "resident_mb": ["before": memBefore, "after": residentMB(), "every_tenth": memSamples],
        "stats": ["scheduled": player.stats.scheduled, "dropped_stale": player.stats.droppedStale, "stops": player.stats.stops],
        "pass": accepted == 0 && silentAfter,
    ]
    print(String(decoding: try JSONSerialization.data(withJSONObject: out, options: [.prettyPrinted, .sortedKeys]), as: UTF8.self))
    exit(accepted == 0 && silentAfter ? 0 : 1)
}

func flag(_ name: String) -> Bool { args.contains(name) }
func value(_ name: String, _ fallback: Int) -> Int { args.firstIndex(of: name).flatMap { Int(args[$0 + 1]) } ?? fallback }

let repoRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let rounds = value("--rounds", 3)
let warm = !flag("--cold")
let paced = !flag("--unpaced")

func ms(_ d: Duration?) -> Double? { d.map { Double($0.components.seconds) * 1000 + Double($0.components.attoseconds) / 1e15 } }
func summary(_ values: [Double]) -> [String: Double] {
    let v = values.sorted()
    guard !v.isEmpty else { return [:] }
    func at(_ p: Double) -> Double { v[min(v.count - 1, Int((Double(v.count - 1) * p).rounded()))] }
    return ["n": Double(v.count), "p50_ms": at(0.5), "p95_ms": at(0.95), "max_ms": v.last!]
}
func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("voice-bench: \(message)\n".utf8))
    exit(2)
}

guard let binary = SessionBinary.locate(repoRoot: repoRoot) else { fail("voice-session not built (cargo build --release -p voice-session)") }
let model = FoundationModelsProvider()
if case .unavailable(_, let detail) = model.availability() { fail("FoundationModels unavailable: \(detail)") }
let recognizer = AppleSpeechRecognizer()
guard await recognizer.availability() == .ready else { fail("on-device speech assets not installed") }
let synthesizer = SystemVoiceSynthesizer()
if warm { await synthesizer.warmUp() }

let fixtures: [(String, String)] = try String(contentsOf: repoRoot.appending(path: "fixtures/audio/utterances.tsv"), encoding: .utf8)
    .split(separator: "\n").filter { !$0.hasPrefix("#") }
    .map { let p = $0.split(separator: "\t", maxSplits: 1).map(String.init); return (p[0], p[1]) }
let audio = try fixtures.map { try Audio.read(repoRoot.appending(path: "fixtures/audio/\($0.0).wav")) }

let dataDir = try DataDirectory.prepare(FileManager.default.temporaryDirectory.appending(path: "pva-bench-\(UUID().uuidString)"))
var config = ConversationConfig(binary: binary, dataDir: dataDir, recognizer: recognizer, model: model, synthesizer: synthesizer)
config.offlinePlayback = true
if !warm { config.model = ColdModel(model) }
let conversation = try Conversation(config)
let events = ConversationRecorder(conversation.events)
let pump = OfflineRenderPump(conversation.player, realTime: true)
try await conversation.start()
guard await events.next({ if case .ready = $0 { true } else { false } }) != nil else { fail("service not ready") }
await conversation.setCaptureConsent(true)
_ = await events.next { if case .consent = $0 { true } else { false } }

/// FoundationModels without the startup prewarm, for --cold runs.
struct ColdModel: LanguageModeling {
    let inner: FoundationModelsProvider
    init(_ inner: FoundationModelsProvider) { self.inner = inner }
    var modelID: String { inner.modelID }
    var instructionsVersion: String { inner.instructionsVersion }
    var isTestProvider: Bool { false }
    func availability() -> LMAvailability { inner.availability() }
    func respond(to heard: String, events: @escaping @Sendable (LMEvent) -> Void) async throws { try await inner.respond(to: heard, events: events) }
}

func speak(_ pcm: PCM) async {
    await conversation.pushToTalkDown()
    _ = await events.next { if case .capture(_, on: true) = $0 { true } else { false } }
    for slice in Audio.slices(pcm, frames: Int(pcm.buffer.format.sampleRate / 10)) {
        await conversation.feed(slice)
        if paced { try? await Task.sleep(for: .milliseconds(100)) }
    }
    await conversation.pushToTalkUp()
}

var turns: [[String: Any]] = []
var e2e: [Double] = [], stt: [Double] = [], lm: [Double] = [], tts: [Double] = []
var firstTurn: [String: Double]?
var outcomes: [String: Int] = [:]
var stopTails: [Double] = []
var memory: [[String: Double]] = [["round": 0, "bench_mb": residentMB(), "service_mb": residentMB(pid: await conversation.servicePID ?? 0)]]
var audioAfterStop = 0
let started = ContinuousClock.now

for round in 1...rounds {
    for (i, (name, _)) in fixtures.enumerated() {
        await speak(audio[i])
        guard case .turnClosed(_, let outcome, let timing)? = await events.next(timeout: .seconds(60), { if case .turnClosed = $0 { true } else { false } }) else {
            fail("turn did not close: round \(round) \(name)")
        }
        outcomes[outcome.rawValue, default: 0] += 1
        if outcome == .degraded, case .degraded(_, let stage, let reason)? = await events.recent({ if case .degraded = $0 { true } else { false } }) {
            outcomes["degraded_\(stage.rawValue)_\(reason.rawValue)", default: 0] += 1
        }
        let row: [String: Double] = [
            "end_to_first_audio_ms": ms(timing.endToFirstAudio) ?? -1, "stt_finalize_ms": ms(timing.recognizerFinalize) ?? -1,
            "lm_first_sentence_ms": ms(timing.modelFirstSentence) ?? -1, "tts_first_audio_ms": ms(timing.synthesisFirstAudio) ?? -1,
        ]
        turns.append(["round": round, "fixture": name, "outcome": outcome.rawValue, "timing": row])
        if firstTurn == nil { firstTurn = row } else if outcome == .completed {
            if let v = ms(timing.endToFirstAudio) { e2e.append(v) }
            if let v = ms(timing.recognizerFinalize) { stt.append(v) }
            if let v = ms(timing.modelFirstSentence) { lm.append(v) }
            if let v = ms(timing.synthesisFirstAudio) { tts.append(v) }
        }
    }
    // Interrupt drill: let the reply start, interrupt 300 ms in, then listen for 1 s.
    await speak(audio[fixtures.firstIndex { $0.0 == "story-request" } ?? 0])
    let audible = ContinuousClock.now + .seconds(30)
    let before = pump.loudBlocks.withLock { $0 }
    while pump.loudBlocks.withLock({ $0 }) == before && ContinuousClock.now < audible { try await Task.sleep(for: .milliseconds(5)) }
    try await Task.sleep(for: .milliseconds(300))
    let interruptAt = ContinuousClock.now
    await conversation.interrupt()
    _ = await events.next(timeout: .seconds(10)) { if case .turnClosed(_, .interrupted, _) = $0 { true } else { false } }
    try await Task.sleep(for: .seconds(1))
    if let last = pump.lastLoud.withLock({ $0 }) {
        let tail = ms(last - interruptAt) ?? 0
        stopTails.append(max(0, tail))
        if tail > 100 { audioAfterStop += 1 }
    }
    outcomes["interrupted_drill", default: 0] += 1
    memory.append(["round": Double(round), "bench_mb": residentMB(), "service_mb": residentMB(pid: await conversation.servicePID ?? 0)])
}

let result: [String: Any] = [
    "rounds": rounds, "warm": warm, "paced": paced, "elapsed_s": ms(ContinuousClock.now - started).map { $0 / 1000 } ?? 0,
    "model": model.modelID, "instructions": model.instructionsVersion,
    "first_turn_ms": firstTurn ?? [:],
    "steady_state": [
        "end_to_first_audio": summary(e2e), "stt_finalize": summary(stt),
        "lm_first_sentence": summary(lm), "tts_first_audio": summary(tts),
    ],
    "outcomes": outcomes,
    "interrupt_drill": [
        "runs": stopTails.count, "audio_more_than_100ms_after_interrupt": audioAfterStop,
        "last_audio_after_interrupt": summary(stopTails),
        "stale_chunks_refused_by_player": conversation.player.stats.droppedStale,
    ],
    "resident_memory": memory,
    "turns": turns,
]
pump.stop()
await conversation.shutdown()
let json = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
print(String(decoding: json, as: UTF8.self))
exit(audioAfterStop == 0 ? 0 : 1)
