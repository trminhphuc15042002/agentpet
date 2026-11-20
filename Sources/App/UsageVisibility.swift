import Foundation
import SwiftUI

/// Which subscription providers the user wants to see. Hidden providers are
/// left out of the Limits rows and of the pet's "limit low" mood, so an AI
/// you never use can't clutter the HUD or make the pet anxious.
@MainActor
final class UsageVisibility: ObservableObject {
    static let shared = UsageVisibility()

    private static let key = "agentpet.usage.hiddenProviders"
    private let defaults: UserDefaults

    @Published private(set) var hidden: Set<String>

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        hidden = Set(defaults.stringArray(forKey: Self.key) ?? [])
    }

    func isVisible(_ id: String) -> Bool { !hidden.contains(id) }

    func setVisible(_ id: String, _ visible: Bool) {
        if visible { hidden.remove(id) } else { hidden.insert(id) }
        defaults.set(hidden.sorted(), forKey: Self.key)
    }

    func binding(for id: String) -> Binding<Bool> {
        Binding(get: { self.isVisible(id) }, set: { self.setVisible(id, $0) })
    }

    func visible(_ providers: [OpenUsageClient.Provider]) -> [OpenUsageClient.Provider] {
        providers.filter { isVisible($0.id) }
    }

    /// Tightest budget left among visible providers. Feeds the rate-limit bubble.
    func lowestFractionLeft(_ providers: [OpenUsageClient.Provider]) -> Double? {
        visible(providers).compactMap(\.fractionLeft).min()
    }

    /// True when a visible provider is nearly spent. Feeds the pet's anxious mood.
    func limitLow(_ providers: [OpenUsageClient.Provider]) -> Bool {
        guard let left = lowestFractionLeft(providers) else { return false }
        return left < Thresholds.RateLimit.low
    }
}
