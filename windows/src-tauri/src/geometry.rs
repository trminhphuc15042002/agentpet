//! Pure pet-window geometry (no Tauri). Port of macOS `PetWindowGeometry`
//! plus Windows top-left / physical-pixel helpers. Standalone-testable:
//! `rustc --test geometry.rs`.

pub const LEGACY_LOGICAL_WIDTH: f64 = 260.0;
pub const LEGACY_LOGICAL_HEIGHT: f64 = 320.0;
pub const CONTENT_PAD: f64 = 4.0;

#[inline]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if lo > hi {
        hi
    } else {
        v.max(lo).min(hi)
    }
}

#[inline]
pub fn pad_content(width: f64, height: f64) -> (f64, f64) {
    (width + CONTENT_PAD, height + CONTENT_PAD)
}

#[inline]
pub fn logical_to_physical(v: f64, scale: f64) -> f64 {
    v * scale
}

#[inline]
pub fn physical_to_logical(v: f64, scale: f64) -> f64 {
    if scale == 0.0 {
        v
    } else {
        v / scale
    }
}

/// Axis-aligned visible rect in the same coordinate space as the origin
/// (physical pixels, top-left origin on Windows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleRect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl VisibleRect {
    pub fn from_pos_size(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            min_x: x,
            min_y: y,
            max_x: x + w,
            max_y: y + h,
        }
    }

    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x < self.max_x && y >= self.min_y && y < self.max_y
    }
}

/// Clamps a top-left `origin` so a window of `width`×`height` sits fully inside
/// `visible`. If the window is larger than the visible area on an axis, it pins
/// to that axis's minimum edge.
pub fn clamp_origin(
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    visible: VisibleRect,
) -> (f64, f64) {
    let max_x = visible.min_x.max(visible.max_x - width);
    let max_y = visible.min_y.max(visible.max_y - height);
    (
        clamp(origin_x, visible.min_x, max_x),
        clamp(origin_y, visible.min_y, max_y),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HorizontalLayout {
    pub origin_x: f64,
    pub pet_offset: f64,
}

/// Horizontal placement for a pet window of `width` whose pet (`pet_width`
/// wide) should stay centred at `anchor_x`. The window stays inside
/// `[visible_min_x, visible_max_x]`; the pet is then shifted inside the window
/// by `pet_offset` so it does not move on screen.
pub fn horizontal_layout(
    anchor_x: f64,
    width: f64,
    pet_width: f64,
    visible_min_x: f64,
    visible_max_x: f64,
) -> HorizontalLayout {
    let max_origin_x = visible_min_x.max(visible_max_x - width);
    let origin_x = clamp(anchor_x - width / 2.0, visible_min_x, max_origin_x);
    let limit = 0.0_f64.max((width - pet_width) / 2.0);
    let offset = clamp(anchor_x - (origin_x + width / 2.0), -limit, limit);
    HorizontalLayout {
        origin_x,
        pet_offset: offset,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleLayout {
    pub bubble_shift: f64,
    pub tail_shift: f64,
}

/// Keeps a speech bubble over the pet when the pet is offset inside its window.
pub fn bubble_layout(
    pet_offset: f64,
    window_width: f64,
    bubble_width: f64,
    bubble_inset: f64,
    tail_clearance: f64,
) -> BubbleLayout {
    let room = 0.0_f64.max((window_width - bubble_width) / 2.0);
    let bubble_shift = clamp(pet_offset, -room, room);
    let tail_limit = 0.0_f64.max((bubble_width - 2.0 * bubble_inset) / 2.0 - tail_clearance);
    BubbleLayout {
        bubble_shift,
        tail_shift: clamp(pet_offset - bubble_shift, -tail_limit, tail_limit),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowLayout {
    pub origin_x: f64,
    pub origin_y: f64,
    pub pet_offset: f64,
}

/// Resize around a fixed bottom-center anchor (Windows top-left origin).
/// X uses `horizontal_layout`. Y keeps the bottom at `anchor_y` unless the
/// taller window would run off the top of the work area.
pub fn layout_window(
    anchor_x: f64,
    anchor_y: f64,
    width: f64,
    height: f64,
    pet_width: f64,
    visible: VisibleRect,
) -> WindowLayout {
    let h = horizontal_layout(anchor_x, width, pet_width, visible.min_x, visible.max_x);
    let max_y = visible.min_y.max(visible.max_y - height);
    let origin_y = clamp(anchor_y - height, visible.min_y, max_y);
    WindowLayout {
        origin_x: h.origin_x,
        origin_y,
        pet_offset: h.pet_offset,
    }
}

/// True when a top-left frame is not fully inside `work` (partially off the
/// work area, not only "monitor unplugged" / oversized).
pub fn frame_outside_work(
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    work: VisibleRect,
) -> bool {
    let (cx, cy) = clamp_origin(origin_x, origin_y, width, height, work);
    (cx - origin_x).abs() > 0.5 || (cy - origin_y).abs() > 0.5
}

/// Clamp using the **live** frame's bottom-center (not a stale stored anchor).
pub fn layout_from_live_frame(
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    pet_offset: f64,
    pet_width: f64,
    work: VisibleRect,
) -> WindowLayout {
    let (ax, ay) = anchor_from_frame(origin_x, origin_y, width, height, pet_offset);
    layout_window(ax, ay, width, height, pet_width.max(1.0), work)
}

/// Bottom-center of a window at top-left `origin` with physical `width`×`height`,
/// plus an in-window pet offset (same units as width).
pub fn anchor_from_frame(
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    pet_offset: f64,
) -> (f64, f64) {
    (origin_x + width / 2.0 + pet_offset, origin_y + height)
}

/// Legacy saved origin was the top-left of a 260×320 *logical* window.
/// Convert to a physical bottom-center anchor using that display's DPI scale.
pub fn migrate_legacy_origin(origin_x: f64, origin_y: f64, scale: f64) -> (f64, f64) {
    let w = logical_to_physical(LEGACY_LOGICAL_WIDTH, scale);
    let h = logical_to_physical(LEGACY_LOGICAL_HEIGHT, scale);
    (origin_x + w / 2.0, origin_y + h)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SavedPos {
    /// Pre-v2 raw window origin (physical px), assumed 260×320 logical.
    Legacy { x: f64, y: f64 },
    /// v2 pet bottom-center anchor (physical px).
    Anchor { x: f64, y: f64 },
}

/// Parse the versioned pos file. `x,y` → legacy; `2,ax,ay` → anchor.
pub fn parse_saved_pos(s: &str) -> Option<SavedPos> {
    let parts: Vec<&str> = s.trim().split(',').map(str::trim).collect();
    match parts.as_slice() {
        [a, b] => Some(SavedPos::Legacy {
            x: a.parse().ok()?,
            y: b.parse().ok()?,
        }),
        ["2", a, b] => Some(SavedPos::Anchor {
            x: a.parse().ok()?,
            y: b.parse().ok()?,
        }),
        _ => None,
    }
}

pub fn format_anchor(x: f64, y: f64) -> String {
    format!("2,{x},{y}")
}

/// Resolve a saved position to a physical bottom-center anchor.
pub fn saved_anchor(pos: SavedPos, scale: f64) -> (f64, f64) {
    match pos {
        SavedPos::Legacy { x, y } => migrate_legacy_origin(x, y, scale),
        SavedPos::Anchor { x, y } => (x, y),
    }
}

fn dist2_to_rect(x: f64, y: f64, r: VisibleRect) -> f64 {
    let cx = clamp(x, r.min_x, r.max_x);
    let cy = clamp(y, r.min_y, r.max_y);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy
}

/// Monitor containing physical `x,y`, else the nearest frame (removed display).
pub fn nearest_monitor(x: f64, y: f64, monitors: &[MonitorGeom]) -> Option<usize> {
    monitors
        .iter()
        .position(|m| m.frame.contains_point(x, y))
        .or_else(|| {
            let mut best: Option<(usize, f64)> = None;
            for (i, m) in monitors.iter().enumerate() {
                let d = dist2_to_rect(x, y, m.frame);
                if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                    best = Some((i, d));
                }
            }
            best.map(|(i, _)| i)
        })
}

/// DPI for a legacy physical origin: the monitor that contains it, else nearest.
pub fn scale_for_legacy_origin(origin_x: f64, origin_y: f64, monitors: &[MonitorGeom]) -> Option<f64> {
    nearest_monitor(origin_x, origin_y, monitors).map(|i| monitors[i].scale)
}

/// v2 anchors are already physical (scale ignored). Legacy uses the origin's monitor.
pub fn saved_anchor_on_monitors(
    pos: SavedPos,
    monitors: &[MonitorGeom],
    fallback_scale: f64,
) -> (f64, f64) {
    match pos {
        SavedPos::Legacy { x, y } => {
            let scale = scale_for_legacy_origin(x, y, monitors).unwrap_or(fallback_scale);
            migrate_legacy_origin(x, y, scale)
        }
        SavedPos::Anchor { x, y } => (x, y),
    }
}

/// Monitor in physical pixels: full frame for hit-testing, work area for clamp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorGeom {
    pub frame: VisibleRect,
    pub work: VisibleRect,
    pub scale: f64,
}

/// Screen holding the pet's bottom-center (probe 1px above the bottom so a
/// pet sitting on the edge still belongs to that display), else `fallback`.
pub fn monitor_containing_pet(
    anchor_x: f64,
    anchor_y: f64,
    fallback_cx: f64,
    fallback_cy: f64,
    monitors: &[MonitorGeom],
) -> Option<usize> {
    let probe_y = anchor_y - 1.0;
    monitors
        .iter()
        .position(|m| m.frame.contains_point(anchor_x, probe_y))
        .or_else(|| {
            monitors
                .iter()
                .position(|m| m.frame.contains_point(fallback_cx, fallback_cy))
        })
}

pub fn size_changed(a_w: f64, a_h: f64, b_w: f64, b_h: f64, tol: f64) -> bool {
    (a_w - b_w).abs() > tol || (a_h - b_h).abs() > tol
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIS: VisibleRect = VisibleRect {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 1920.0,
        max_y: 1055.0,
    };
    const W: f64 = 260.0;
    const H: f64 = 320.0;

    fn almost(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{} != {}", a, b);
    }

    #[test]
    fn inside_position_is_unchanged() {
        let (x, y) = clamp_origin(800.0, 400.0, W, H, VIS);
        almost(x, 800.0);
        almost(y, 400.0);
    }

    #[test]
    fn off_right_edge_clamps_back_in() {
        let (x, y) = clamp_origin(2181.0, 375.0, W, H, VIS);
        almost(x, 1920.0 - 260.0);
        almost(y, 375.0);
        assert!(x + W <= VIS.max_x);
    }

    #[test]
    fn off_left_and_top_clamp_to_min_edges() {
        let (x, y) = clamp_origin(-500.0, -200.0, W, H, VIS);
        almost(x, 0.0);
        almost(y, 0.0);
    }

    #[test]
    fn window_larger_than_screen_pins_to_min_edge() {
        let (x, _) = clamp_origin(2181.0, 40.0, 3000.0, H, VIS);
        almost(x, 0.0);
    }

    #[test]
    fn offscreen_origin_on_shifted_screen_clamps() {
        let right = VisibleRect::from_pos_size(1920.0, 0.0, 1920.0, 1055.0);
        let (x, _) = clamp_origin(5000.0, 400.0, W, H, right);
        almost(x, 1920.0 + 1920.0 - 260.0);
    }

    #[test]
    fn layout_centred_when_room_on_both_sides() {
        let r = horizontal_layout(800.0, 364.0, 120.0, 0.0, 1680.0);
        almost(r.origin_x, 800.0 - 182.0);
        almost(r.pet_offset, 0.0);
    }

    #[test]
    fn layout_near_right_edge_shifts_window_but_not_pet() {
        let r = horizontal_layout(1558.0, 364.0, 120.0, 0.0, 1680.0);
        almost(r.origin_x, 1680.0 - 364.0);
        almost(r.origin_x + 364.0 / 2.0 + r.pet_offset, 1558.0);
    }

    #[test]
    fn layout_near_left_edge_on_shifted_screen() {
        let r = horizontal_layout(1760.0, 364.0, 120.0, 1680.0, 3728.0);
        almost(r.origin_x, 1680.0);
        almost(r.origin_x + 182.0 + r.pet_offset, 1760.0);
    }

    #[test]
    fn layout_pet_past_edge_is_pulled_inside_window() {
        let r = horizontal_layout(1670.0, 364.0, 120.0, 0.0, 1680.0);
        almost(r.origin_x, 1316.0);
        almost(r.pet_offset, 122.0);
    }

    #[test]
    fn layout_window_wider_than_screen_pins_to_min_x() {
        let r = horizontal_layout(100.0, 500.0, 120.0, 0.0, 400.0);
        almost(r.origin_x, 0.0);
        almost(r.pet_offset, -150.0);
    }

    fn bubble(offset: f64, window: f64, bubble: f64) -> BubbleLayout {
        bubble_layout(offset, window, bubble, 10.0, 20.0)
    }

    #[test]
    fn settled_wide_bubble_stays_put_and_tail_points_at_pet() {
        let r = bubble(60.0, 364.0, 364.0);
        almost(r.bubble_shift, 0.0);
        almost(r.tail_shift, 60.0);
    }

    #[test]
    fn narrow_bubble_moves_over_pet() {
        let r = bubble(60.0, 364.0, 120.0);
        almost(r.bubble_shift, 60.0);
        almost(r.tail_shift, 0.0);
    }

    #[test]
    fn shrinking_bubble_uses_old_window_width() {
        let r = bubble(-100.0, 364.0, 150.0);
        almost(r.bubble_shift, -100.0);
        almost(r.tail_shift, 0.0);
    }

    #[test]
    fn tail_clear_of_corners() {
        let r = bubble(50.0, 100.0, 100.0);
        almost(r.bubble_shift, 0.0);
        almost(r.tail_shift, 20.0);
    }

    #[test]
    fn no_offset_is_no_shift() {
        let r = bubble(0.0, 364.0, 200.0);
        almost(r.bubble_shift, 0.0);
        almost(r.tail_shift, 0.0);
    }

    #[test]
    fn layout_window_keeps_bottom_center_when_centred() {
        let vis = VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1080.0);
        let r = layout_window(800.0, 1000.0, 364.0, 400.0, 120.0, vis);
        almost(r.origin_x, 800.0 - 182.0);
        almost(r.origin_y, 1000.0 - 400.0);
        almost(r.pet_offset, 0.0);
        let (ax, ay) = anchor_from_frame(r.origin_x, r.origin_y, 364.0, 400.0, r.pet_offset);
        almost(ax, 800.0);
        almost(ay, 1000.0);
    }

    #[test]
    fn layout_window_right_edge_oversize_offset() {
        let vis = VisibleRect::from_pos_size(0.0, 0.0, 1680.0, 1050.0);
        let r = layout_window(1558.0, 900.0, 364.0, 400.0, 120.0, vis);
        almost(r.origin_x, 1680.0 - 364.0);
        almost(r.origin_y, 500.0);
        almost(r.origin_x + 182.0 + r.pet_offset, 1558.0);
    }

    #[test]
    fn layout_window_left_edge_oversize_offset() {
        let vis = VisibleRect::from_pos_size(1680.0, 0.0, 2048.0, 1050.0);
        let r = layout_window(1760.0, 800.0, 364.0, 300.0, 120.0, vis);
        almost(r.origin_x, 1680.0);
        almost(r.origin_x + 182.0 + r.pet_offset, 1760.0);
    }

    #[test]
    fn layout_window_nudge_down_when_taller_than_top() {
        let vis = VisibleRect::from_pos_size(0.0, 40.0, 1920.0, 1000.0);
        let r = layout_window(200.0, 100.0, 260.0, 320.0, 120.0, vis);
        almost(r.origin_y, 40.0);
    }

    #[test]
    fn dpi_physical_conversion() {
        almost(logical_to_physical(260.0, 1.5), 390.0);
        almost(physical_to_logical(390.0, 1.5), 260.0);
        let vis = VisibleRect::from_pos_size(0.0, 0.0, 2880.0, 1620.0);
        let w = logical_to_physical(364.0, 1.5);
        let pet = logical_to_physical(120.0, 1.5);
        let r = horizontal_layout(1200.0, w, pet, vis.min_x, vis.max_x);
        almost(r.pet_offset, 0.0);
        almost(physical_to_logical(r.pet_offset, 1.5), 0.0);
    }

    #[test]
    fn dpi_oversize_offset_stays_in_logical_after_convert() {
        let scale = 2.0;
        let vis = VisibleRect::from_pos_size(0.0, 0.0, 1680.0 * scale, 1050.0 * scale);
        let width = logical_to_physical(364.0, scale);
        let pet = logical_to_physical(120.0, scale);
        let anchor = 1558.0 * scale;
        let r = horizontal_layout(anchor, width, pet, vis.min_x, vis.max_x);
        almost(physical_to_logical(r.origin_x, scale), 1680.0 - 364.0);
        almost(physical_to_logical(r.pet_offset, scale), 1558.0 - (1316.0 + 182.0));
    }

    #[test]
    fn legacy_migration_scale_1() {
        let (ax, ay) = migrate_legacy_origin(100.0, 200.0, 1.0);
        almost(ax, 100.0 + 130.0);
        almost(ay, 200.0 + 320.0);
    }

    #[test]
    fn legacy_migration_scale_1_5() {
        let (ax, ay) = migrate_legacy_origin(100.0, 200.0, 1.5);
        almost(ax, 100.0 + 195.0);
        almost(ay, 200.0 + 480.0);
    }

    #[test]
    fn parse_legacy_and_v2_pos() {
        assert_eq!(
            parse_saved_pos("1200,600"),
            Some(SavedPos::Legacy { x: 1200.0, y: 600.0 })
        );
        assert_eq!(
            parse_saved_pos("2,1234.5,567.8"),
            Some(SavedPos::Anchor { x: 1234.5, y: 567.8 })
        );
        assert_eq!(parse_saved_pos("nope"), None);
        assert_eq!(parse_saved_pos("2,1,x"), None);
        assert_eq!(
            parse_saved_pos("2,1"),
            Some(SavedPos::Legacy { x: 2.0, y: 1.0 })
        );
        let a = saved_anchor(SavedPos::Legacy { x: 0.0, y: 0.0 }, 1.0);
        almost(a.0, 130.0);
        almost(a.1, 320.0);
        let b = saved_anchor(SavedPos::Anchor { x: 9.0, y: 8.0 }, 2.0);
        almost(b.0, 9.0);
        almost(b.1, 8.0);
        assert_eq!(format_anchor(1.5, 2.5), "2,1.5,2.5");
    }

    #[test]
    fn monitor_containing_pet_prefers_anchor_screen() {
        let left = MonitorGeom {
            frame: VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1080.0),
            work: VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1040.0),
            scale: 1.0,
        };
        let right = MonitorGeom {
            frame: VisibleRect::from_pos_size(1920.0, 0.0, 1920.0, 1080.0),
            work: VisibleRect::from_pos_size(1920.0, 0.0, 1920.0, 1040.0),
            scale: 1.25,
        };
        let mons = [left, right];
        assert_eq!(
            monitor_containing_pet(2000.0, 800.0, 100.0, 100.0, &mons),
            Some(1)
        );
        assert_eq!(
            monitor_containing_pet(100.0, 800.0, 2000.0, 100.0, &mons),
            Some(0)
        );
        // Off every frame → fallback window center on the right display.
        assert_eq!(
            monitor_containing_pet(-50.0, -50.0, 2000.0, 100.0, &mons),
            Some(1)
        );
        assert_eq!(
            monitor_containing_pet(-50.0, -50.0, -10.0, -10.0, &mons),
            None
        );
    }

    #[test]
    fn size_changed_1px_tolerance() {
        assert!(!size_changed(260.0, 320.0, 260.4, 320.4, 1.0));
        assert!(size_changed(260.0, 320.0, 262.0, 320.0, 1.0));
    }

    #[test]
    fn pad_content_adds_margin() {
        assert_eq!(pad_content(256.0, 316.0), (260.0, 320.0));
    }

    fn mixed_dpi_mons() -> [MonitorGeom; 2] {
        [
            MonitorGeom {
                frame: VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1080.0),
                work: VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1040.0),
                scale: 1.0,
            },
            MonitorGeom {
                frame: VisibleRect::from_pos_size(1920.0, 0.0, 1920.0, 1080.0),
                work: VisibleRect::from_pos_size(1920.0, 0.0, 1920.0, 1040.0),
                scale: 1.5,
            },
        ]
    }

    #[test]
    fn legacy_scale_uses_origin_monitor_not_fallback() {
        let mons = mixed_dpi_mons();
        almost(scale_for_legacy_origin(2000.0, 400.0, &mons).unwrap(), 1.5);
        almost(scale_for_legacy_origin(100.0, 400.0, &mons).unwrap(), 1.0);
        let on_hi = saved_anchor_on_monitors(
            SavedPos::Legacy { x: 2000.0, y: 400.0 },
            &mons,
            1.0,
        );
        let want = migrate_legacy_origin(2000.0, 400.0, 1.5);
        almost(on_hi.0, want.0);
        almost(on_hi.1, want.1);
        let wrong = migrate_legacy_origin(2000.0, 400.0, 1.0);
        assert!((on_hi.0 - wrong.0).abs() > 1.0);
    }

    #[test]
    fn legacy_scale_nearest_fallback_when_monitor_removed() {
        let mons = mixed_dpi_mons();
        almost(scale_for_legacy_origin(-80.0, 400.0, &mons).unwrap(), 1.0);
        almost(scale_for_legacy_origin(5000.0, 400.0, &mons).unwrap(), 1.5);
        let far = saved_anchor_on_monitors(
            SavedPos::Legacy { x: 5000.0, y: 400.0 },
            &mons,
            1.0,
        );
        let want = migrate_legacy_origin(5000.0, 400.0, 1.5);
        almost(far.0, want.0);
        almost(far.1, want.1);
        assert!(scale_for_legacy_origin(0.0, 0.0, &[]).is_none());
        let empty = saved_anchor_on_monitors(
            SavedPos::Legacy { x: 10.0, y: 20.0 },
            &[],
            2.0,
        );
        let fb = migrate_legacy_origin(10.0, 20.0, 2.0);
        almost(empty.0, fb.0);
        almost(empty.1, fb.1);
    }

    #[test]
    fn layout_window_clamps_bottom_when_anchor_below_work() {
        let vis = VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1055.0);
        let r = layout_window(400.0, 1400.0, 260.0, 320.0, 160.0, vis);
        almost(r.origin_y, 1055.0 - 320.0);
        assert!(r.origin_y + 320.0 <= vis.max_y + 1e-9);
    }

    #[test]
    fn frame_outside_work_detects_partial_off_bottom() {
        let work = VisibleRect::from_pos_size(0.0, 0.0, 2560.0, 1525.0);
        assert!(!frame_outside_work(2182.0, 1081.0, 246.0, 339.0, work));
        assert!(frame_outside_work(2314.0, 1485.0, 246.0, 339.0, work));
    }

    #[test]
    fn live_frame_not_stale_anchor_clamps_off_work() {
        let work = VisibleRect::from_pos_size(0.0, 0.0, 1920.0, 1040.0);
        let stale_ax = 800.0;
        let stale_ay = 900.0;
        let ox = 1800.0;
        let oy = 900.0;
        let w = 260.0;
        let h = 320.0;
        let from_stale = layout_window(stale_ax, stale_ay, w, h, 160.0, work);
        assert!(from_stale.origin_x < 1000.0);
        let live = layout_from_live_frame(ox, oy, w, h, 0.0, 160.0, work);
        almost(live.origin_x, 1920.0 - 260.0);
        assert!(live.origin_x + w <= work.max_x + 1e-9);
        assert!(live.origin_y + h <= work.max_y + 1e-9);
    }

    #[test]
    fn mixed_dpi_simulated_clamp_uses_live_frame_monitor_work() {
        let mons = mixed_dpi_mons();
        let ox = 2100.0;
        let oy = 700.0;
        let w = 390.0;
        let h = 480.0;
        let (ax, ay) = anchor_from_frame(ox, oy, w, h, 0.0);
        let idx = monitor_containing_pet(ax, ay, ox + w / 2.0, oy + h / 2.0, &mons).unwrap();
        almost(mons[idx].scale, 1.5);
        let r = layout_from_live_frame(ox, oy, w, h, 0.0, 240.0, mons[idx].work);
        assert!(r.origin_x >= mons[idx].work.min_x - 1e-9);
        assert!(r.origin_x + w <= mons[idx].work.max_x + 1e-9);
        assert!(r.origin_y + h <= mons[idx].work.max_y + 1e-9);
    }

    #[test]
    fn v2_anchor_ignores_monitors_and_fallback_scale() {
        let mons = mixed_dpi_mons();
        let a = saved_anchor_on_monitors(
            SavedPos::Anchor { x: 2195.0, y: 880.0 },
            &mons,
            9.0,
        );
        almost(a.0, 2195.0);
        almost(a.1, 880.0);
        let b = saved_anchor_on_monitors(SavedPos::Anchor { x: 1.0, y: 2.0 }, &[], 3.0);
        almost(b.0, 1.0);
        almost(b.1, 2.0);
    }
}
