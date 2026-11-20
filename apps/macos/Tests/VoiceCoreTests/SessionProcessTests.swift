import Foundation
import Synchronization
import Testing
@testable import VoiceCore

@Suite("The session process boundary", .serialized, .removesScratch)
struct SessionProcessTests {
    @Test("the session service binary is built") func binaryPresent() { #expect(haveBinary, binaryMissing) }

    /// A reader that falls so far behind that the bounded event buffer fills: the session is
    /// treated as dead (killed, stream ended, overflow reported), never silently thinned out.
    @Test("an overflowing event buffer ends the session instead of dropping messages")
    func overflowEndsTheSession() async throws {
        let p = try SessionProcess(binary: sessionBinary, dataDir: try tempDataDir("overflow"), eventBuffer: 4)
        try p.send(.hello(protocol: protocolVersion, client: "overflow-test"))
        for seq in 1...200 { try? p.send(.ping(seq: UInt64(seq))) }
        try await Task.sleep(for: .milliseconds(500))  // nobody reads: the buffer overflows
        let ended = Task { () -> Int in
            var n = 0
            for await _ in p.events { n += 1 }
            return n
        }
        let timeout = Task {
            try await Task.sleep(for: .seconds(5))
            ended.cancel()
        }
        let received = await ended.value
        timeout.cancel()
        #expect(received < 201, "not every message could be kept")
        #expect(p.overflowed, "the overflow is reported")
        #expect(!p.isRunning, "the session that lost messages was ended")
    }

    /// Writing to a service that just died must not kill the app with SIGPIPE: the write
    /// fails, the service is reported down, and the app lives on. 50 crashes, each while a
    /// writer is sending every half millisecond.
    @Test("50 crashes while sending: the app survives every one and sees each exit")
    func survivesWritesToADeadService() async throws {
        for run in 1...50 {
            let p = try SessionProcess(binary: sessionBinary, dataDir: try tempDataDir("sigpipe"))
            try p.send(.hello(protocol: protocolVersion, client: "sigpipe-test"))
            let exited = Task { () -> Bool in
                for await e in p.events { if case .exited = e { return true } }
                return false
            }
            let writer = Task.detached {
                var failures = 0
                while !Task.isCancelled {
                    do { try p.send(.interrupt) } catch { failures += 1 }
                    try? await Task.sleep(for: .microseconds(500))
                }
                return failures
            }
            try await Task.sleep(for: .milliseconds(Int.random(in: 5...30)))
            p.kill()
            try await Task.sleep(for: .milliseconds(100))  // keep sending into the dead pipe
            writer.cancel()
            _ = await writer.value
            #expect(await exited.value, "run \(run): the exit was reported")
            #expect(!p.isRunning, "run \(run): the process is known to be down")
        }
    }

    /// `createDirectory`'s attributes apply only to a directory it creates; one that
    /// already existed keeps its mode. A loose data directory is tightened at prepare —
    /// a stricter one is never loosened, and owner bits are never added.
    @Test("an existing loose data directory is tightened at prepare")
    func tightensAnExistingLooseDirectory() throws {
        let fm = FileManager.default
        func mode(_ url: URL) throws -> Int {
            try #require(fm.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int) & 0o7777
        }
        let loose = TestScratch.url("dirmode")
        try fm.createDirectory(at: loose, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o755])
        _ = try DataDirectory.prepare(loose)
        #expect(try mode(loose) == 0o700, "group/other access is removed")
        // 0555 has group/other access: those bits go, owner-write is never added.
        try fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: loose.path)
        _ = try? DataDirectory.prepare(loose)
        #expect(try mode(loose) == 0o500, "only group/other bits are stripped, never granted")
        // A stricter directory is left untouched.
        let strict = TestScratch.url("dirmode-strict")
        try fm.createDirectory(at: strict, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o500])
        _ = try? DataDirectory.prepare(strict)
        #expect(try mode(strict) == 0o500, "a stricter mode is never loosened")
    }
}
