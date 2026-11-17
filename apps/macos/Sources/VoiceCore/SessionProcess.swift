// The session service as the app's child process (D-015): JSON lines over its stdin and
// stdout, no socket. The child exits when its stdin closes, so it cannot outlive the app.

import Foundation
import Synchronization

public enum SessionEvent: Sendable {
    case message(ServiceMessage)
    /// A line the app could not decode. Its content is not kept.
    case undecodable
    /// The child exited (crash or clean end), with its exit status.
    case exited(Int32)
}

public enum SessionProcessError: Error, Equatable, Sendable {
    case binaryMissing(String)
    case notRunning
    case launchFailed
}

public final class SessionProcess: @unchecked Sendable {
    public static let maxLine = 64 * 1024

    public let events: AsyncStream<SessionEvent>
    private let continuation: AsyncStream<SessionEvent>.Continuation
    private let process = Process()
    private let stdin = Pipe()
    private let stdout = Pipe()
    private let stderr = Pipe()
    private let writes = DispatchQueue(label: "voice.session.stdin")
    private let alive = Flag()

    /// A shareable atomic flag (Atomic itself is non-copyable and cannot be captured).
    final class Flag: Sendable {
        private let value = Atomic<Bool>(false)
        func set(_ v: Bool) { value.store(v, ordering: .sequentiallyConsistent) }
        var isSet: Bool { value.load(ordering: .sequentiallyConsistent) }
    }

    /// Starts `binary --data-dir dataDir [--limit k=v]...`.
    public init(binary: URL, dataDir: URL, limits: [String: String] = [:], eventBuffer: Int = 8_192) throws {
        guard FileManager.default.isExecutableFile(atPath: binary.path) else { throw SessionProcessError.binaryMissing(binary.path) }
        // Service messages are never dropped silently: the stream is large and bounded, and an
        // app that falls this far behind is treated like a dead session.
        (events, continuation) = AsyncStream.makeStream(of: SessionEvent.self, bufferingPolicy: .bufferingOldest(eventBuffer))
        process.executableURL = binary
        process.arguments = ["--data-dir", dataDir.path] + limits.sorted { $0.key < $1.key }.flatMap { ["--limit", "\($0.key)=\($0.value)"] }
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr
        let continuation = self.continuation
        let alive = self.alive
        process.terminationHandler = { p in
            alive.set(false)
            continuation.yield(.exited(p.terminationStatus))
            continuation.finish()
        }
        // A write to a child that just died must fail with EPIPE, not kill the app with
        // SIGPIPE: the exit is only noticed later, by the termination handler.
        _ = fcntl(stdin.fileHandleForWriting.fileDescriptor, F_SETNOSIGPIPE, 1)
        do { try process.run() } catch { throw SessionProcessError.launchFailed }
        alive.set(true)
        startReading(pid: process.processIdentifier)
        // Drain stderr so the child's log writer never blocks; its lines are content-free (D-011).
        stderr.fileHandleForReading.readabilityHandler = { handle in _ = handle.availableData }
    }

    public var pid: Int32 { process.processIdentifier }
    public var isRunning: Bool { alive.isSet }
    /// Set when the event buffer overflowed: the session was ended rather than thinned out.
    public var overflowed: Bool { overflow.isSet }
    private let overflow = Flag()

    private func startReading(pid: Int32) {
        let handle = stdout.fileHandleForReading
        let continuation = self.continuation
        let overflow = self.overflow
        Thread.detachNewThread {
            var pending = Data()
            var discarding = false
            while true {
                let chunk = handle.availableData
                if chunk.isEmpty { break }
                pending.append(chunk)
                while let newline = pending.firstIndex(of: 0x0A) {
                    let line = pending[pending.startIndex..<newline]
                    pending.removeSubrange(pending.startIndex...newline)
                    if discarding {
                        discarding = false
                        continue
                    }
                    if let message = try? LineCodec.decodeService(Data(line)) {
                        if case .dropped = continuation.yield(.message(message)), !overflow.isSet {
                            // The app fell this far behind: a session that has lost messages
                            // cannot be trusted. End it; the stream finishes on its exit.
                            overflow.set(true)
                            Foundation.kill(pid, SIGKILL)
                        }
                    } else {
                        continuation.yield(.undecodable)
                    }
                }
                if pending.count > Self.maxLine {
                    pending.removeAll(keepingCapacity: true)
                    discarding = true
                    continuation.yield(.undecodable)
                }
            }
        }
    }

    /// Sends one message. Writes are serialized; a dead child refuses them.
    public func send(_ message: ClientMessage) throws {
        guard isRunning else { throw SessionProcessError.notRunning }
        let line = try LineCodec.encode(message)
        let handle = stdin.fileHandleForWriting
        let alive = self.alive
        writes.async {
            do {
                try handle.write(contentsOf: line)
            } catch {
                // EPIPE (or any write error): the service is down. The termination handler
                // reports the exit; from now on sends are refused.
                alive.set(false)
            }
        }
    }

    /// Closes stdin: the service records `session_closed` and exits.
    public func close() {
        writes.sync { try? stdin.fileHandleForWriting.close() }
    }

    /// For crash drills only: SIGKILL, no shutdown path.
    public func kill() {
        if isRunning { Foundation.kill(process.processIdentifier, SIGKILL) }
    }

    public func waitUntilExit() { process.waitUntilExit() }
}

/// Where the app finds `voice-session`: next to its own executable (a bundled app), then
/// `PVA_SESSION_BINARY`, then the repository's cargo output (development).
public enum SessionBinary {
    public static func locate(repoRoot: URL? = nil) -> URL? {
        var candidates: [URL] = []
        if let exe = Bundle.main.executableURL { candidates.append(exe.deletingLastPathComponent().appending(path: "voice-session")) }
        if let env = ProcessInfo.processInfo.environment["PVA_SESSION_BINARY"] { candidates.append(URL(fileURLWithPath: env)) }
        if let root = repoRoot {
            candidates.append(root.appending(path: "target/release/voice-session"))
            candidates.append(root.appending(path: "target/debug/voice-session"))
        }
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }
}

/// The app's private data directory: created 0700 and excluded from Time Machine (D-018).
public enum DataDirectory {
    public static func prepare(_ url: URL) throws -> URL {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        var u = url
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try u.setResourceValues(values)
        return u
    }

    public static var standard: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appending(path: "PersonalVoiceAgent")
    }
}
