import Foundation
import Synchronization
import Testing
@testable import VoiceCore

@Suite("The session process boundary", .serialized, .enabled("voice-session is built (cargo build -p voice-session)") { haveBinary })
struct SessionProcessTests {
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
}
