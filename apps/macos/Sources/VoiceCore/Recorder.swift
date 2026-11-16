// Collects a conversation's events so a caller (tests, the bench, drills) can wait for one.

import Foundation

public actor ConversationRecorder {
    public private(set) var seen: [ConversationEvent] = []
    private var cursor = 0

    public init(_ events: AsyncStream<ConversationEvent>) {
        Task { for await e in events { await self.add(e) } }
    }

    private func add(_ e: ConversationEvent) { seen.append(e) }

    /// The most recent event already seen that satisfies `match`.
    public func recent(_ match: @Sendable (ConversationEvent) -> Bool) -> ConversationEvent? {
        seen.last(where: match)
    }

    /// The next event after the previous match that satisfies `match`, or nil at the timeout.
    public func next(timeout: Duration = .seconds(10), _ match: @Sendable (ConversationEvent) -> Bool) async -> ConversationEvent? {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            while cursor < seen.count {
                let e = seen[cursor]
                cursor += 1
                if match(e) { return e }
            }
            try? await Task.sleep(for: .milliseconds(2))
        }
        return nil
    }
}
