// The SwiftUI shell. V0: a window that shows the build is alive; V1/V2 add push-to-talk,
// the transcript and the session process.
import SwiftUI
import VoiceCore

@main
struct VoiceAgentApp: App {
    var body: some Scene {
        WindowGroup("Personal Voice Agent") {
            Text("Personal Voice Agent — protocol v\(protocolVersion)")
                .padding(40)
        }
    }
}
