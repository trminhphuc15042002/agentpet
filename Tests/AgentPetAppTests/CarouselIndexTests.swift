import XCTest
@testable import agentpet

final class CarouselIndexTests: XCTestCase {
    /// A group dropping out leaves `index` past the end; it must still map to
    /// a real page (this was drawn as an empty bubble before).
    func testStaleIndexMapsToExistingPage() {
        XCTAssertEqual(CarouselIndex.shown(1, count: 1), 0)
        XCTAssertEqual(CarouselIndex.shown(2, count: 2), 0)
        XCTAssertEqual(CarouselIndex.shown(5, count: 3), 2)
    }

    func testEmptyCarouselShowsPageZero() {
        XCTAssertEqual(CarouselIndex.shown(3, count: 0), 0)
        XCTAssertEqual(CarouselIndex.step(3, by: 1, count: 0), 0)
    }

    func testStepWrapsBothWays() {
        XCTAssertEqual(CarouselIndex.step(0, by: 1, count: 3), 1)
        XCTAssertEqual(CarouselIndex.step(2, by: 1, count: 3), 0)
        XCTAssertEqual(CarouselIndex.step(0, by: -1, count: 3), 2)
    }

    /// Stepping with the live count after a shrink stays in range, whatever
    /// the stale index was.
    func testStepAfterShrinkStaysInRange() {
        for stale in 0..<6 {
            let next = CarouselIndex.step(stale, by: 1, count: 1)
            XCTAssertEqual(next, 0, "stale index \(stale)")
        }
    }
}
