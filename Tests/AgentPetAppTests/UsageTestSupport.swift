import XCTest
@testable import agentpet

/// Helpers shared by the usage tests.
enum UsageTestSupport {
    /// Parses a JSON object the way the live probes do (numbers become NSNumber).
    static func json(_ s: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(s.utf8)) as? [String: Any])
    }

    /// A provider with one window `left` full, for visibility and mood tests.
    static func provider(_ id: String, left: Double) -> OpenUsageClient.Provider {
        OpenUsageClient.Provider(id: id, displayName: id, plan: nil, todayLabel: nil,
                                 windows: [.init(label: "Session", fractionLeft: left)])
    }

    /// A UserDefaults suite that is empty on entry and wiped after `body`.
    static func withCleanDefaults(_ body: (UserDefaults) throws -> Void) throws {
        let suite = "agentpet.tests.usageVisibility"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        try body(defaults)
    }
}
