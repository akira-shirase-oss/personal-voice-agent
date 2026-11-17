import Foundation
import Testing
@testable import VoiceCore

/// The repository root, found from this file's path (Tests/VoiceCoreTests/ → ../../../..).
let repoRoot: URL = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent()
    .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()

func goldenLines(_ name: String) throws -> [Data] {
    let url = repoRoot.appending(path: "fixtures/protocol/\(name)")
    return try String(contentsOf: url, encoding: .utf8)
        .split(separator: "\n").filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        .map { Data($0.utf8) }
}

func jsonObject(_ data: Data) throws -> NSDictionary {
    try #require(try JSONSerialization.jsonObject(with: data) as? NSDictionary)
}

@Suite("Protocol golden lines")
struct ProtocolTests {
    @Test("every client line decodes and re-encodes to the same JSON")
    func clientLines() throws {
        var types = Set<String>()
        for line in try goldenLines("client.jsonl") {
            let message = try JSONDecoder().decode(ClientMessage.self, from: line)
            let again = try JSONEncoder().encode(message)
            #expect(try jsonObject(again) == jsonObject(line), "\(String(decoding: line, as: UTF8.self))")
            types.insert(message.type)
        }
        #expect(types.count == 19)
    }

    @Test("every service line decodes and re-encodes to the same JSON")
    func serviceLines() throws {
        var types = Set<String>()
        for line in try goldenLines("service.jsonl") {
            let message = try JSONDecoder().decode(ServiceMessage.self, from: line)
            let again = try JSONEncoder().encode(message)
            #expect(try jsonObject(again) == jsonObject(line), "\(String(decoding: line, as: UTF8.self))")
            types.insert(message.type)
        }
        #expect(types.count == 17)
    }

    @Test("an unknown failure code from a provider becomes error")
    func unknownReason() throws {
        let line = Data(#"{"type":"degraded","turn":1,"stage":"lm","reason":"something new"}"#.utf8)
        #expect(try LineCodec.decodeService(line) == .degraded(turn: TurnID(1), stage: .lm, reason: .error))
    }
}
