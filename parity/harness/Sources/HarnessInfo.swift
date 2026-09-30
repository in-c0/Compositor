import Foundation
import Metal

/// harness-info.json: the machine and toolchain the references were made on, and how each case went.
nonisolated struct HarnessInfo: Encodable {
    nonisolated struct GPU: Encodable {
        /// Whether `MTLCreateSystemDefaultDevice()` returned a device. Layer effects render with Metal when it does
        /// and fall back to the CPU when it doesn't, so references made both ways can differ.
        let metalAvailable: Bool
        let device: String?
    }

    /// `ProcessInfo.operatingSystemVersionString`, such as "Version 26.0 (Build 25A354)".
    let macOS: String
    let macOSVersion: String
    let architecture: String
    let buildConfiguration: String
    /// `xcrun xcodebuild -version` and `xcrun swiftc --version` on the rendering machine, when they run.
    let xcode: String?
    let swiftCompiler: String?
    let gpu: GPU
    let cases: [CaseResult]
}

extension HarnessInfo {
    static func collect(cases: [CaseResult]) -> HarnessInfo {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        let device = MTLCreateSystemDefaultDevice()
        #if arch(arm64)
        let architecture = "arm64"
        #elseif arch(x86_64)
        let architecture = "x86_64"
        #else
        let architecture = "unknown"
        #endif
        #if DEBUG
        let configuration = "Debug"
        #else
        let configuration = "Release"
        #endif
        return HarnessInfo(
            macOS: ProcessInfo.processInfo.operatingSystemVersionString,
            macOSVersion: "\(version.majorVersion).\(version.minorVersion).\(version.patchVersion)",
            architecture: architecture,
            buildConfiguration: configuration,
            xcode: commandOutput("/usr/bin/xcrun", ["xcodebuild", "-version"]),
            swiftCompiler: commandOutput("/usr/bin/xcrun", ["swiftc", "--version"]),
            gpu: GPU(metalAvailable: device != nil, device: device?.name),
            cases: cases)
    }

    /// A tool's output, or nil if it can't be run or fails.
    private static func commandOutput(_ executable: String, _ arguments: [String]) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        process.standardInput = FileHandle.nullDevice
        do { try process.run() } catch { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}
