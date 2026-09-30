import XCTest
@testable import AgentPetCore

/// Phase 2 approve/deny round trip over the socket, plus the CLI hook's JSON helper.
final class ApprovalRoundTripTests: XCTestCase {
    private final class Box<T>: @unchecked Sendable {
        var value: T?
    }

    private func makeApprovalEvent(requestId: String) -> AgentEvent {
        AgentEvent(
            sessionId: "s1", agentKind: .claude, eventName: "PreToolUse",
            approvalRequestId: requestId, toolName: "Bash", toolSummary: "rm -rf /tmp/x",
            timestamp: Date(timeIntervalSince1970: 1)
        )
    }

    func testApproveRoundTrip() throws {
        let path = "/tmp/agentpet-\(UUID().uuidString).sock"
        let server = EventSocketServer(path: path)
        defer { server.stop() }

        let requestId = "req-approve-\(UUID().uuidString)"
        let eventReceived = expectation(description: "onEvent invoked")
        try server.start { event in
            XCTAssertEqual(event.approvalRequestId, requestId)
            PendingApprovalRegistry.shared.resolve(requestId: requestId, decision: .allow)
            eventReceived.fulfill()
        }

        let event = makeApprovalEvent(requestId: requestId)
        let resultBox = Box<ApprovalDecision>()
        let replyReceived = expectation(description: "reply received")
        DispatchQueue.global().async {
            resultBox.value = EventSender.sendAndAwaitReply(event, socketPath: path, timeout: 5)
            replyReceived.fulfill()
        }

        wait(for: [eventReceived, replyReceived], timeout: 5)
        XCTAssertEqual(resultBox.value, .allow)
    }

    func testDenyRoundTrip() throws {
        let path = "/tmp/agentpet-\(UUID().uuidString).sock"
        let server = EventSocketServer(path: path)
        defer { server.stop() }

        let requestId = "req-deny-\(UUID().uuidString)"
        let eventReceived = expectation(description: "onEvent invoked")
        try server.start { event in
            PendingApprovalRegistry.shared.resolve(requestId: requestId, decision: .deny)
            eventReceived.fulfill()
        }

        let event = makeApprovalEvent(requestId: requestId)
        let resultBox = Box<ApprovalDecision>()
        let replyReceived = expectation(description: "reply received")
        DispatchQueue.global().async {
            resultBox.value = EventSender.sendAndAwaitReply(event, socketPath: path, timeout: 5)
            replyReceived.fulfill()
        }

        wait(for: [eventReceived, replyReceived], timeout: 5)
        XCTAssertEqual(resultBox.value, .deny)
    }

    func testTimeoutReturnsAskWithoutHanging() throws {
        let path = "/tmp/agentpet-\(UUID().uuidString).sock"
        let server = EventSocketServer(path: path)
        defer { server.stop() }

        let requestId = "req-timeout-\(UUID().uuidString)"
        let eventReceived = expectation(description: "onEvent invoked")
        // Deliberately never resolve this request; the client must time out on its own.
        try server.start { _ in eventReceived.fulfill() }

        let event = makeApprovalEvent(requestId: requestId)
        let resultBox = Box<ApprovalDecision>()
        let replyReceived = expectation(description: "reply received")
        DispatchQueue.global().async {
            resultBox.value = EventSender.sendAndAwaitReply(event, socketPath: path, timeout: 1)
            replyReceived.fulfill()
        }

        wait(for: [eventReceived, replyReceived], timeout: 3)
        XCTAssertEqual(resultBox.value, .ask)
    }

    func testNoDaemonReturnsAskImmediately() {
        let path = "/tmp/agentpet-\(UUID().uuidString).sock" // nothing listening here
        let start = Date()
        let decision = EventSender.sendAndAwaitReply(
            makeApprovalEvent(requestId: "req-no-daemon"), socketPath: path, timeout: 5
        )
        XCTAssertEqual(decision, .ask)
        XCTAssertLessThan(Date().timeIntervalSince(start), 2, "connect failure must not wait for the timeout")
    }

    func testNonApprovalEventClosesConnectionAfterSend() throws {
        let path = "/tmp/agentpet-\(UUID().uuidString).sock"
        let server = EventSocketServer(path: path)
        defer { server.stop() }

        let eventReceived = expectation(description: "onEvent invoked")
        let box = Box<AgentEvent>()
        try server.start { event in
            box.value = event
            eventReceived.fulfill()
        }

        let event = AgentEvent(
            sessionId: "s-plain", agentKind: .claude, eventName: "Stop",
            timestamp: Date(timeIntervalSince1970: 42)
        )
        let dir = NSTemporaryDirectory() + "agentpet-q-\(UUID().uuidString)"
        let delivered = EventSender.send(event, socketPath: path, queueDir: dir)

        wait(for: [eventReceived], timeout: 5)
        XCTAssertTrue(delivered, "socket path exists, must deliver over the socket (not fall back to queue)")
        XCTAssertEqual(box.value?.approvalRequestId, nil)
        XCTAssertEqual(box.value, event)
    }

    /// Root cause of the old flaky `testApproveRoundTrip`: on macOS,
    /// setting SO_RCVTIMEO on a socket whose peer already replied and closed
    /// fails with EINVAL. `sendAndAwaitReply` must therefore set it before
    /// writing, while the peer can't have replied yet. This pins the OS
    /// behaviour the ordering depends on.
    func testReceiveTimeoutFailsOncePeerHasClosed() {
        var fds: [Int32] = [0, 0]
        fds.withUnsafeMutableBufferPointer { _ = socketpair(AF_UNIX, SOCK_STREAM, 0, $0.baseAddress) }
        defer { close(fds[0]) }
        var tv = timeval(tv_sec: 1, tv_usec: 0)
        let size = socklen_t(MemoryLayout<timeval>.size)
        XCTAssertEqual(setsockopt(fds[0], SOL_SOCKET, SO_RCVTIMEO, &tv, size), 0, "peer open: must succeed")

        _ = Data("x\n".utf8).withUnsafeBytes { write(fds[1], $0.baseAddress, $0.count) }
        close(fds[1])
        XCTAssertNotEqual(setsockopt(fds[0], SOL_SOCKET, SO_RCVTIMEO, &tv, size), 0,
                          "peer closed: macOS rejects SO_RCVTIMEO")
    }

    func testApprovalResponseJSONForAllow() throws {
        try assertApprovalResponseJSON(.allow, expectedPermissionDecision: "allow")
    }

    func testApprovalResponseJSONForDeny() throws {
        try assertApprovalResponseJSON(.deny, expectedPermissionDecision: "deny")
    }

    func testApprovalResponseJSONForAsk() throws {
        try assertApprovalResponseJSON(.ask, expectedPermissionDecision: "ask")
    }

    private func assertApprovalResponseJSON(
        _ decision: ApprovalDecision, expectedPermissionDecision: String,
        file: StaticString = #filePath, line: UInt = #line
    ) throws {
        let json = ApprovalHookResponse.json(for: decision)
        let data = try XCTUnwrap(json.data(using: .utf8), file: file, line: line)
        let parsed = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let hookSpecificOutput = try XCTUnwrap(
            parsed?["hookSpecificOutput"] as? [String: Any], file: file, line: line
        )
        XCTAssertEqual(
            hookSpecificOutput["hookEventName"] as? String, "PreToolUse", file: file, line: line
        )
        XCTAssertEqual(
            hookSpecificOutput["permissionDecision"] as? String, expectedPermissionDecision,
            file: file, line: line
        )
    }
}
