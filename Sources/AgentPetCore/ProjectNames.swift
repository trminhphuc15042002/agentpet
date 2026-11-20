import Foundation

/// Friendly names for projects, keyed by the same FNV-1a id the usage store and
/// the Windows app use, so a name follows the folder no matter how the path is
/// spelled. Empty means "show the folder name", as before.
public enum ProjectNames {
    public static let maxLength = 60

    /// Stable id for a project path (same hash as `ProjectUsageStore`).
    public static func id(for path: String) -> String {
        var h: UInt32 = 2166136261
        for b in path.utf8 { h ^= UInt32(b); h = h &* 16777619 }
        return "p" + String(format: "%08x", h)
    }

    /// The folder name, or the whole path for `/`.
    public static func folderName(_ path: String) -> String {
        let last = (path as NSString).lastPathComponent
        return last.isEmpty ? path : last
    }

    /// Trimmed and capped; blank means "no custom name".
    public static func clean(_ name: String) -> String {
        String(name.trimmingCharacters(in: .whitespacesAndNewlines).prefix(maxLength))
    }

    /// The custom name for `path`, else the folder name.
    public static func label(for path: String, names: [String: String]) -> String {
        if let custom = names[id(for: path)], !custom.isEmpty { return custom }
        return folderName(path)
    }
}
