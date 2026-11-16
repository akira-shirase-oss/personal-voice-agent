// The app ↔ service protocol (docs/PROTOCOL.md, v1), mirrored from voice-domain's
// `protocol` module. Both sides decode the golden lines in fixtures/protocol/.

import Foundation

public let protocolVersion = 1

public struct TurnID: Codable, Hashable, Comparable, Sendable, CustomStringConvertible {
    public let raw: UInt64
    public init(_ raw: UInt64) { self.raw = raw }
    public init(from decoder: Decoder) throws { raw = try decoder.singleValueContainer().decode(UInt64.self) }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(raw)
    }
    public static func < (a: TurnID, b: TurnID) -> Bool { a.raw < b.raw }
    public var description: String { "\(raw)" }
}

public struct CallID: Codable, Hashable, Comparable, Sendable, CustomStringConvertible {
    public let raw: UInt64
    public init(_ raw: UInt64) { self.raw = raw }
    public init(from decoder: Decoder) throws { raw = try decoder.singleValueContainer().decode(UInt64.self) }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(raw)
    }
    public static func < (a: CallID, b: CallID) -> Bool { a.raw < b.raw }
    public var description: String { "\(raw)" }
}

public enum ProposalKind: String, Codable, Sendable { case reply, action }

public enum SlotKind: String, Codable, Sendable { case amount, datetime, person, recipient, text }

public struct Slot: Codable, Equatable, Sendable {
    public var name: String
    public var kind: SlotKind
    public var value: String
    public init(name: String, kind: SlotKind, value: String) {
        self.name = name
        self.kind = kind
        self.value = value
    }
}

public enum Proposal: Codable, Equatable, Sendable {
    case reply(String)
    case action(reply: String, tool: String, slots: [Slot])

    enum Keys: String, CodingKey { case kind, reply, tool, slots }

    public var kind: ProposalKind {
        switch self {
        case .reply: .reply
        case .action: .action
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(ProposalKind.self, forKey: .kind) {
        case .reply: self = .reply(try c.decode(String.self, forKey: .reply))
        case .action:
            self = .action(
                reply: try c.decode(String.self, forKey: .reply),
                tool: try c.decode(String.self, forKey: .tool),
                slots: try c.decode([Slot].self, forKey: .slots))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        try c.encode(kind, forKey: .kind)
        switch self {
        case .reply(let reply): try c.encode(reply, forKey: .reply)
        case .action(let reply, let tool, let slots):
            try c.encode(reply, forKey: .reply)
            try c.encode(tool, forKey: .tool)
            try c.encode(slots, forKey: .slots)
        }
    }
}

/// Why a provider failed. The service maps any code it does not know to `error`.
public enum FailureReason: String, Codable, Sendable {
    case deadline, unavailable, error, cancelled, guardrail, protocolError = "protocol"
    case modelNotReady = "model_not_ready"
    case assetsMissing = "assets_missing"
    case contextWindow = "context_window"
    case audioFormat = "audio_format"
    case tooLong = "too_long"
    case ledgerFull = "ledger_full"
    case ledgerIO = "ledger_io"

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = FailureReason(rawValue: raw) ?? .error
    }
}

public enum Stage: String, Codable, Sendable { case stt, lm, tts, tool, ledger }

public enum Outcome: String, Codable, Sendable {
    case completed, interrupted, degraded, refused, rejected, expired, empty, abandoned
    case consentRevoked = "consent_revoked"
}

public enum ConsentScope: String, Codable, Sendable {
    case capture
    case personalVoice = "personal_voice"
}

public enum ConfirmReason: String, Codable, Sendable {
    case irreversibleEffect = "irreversible_effect"
    case sensitiveSlot = "sensitive_slot"
    case lowConfidence = "low_confidence"
    case noConfidence = "no_confidence"
    case slotNotHeard = "slot_not_heard"
}

public enum Voice: String, Codable, Sendable { case standard }

public struct ConsentState: Codable, Equatable, Sendable {
    public var capture: Bool
    public var personalVoice: Bool
    enum CodingKeys: String, CodingKey {
        case capture
        case personalVoice = "personal_voice"
    }
    public init(capture: Bool, personalVoice: Bool) {
        self.capture = capture
        self.personalVoice = personalVoice
    }
}

public struct Recovered: Codable, Equatable, Sendable {
    public var sessions: UInt32
    public var abandonedTurns: UInt32
    public var uncertainCalls: UInt32
    public var tornTails: UInt32
    enum CodingKeys: String, CodingKey {
        case sessions
        case abandonedTurns = "abandoned_turns"
        case uncertainCalls = "uncertain_calls"
        case tornTails = "torn_tails"
    }
}

/// Every field name used by either message direction.
enum Field: String, CodingKey {
    case type, `protocol`, client, turn, call, text, kind, index, proposal, model, instructions, reason, digest
    case approve, scope, grant, session, consent, recovered, on, heard, seq, voice, fence, tool, slots, reasons
    case outcome, stage, code, detail, capture
    case minConfidence = "min_confidence"
    case rightsReview = "rights_review"
    case deadlineMs = "deadline_ms"
    case personalVoice = "personal_voice"
}

// MARK: - App → service

public enum ClientMessage: Codable, Equatable, Sendable {
    case hello(protocol: Int, client: String)
    case pttDown
    case pttUp(turn: TurnID)
    case interrupt
    case sttFinal(call: CallID, text: String, minConfidence: Double?)
    case lmKind(call: CallID, kind: ProposalKind)
    case lmSentence(call: CallID, index: UInt32, text: String)
    case lmDone(call: CallID, proposal: Proposal, model: String, instructions: String)
    case ttsStarted(call: CallID)
    case ttsFinished(call: CallID)
    case sttFailed(call: CallID, reason: FailureReason)
    case lmFailed(call: CallID, reason: FailureReason)
    case ttsFailed(call: CallID, reason: FailureReason)
    case confirm(turn: TurnID, digest: String, approve: Bool)
    case consent(scope: ConsentScope, grant: Bool, rightsReview: String?)
    case flush
    case shutdown

    public var type: String {
        switch self {
        case .hello: "hello"
        case .pttDown: "ptt_down"
        case .pttUp: "ptt_up"
        case .interrupt: "interrupt"
        case .sttFinal: "stt_final"
        case .lmKind: "lm_kind"
        case .lmSentence: "lm_sentence"
        case .lmDone: "lm_done"
        case .ttsStarted: "tts_started"
        case .ttsFinished: "tts_finished"
        case .sttFailed: "stt_failed"
        case .lmFailed: "lm_failed"
        case .ttsFailed: "tts_failed"
        case .confirm: "confirm"
        case .consent: "consent"
        case .flush: "flush"
        case .shutdown: "shutdown"
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        try c.encode(type, forKey: .type)
        switch self {
        case .hello(let p, let client):
            try c.encode(p, forKey: .protocol)
            try c.encode(client, forKey: .client)
        case .pttDown, .interrupt, .flush, .shutdown: break
        case .pttUp(let turn): try c.encode(turn, forKey: .turn)
        case .sttFinal(let call, let text, let confidence):
            try c.encode(call, forKey: .call)
            try c.encode(text, forKey: .text)
            try c.encode(confidence, forKey: .minConfidence)
        case .lmKind(let call, let kind):
            try c.encode(call, forKey: .call)
            try c.encode(kind, forKey: .kind)
        case .lmSentence(let call, let index, let text):
            try c.encode(call, forKey: .call)
            try c.encode(index, forKey: .index)
            try c.encode(text, forKey: .text)
        case .lmDone(let call, let proposal, let model, let instructions):
            try c.encode(call, forKey: .call)
            try c.encode(proposal, forKey: .proposal)
            try c.encode(model, forKey: .model)
            try c.encode(instructions, forKey: .instructions)
        case .ttsStarted(let call), .ttsFinished(let call): try c.encode(call, forKey: .call)
        case .sttFailed(let call, let reason), .lmFailed(let call, let reason), .ttsFailed(let call, let reason):
            try c.encode(call, forKey: .call)
            try c.encode(reason, forKey: .reason)
        case .confirm(let turn, let digest, let approve):
            try c.encode(turn, forKey: .turn)
            try c.encode(digest, forKey: .digest)
            try c.encode(approve, forKey: .approve)
        case .consent(let scope, let grant, let review):
            try c.encode(scope, forKey: .scope)
            try c.encode(grant, forKey: .grant)
            try c.encode(review, forKey: .rightsReview)
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let type = try c.decode(String.self, forKey: .type)
        func call() throws -> CallID { try c.decode(CallID.self, forKey: .call) }
        switch type {
        case "hello": self = .hello(protocol: try c.decode(Int.self, forKey: .protocol), client: try c.decode(String.self, forKey: .client))
        case "ptt_down": self = .pttDown
        case "ptt_up": self = .pttUp(turn: try c.decode(TurnID.self, forKey: .turn))
        case "interrupt": self = .interrupt
        case "stt_final":
            self = .sttFinal(
                call: try call(), text: try c.decode(String.self, forKey: .text),
                minConfidence: try c.decodeIfPresent(Double.self, forKey: .minConfidence))
        case "lm_kind": self = .lmKind(call: try call(), kind: try c.decode(ProposalKind.self, forKey: .kind))
        case "lm_sentence":
            self = .lmSentence(call: try call(), index: try c.decode(UInt32.self, forKey: .index), text: try c.decode(String.self, forKey: .text))
        case "lm_done":
            self = .lmDone(
                call: try call(), proposal: try c.decode(Proposal.self, forKey: .proposal),
                model: try c.decode(String.self, forKey: .model), instructions: try c.decode(String.self, forKey: .instructions))
        case "tts_started": self = .ttsStarted(call: try call())
        case "tts_finished": self = .ttsFinished(call: try call())
        case "stt_failed": self = .sttFailed(call: try call(), reason: try c.decode(FailureReason.self, forKey: .reason))
        case "lm_failed": self = .lmFailed(call: try call(), reason: try c.decode(FailureReason.self, forKey: .reason))
        case "tts_failed": self = .ttsFailed(call: try call(), reason: try c.decode(FailureReason.self, forKey: .reason))
        case "confirm":
            self = .confirm(
                turn: try c.decode(TurnID.self, forKey: .turn), digest: try c.decode(String.self, forKey: .digest),
                approve: try c.decode(Bool.self, forKey: .approve))
        case "consent":
            self = .consent(
                scope: try c.decode(ConsentScope.self, forKey: .scope), grant: try c.decode(Bool.self, forKey: .grant),
                rightsReview: try c.decodeIfPresent(String.self, forKey: .rightsReview))
        case "flush": self = .flush
        case "shutdown": self = .shutdown
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: c, debugDescription: "unknown client message type")
        }
    }
}

// MARK: - Service → app

public enum ServiceMessage: Codable, Equatable, Sendable {
    case ready(protocol: Int, session: String, consent: ConsentState, recovered: Recovered)
    case turnOpened(turn: TurnID)
    case capture(turn: TurnID, on: Bool)
    case sttRequest(call: CallID, turn: TurnID, deadlineMs: UInt64)
    case lmRequest(call: CallID, turn: TurnID, deadlineMs: UInt64, heard: String)
    case ttsRequest(call: CallID, turn: TurnID, seq: UInt32, text: String, voice: Voice, deadlineMs: UInt64)
    case cancel(call: CallID)
    case stopPlayback(fence: TurnID)
    case confirmRequest(turn: TurnID, digest: String, heard: String, tool: String, slots: [Slot], reasons: [ConfirmReason])
    case turnClosed(turn: TurnID, outcome: Outcome)
    case degraded(turn: TurnID?, stage: Stage, reason: FailureReason)
    case consentState(ConsentState)
    case notice(code: String, turn: TurnID?)
    case flushed(seq: UInt64)
    case error(code: String, detail: String)

    public var type: String {
        switch self {
        case .ready: "ready"
        case .turnOpened: "turn_opened"
        case .capture: "capture"
        case .sttRequest: "stt_request"
        case .lmRequest: "lm_request"
        case .ttsRequest: "tts_request"
        case .cancel: "cancel"
        case .stopPlayback: "stop_playback"
        case .confirmRequest: "confirm_request"
        case .turnClosed: "turn_closed"
        case .degraded: "degraded"
        case .consentState: "consent_state"
        case .notice: "notice"
        case .flushed: "flushed"
        case .error: "error"
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Field.self)
        let type = try c.decode(String.self, forKey: .type)
        func turn() throws -> TurnID { try c.decode(TurnID.self, forKey: .turn) }
        func call() throws -> CallID { try c.decode(CallID.self, forKey: .call) }
        func deadline() throws -> UInt64 { try c.decode(UInt64.self, forKey: .deadlineMs) }
        switch type {
        case "ready":
            self = .ready(
                protocol: try c.decode(Int.self, forKey: .protocol), session: try c.decode(String.self, forKey: .session),
                consent: try c.decode(ConsentState.self, forKey: .consent), recovered: try c.decode(Recovered.self, forKey: .recovered))
        case "turn_opened": self = .turnOpened(turn: try turn())
        case "capture": self = .capture(turn: try turn(), on: try c.decode(Bool.self, forKey: .on))
        case "stt_request": self = .sttRequest(call: try call(), turn: try turn(), deadlineMs: try deadline())
        case "lm_request":
            self = .lmRequest(call: try call(), turn: try turn(), deadlineMs: try deadline(), heard: try c.decode(String.self, forKey: .heard))
        case "tts_request":
            self = .ttsRequest(
                call: try call(), turn: try turn(), seq: try c.decode(UInt32.self, forKey: .seq),
                text: try c.decode(String.self, forKey: .text), voice: try c.decode(Voice.self, forKey: .voice), deadlineMs: try deadline())
        case "cancel": self = .cancel(call: try call())
        case "stop_playback": self = .stopPlayback(fence: try c.decode(TurnID.self, forKey: .fence))
        case "confirm_request":
            self = .confirmRequest(
                turn: try turn(), digest: try c.decode(String.self, forKey: .digest), heard: try c.decode(String.self, forKey: .heard),
                tool: try c.decode(String.self, forKey: .tool), slots: try c.decode([Slot].self, forKey: .slots),
                reasons: try c.decode([ConfirmReason].self, forKey: .reasons))
        case "turn_closed": self = .turnClosed(turn: try turn(), outcome: try c.decode(Outcome.self, forKey: .outcome))
        case "degraded":
            self = .degraded(
                turn: try c.decodeIfPresent(TurnID.self, forKey: .turn), stage: try c.decode(Stage.self, forKey: .stage),
                reason: try c.decode(FailureReason.self, forKey: .reason))
        case "consent_state":
            self = .consentState(
                ConsentState(capture: try c.decode(Bool.self, forKey: .capture), personalVoice: try c.decode(Bool.self, forKey: .personalVoice)))
        case "notice": self = .notice(code: try c.decode(String.self, forKey: .code), turn: try c.decodeIfPresent(TurnID.self, forKey: .turn))
        case "flushed": self = .flushed(seq: try c.decode(UInt64.self, forKey: .seq))
        case "error": self = .error(code: try c.decode(String.self, forKey: .code), detail: try c.decode(String.self, forKey: .detail))
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: c, debugDescription: "unknown service message type")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Field.self)
        try c.encode(type, forKey: .type)
        switch self {
        case .ready(let p, let session, let consent, let recovered):
            try c.encode(p, forKey: .protocol)
            try c.encode(session, forKey: .session)
            try c.encode(consent, forKey: .consent)
            try c.encode(recovered, forKey: .recovered)
        case .turnOpened(let turn): try c.encode(turn, forKey: .turn)
        case .capture(let turn, let on):
            try c.encode(turn, forKey: .turn)
            try c.encode(on, forKey: .on)
        case .sttRequest(let call, let turn, let deadline):
            try c.encode(call, forKey: .call)
            try c.encode(turn, forKey: .turn)
            try c.encode(deadline, forKey: .deadlineMs)
        case .lmRequest(let call, let turn, let deadline, let heard):
            try c.encode(call, forKey: .call)
            try c.encode(turn, forKey: .turn)
            try c.encode(deadline, forKey: .deadlineMs)
            try c.encode(heard, forKey: .heard)
        case .ttsRequest(let call, let turn, let seq, let text, let voice, let deadline):
            try c.encode(call, forKey: .call)
            try c.encode(turn, forKey: .turn)
            try c.encode(seq, forKey: .seq)
            try c.encode(text, forKey: .text)
            try c.encode(voice, forKey: .voice)
            try c.encode(deadline, forKey: .deadlineMs)
        case .cancel(let call): try c.encode(call, forKey: .call)
        case .stopPlayback(let fence): try c.encode(fence, forKey: .fence)
        case .confirmRequest(let turn, let digest, let heard, let tool, let slots, let reasons):
            try c.encode(turn, forKey: .turn)
            try c.encode(digest, forKey: .digest)
            try c.encode(heard, forKey: .heard)
            try c.encode(tool, forKey: .tool)
            try c.encode(slots, forKey: .slots)
            try c.encode(reasons, forKey: .reasons)
        case .turnClosed(let turn, let outcome):
            try c.encode(turn, forKey: .turn)
            try c.encode(outcome, forKey: .outcome)
        case .degraded(let turn, let stage, let reason):
            try c.encode(turn, forKey: .turn)
            try c.encode(stage, forKey: .stage)
            try c.encode(reason, forKey: .reason)
        case .consentState(let state):
            try c.encode(state.capture, forKey: .capture)
            try c.encode(state.personalVoice, forKey: .personalVoice)
        case .notice(let code, let turn):
            try c.encode(code, forKey: .code)
            try c.encode(turn, forKey: .turn)
        case .flushed(let seq): try c.encode(seq, forKey: .seq)
        case .error(let code, let detail):
            try c.encode(code, forKey: .code)
            try c.encode(detail, forKey: .detail)
        }
    }
}

// MARK: - Line coding

public enum LineCodec {
    /// Encodes one message as a JSON line (newline included).
    public static func encode(_ message: ClientMessage) throws -> Data {
        var data = try JSONEncoder().encode(message)
        data.append(0x0A)
        return data
    }

    public static func decodeService(_ line: Data) throws -> ServiceMessage {
        try JSONDecoder().decode(ServiceMessage.self, from: line)
    }
}
