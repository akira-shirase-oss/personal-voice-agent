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
    /// Accepting writes: false once the child exited or a write to it failed.
    private let alive = Flag()
    /// Set only by the termination handler: the child is really gone.
    private let ended = Flag()

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
        let ended = self.ended
        process.terminationHandler = { p in
            alive.set(false)
            ended.set(true)
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
    /// The child has exited (a failed write alone does not mean that).
    public var hasExited: Bool { ended.isSet }
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

    /// SIGKILL, no shutdown path: crash drills, a hung service, the last step of `stop`.
    public func kill() {
        if !hasExited { Foundation.kill(process.processIdentifier, SIGKILL) }
    }

    public func waitUntilExit() { process.waitUntilExit() }

    /// Waits at most `timeout` for the child to exit; true if it did.
    public func waitForExit(_ timeout: Duration) async -> Bool {
        let deadline = ContinuousClock.now + timeout
        while !hasExited && ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
        return hasExited
    }

    /// Ends the child within a bound, whatever state it is in: stdin is closed (the service
    /// closes its session and exits), then SIGTERM after `grace`, then SIGKILL after `term`
    /// (a stopped process ignores everything but SIGKILL). True once it has exited.
    @discardableResult
    public func stop(grace: Duration = .seconds(1), term: Duration = .milliseconds(500)) async -> Bool {
        close()
        if await waitForExit(grace) { return true }
        if !hasExited { Foundation.kill(process.processIdentifier, SIGTERM) }
        if await waitForExit(term) { return true }
        kill()
        return await waitForExit(.seconds(1))
    }
}

/// The data directory's lock (one service per directory). Its holder writes its process ID
/// into it, so the app can tell a stale orphan of its own — the same `voice-session` binary,
/// left behind when an earlier app process died (its parent is now launchd) — from another copy
/// of the app that is still running and must be left alone.
public enum ServiceLock {
    public static func holder(dataDir: URL) -> Int32? {
        guard let text = try? String(contentsOf: dataDir.appending(path: "lock"), encoding: .utf8),
            let pid = Int32(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 0
        else { return nil }
        return pid
    }

    /// Whether `pid` is alive, runs exactly `binary`, and has lost its parent app.
    public static func isStaleOrphan(_ pid: Int32, binary: URL) -> Bool {
        guard pid > 0, pid != getpid(), Foundation.kill(pid, 0) == 0 else { return false }
        var info = proc_bsdinfo()
        let size = Int32(MemoryLayout<proc_bsdinfo>.size)
        guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size, info.pbi_ppid == 1 else { return false }
        var path = [UInt8](repeating: 0, count: 4 * Int(MAXPATHLEN))
        let n = path.withUnsafeMutableBytes { proc_pidpath(pid, $0.baseAddress, UInt32($0.count)) }
        guard n > 0 else { return false }
        return realPath(String(decoding: path.prefix(Int(n)), as: UTF8.self)) == realPath(binary.path)
    }

    /// SIGTERM, a bounded wait, then SIGKILL. True once the process is gone.
    public static func terminate(_ pid: Int32, grace: Duration = .seconds(1)) async -> Bool {
        func gone() -> Bool { Foundation.kill(pid, 0) != 0 && errno == ESRCH }
        func wait(_ d: Duration) async -> Bool {
            let deadline = ContinuousClock.now + d
            while !gone() && ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(10)) }
            return gone()
        }
        Foundation.kill(pid, SIGTERM)
        if await wait(grace) { return true }
        Foundation.kill(pid, SIGKILL)
        return await wait(.seconds(1))
    }

    private static func realPath(_ path: String) -> String? {
        guard let resolved = realpath(path, nil) else { return nil }
        defer { free(resolved) }
        return String(cString: resolved)
    }
}

/// The service's capture-consent token (D-017): `consent.capture.granted` in the data directory.
/// A starting service counts capture consent only while the token exists with its owner's read
/// and write bits. The app removes it itself before it sends a revocation, so a service that is
/// hung or dies before reading the revocation is replaced by one that starts revoked.
public enum ConsentToken {
    /// Unlinks the token (no free space needed), or sets it to 0000 if it cannot be unlinked.
    /// True when the token no longer grants.
    @discardableResult
    public static func revoke(dataDir: URL) -> Bool {
        let path = dataDir.appending(path: "consent.capture.granted").path
        if unlink(path) == 0 || errno == ENOENT {
            let dir = open(dataDir.path, O_RDONLY)
            if dir >= 0 {
                _ = fsync(dir)
                close(dir)
            }
            return true
        }
        return chmod(path, 0) == 0
    }
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
