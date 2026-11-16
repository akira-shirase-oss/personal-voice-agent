// swift-tools-version: 6.2
// The macOS client: VoiceCore (protocol, session process, providers, player) and the
// VoiceAgent SwiftUI app. Needs the macOS 26 SDK (SpeechAnalyzer, FoundationModels).
import PackageDescription

let package = Package(
    name: "VoiceAgent",
    platforms: [.macOS("26.0")],
    products: [
        .library(name: "VoiceCore", targets: ["VoiceCore"]),
        .executable(name: "VoiceAgent", targets: ["VoiceAgent"]),
    ],
    targets: [
        .target(name: "VoiceCore"),
        .executableTarget(name: "VoiceAgent", dependencies: ["VoiceCore"]),
        .testTarget(name: "VoiceCoreTests", dependencies: ["VoiceCore"]),
    ],
    swiftLanguageModes: [.v6]
)
