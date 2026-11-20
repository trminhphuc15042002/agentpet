import SwiftUI

/// One provider's limit windows: a row per window with its own bar, percent
/// used and reset time. Shared by the stats HUD (compact) and the Care tab.
struct LimitWindowsView: View {
    let provider: OpenUsageClient.Provider
    /// Bar colour while a window is comfortably within budget.
    let tint: Color
    var compact = true

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 4 : 6) {
            HStack(spacing: 6) {
                Text(verbatim: provider.displayName)
                    .font(compact ? .system(size: 11, weight: .semibold) : .callout.weight(.medium))
                    .foregroundStyle(compact ? AnyShapeStyle(.white.opacity(0.9)) : AnyShapeStyle(.primary))
                if let plan = provider.plan, !plan.isEmpty {
                    Text(verbatim: plan)
                        .font(.system(size: compact ? 9 : 10))
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }
            // A provider with no limit (text/badge only) has no windows, so it
            // draws no bar rather than a fake "100% used".
            ForEach(Array(provider.windows.enumerated()), id: \.offset) { _, w in
                row(label: LimitFormat.title(w), left: w.fractionLeft, reset: w.resetsAt)
            }
        }
    }

    private func row(label: String, left: Double, reset: Date?) -> some View {
        let used = 1 - left
        let color: Color = used > 0.9 ? .red : (used > 0.75 ? .orange : tint)
        let small: Font = compact ? .system(size: 9) : .caption
        return VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 5) {
                Text(verbatim: label).font(small).foregroundStyle(.secondary).lineLimit(1)
                Spacer(minLength: 4)
                Text(String(format: NSLocalizedString("%d%% used", comment: ""), Int((used * 100).rounded())))
                    .font(compact ? .system(size: 10, weight: .semibold) : .caption.weight(.semibold))
                    .foregroundStyle(color)
                // The HUD is a fixed 300pt card: show only the wall-clock reset
                // there; the Care tab has room for "resets in 2d · Mon 11:00".
                if let text = compact ? LimitFormat.compactReset(reset) : LimitFormat.reset(reset) {
                    Text(verbatim: "· \(text)").font(small).foregroundStyle(.secondary).lineLimit(1)
                        .fixedSize()
                }
            }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.primary.opacity(0.1))
                    Capsule().fill(color).frame(width: max(2, geo.size.width * used))
                }
            }
            .frame(height: compact ? 4 : 5)
        }
    }
}
