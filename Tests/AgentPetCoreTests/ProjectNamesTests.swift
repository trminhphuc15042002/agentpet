import XCTest
@testable import AgentPetCore

final class ProjectNamesTests: XCTestCase {
    func testFolderNameIsLastComponentAndWholePathForRoot() {
        XCTAssertEqual(ProjectNames.folderName("/Users/me/Projects/helm-claw"), "helm-claw")
        XCTAssertEqual(ProjectNames.folderName("/"), "/")
    }

    func testLabelFallsBackToFolderWhenNoOrBlankName() {
        let path = "/Users/me/Projects"
        XCTAssertEqual(ProjectNames.label(for: path, names: [:]), "Projects")
        XCTAssertEqual(ProjectNames.label(for: path, names: [ProjectNames.id(for: path): ""]), "Projects")
    }

    func testLabelUsesCustomNameKeyedByPathId() {
        let path = "/Users/me/Projects"
        let names = [ProjectNames.id(for: path): "Scratch"]
        XCTAssertEqual(ProjectNames.label(for: path, names: names), "Scratch")
        // A different folder with the same last component is a different project.
        XCTAssertEqual(ProjectNames.label(for: "/tmp/Projects", names: names), "Projects")
    }

    func testCleanTrimsAndCaps() {
        XCTAssertEqual(ProjectNames.clean("  hello \n"), "hello")
        XCTAssertEqual(ProjectNames.clean("   "), "")
        XCTAssertEqual(ProjectNames.clean(String(repeating: "x", count: 100)).count, ProjectNames.maxLength)
    }

    /// The id must equal what the usage store (and the Windows app) compute, or a
    /// name set from the usage-derived project list would never match a session.
    func testIdMatchesKnownFnv1aVectors() {
        // FNV-1a 32-bit of "" and "a" (public test vectors), prefixed with "p".
        XCTAssertEqual(ProjectNames.id(for: ""), "p811c9dc5")
        XCTAssertEqual(ProjectNames.id(for: "a"), "pe40c292c")
    }

    func testTickerLineUsesCustomName() {
        let path = "/Users/me/Projects"
        let s = AgentSession(id: "s1", agentKind: .jcode, project: path, state: .working,
                             message: "hi", source: .hook, updatedAt: Date(timeIntervalSince1970: 0))
        XCTAssertEqual(TickerFormatter.line(for: s), "jcode [Projects] → hi")
        XCTAssertEqual(
            TickerFormatter.line(for: s, projectNames: [ProjectNames.id(for: path): "Scratch"]),
            "jcode [Scratch] → hi")
    }
}
