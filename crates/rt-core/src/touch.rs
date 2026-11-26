use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchEvent {
    pub id: u64,
    pub phase: TouchPhase,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TouchGesture {
    // Portrait Pane Swipes
    TopPaneSwipeLeft,
    TopPaneSwipeRight,
    TopPaneSwipeUp,
    TopPaneSwipeDown,
    BottomPaneSwipeLeft,
    BottomPaneSwipeRight,
    BottomPaneSwipeUp,
    BottomPaneSwipeDown,

    // Landscape Swipes
    LandscapeSwipeLeft,
    LandscapeSwipeRight,
    LandscapeSwipeUp,
    LandscapeSwipeDown,

    // Universal Gestures
    SingleTap { x: f32, y: f32 },
    DoubleTap { x: f32, y: f32 },
    TwoFingerTap,
    TwoFingerDoubleTap,
    ThreeFingerTap,
    Scrub { pct: f32 },
    ScrubEnd,
    ScrubCancel,
}

pub struct TouchGestureController {
    // Single touch tracking
    touch_start_pos: Option<(f32, f32)>,
    touch_start_time: Option<Instant>,
    current_pos: Option<(f32, f32)>,
    is_scrubbing: bool,
    last_tap_time: Option<Instant>,
    last_tap_pos: Option<(f32, f32)>,
    pending_single_tap: Option<((f32, f32), Instant)>,
    scale_factor: f32,

    // Multi-touch tracking
    active_touches: HashMap<u64, (f32, f32, Instant)>,
    max_touch_count: usize,
    multi_finger_start_time: Option<Instant>,
}

impl Default for TouchGestureController {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchGestureController {
    pub fn new() -> Self {
        Self {
            touch_start_pos: None,
            touch_start_time: None,
            current_pos: None,
            is_scrubbing: false,
            last_tap_time: None,
            last_tap_pos: None,
            pending_single_tap: None,
            scale_factor: 1.0,
            active_touches: HashMap::new(),
            max_touch_count: 0,
            multi_finger_start_time: None,
        }
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        if scale_factor.is_finite() && scale_factor > 0.0 {
            self.scale_factor = scale_factor;
        }
    }

    pub fn handle_touch(
        &mut self,
        touch: &TouchEvent,
        window_width: f32,
        window_height: f32,
        is_portrait: bool,
        split_y: f32,
    ) -> Option<TouchGesture> {
        self.handle_touch_at(
            touch,
            window_width,
            window_height,
            is_portrait,
            split_y,
            Instant::now(),
        )
    }

    pub(crate) fn handle_touch_at(
        &mut self,
        touch: &TouchEvent,
        window_width: f32,
        window_height: f32,
        is_portrait: bool,
        split_y: f32,
        now: Instant,
    ) -> Option<TouchGesture> {
        let x = touch.x as f32;
        let y = touch.y as f32;
        if !x.is_finite()
            || !y.is_finite()
            || !window_width.is_finite()
            || !window_height.is_finite()
            || !split_y.is_finite()
        {
            return None;
        }
        let sf = self.scale_factor.max(1.0);
        let swipe_min_px = 45.0 * sf;
        let tap_max_move_px = 24.0 * sf;
        let double_tap_max_dist_px = 42.0 * sf;
        let scrub_start_px = 8.0 * sf;

        match touch.phase {
            TouchPhase::Started => {
                // Evict stale pointers whose Ended/Cancelled event was lost (> 30s)
                self.active_touches.retain(|_, (_, _, t0)| {
                    now.saturating_duration_since(*t0) < Duration::from_secs(30)
                });
                self.active_touches.insert(touch.id, (x, y, now));
                let touch_count = self.active_touches.len();
                if touch_count > self.max_touch_count {
                    self.max_touch_count = touch_count;
                }

                if touch_count >= 2 {
                    if self.multi_finger_start_time.is_none() {
                        self.multi_finger_start_time = Some(now);
                    }
                    self.pending_single_tap = None;
                    let was_scrubbing = self.is_scrubbing;
                    self.is_scrubbing = false;
                    if was_scrubbing {
                        return Some(TouchGesture::ScrubCancel);
                    }
                } else if touch_count == 1 {
                    self.touch_start_pos = Some((x, y));
                    self.touch_start_time = Some(now);
                    self.current_pos = Some((x, y));
                    self.is_scrubbing = false;
                }
                None
            }
            TouchPhase::Moved => {
                let entry = self.active_touches.get_mut(&touch.id)?;
                entry.0 = x;
                entry.1 = y;
                self.current_pos = Some((x, y));

                if self.max_touch_count == 1
                    && window_width > 0.0
                    && window_height > 0.0
                    && let Some((start_x, start_y)) = self.touch_start_pos
                {
                    if !self.is_scrubbing && start_y >= window_height * 0.88 {
                        let dx = (x - start_x).abs();
                        let dy = (y - start_y).abs();
                        if dx >= scrub_start_px && dx >= dy {
                            self.is_scrubbing = true;
                            self.pending_single_tap = None;
                        }
                    }
                    if self.is_scrubbing {
                        let pct = (x / window_width).clamp(0.0, 1.0);
                        return Some(TouchGesture::Scrub { pct });
                    }
                }
                None
            }
            TouchPhase::Ended => {
                self.active_touches.remove(&touch.id)?;
                let remaining = self.active_touches.len();

                if self.is_scrubbing {
                    self.is_scrubbing = false;
                    self.touch_start_pos = None;
                    self.touch_start_time = None;
                    self.max_touch_count = remaining;
                    return Some(TouchGesture::ScrubEnd);
                }

                // Check if a multi-finger (2-finger or 3-finger) touch gesture has concluded (all fingers lifted)
                if self.max_touch_count >= 2 {
                    if remaining == 0 {
                        let count = self.max_touch_count;
                        let dt = self
                            .multi_finger_start_time
                            .map(|t| now.saturating_duration_since(t).as_secs_f32())
                            .unwrap_or(1.0);
                        self.max_touch_count = 0;
                        self.multi_finger_start_time = None;
                        self.touch_start_pos = None;
                        self.touch_start_time = None;
                        self.pending_single_tap = None;

                        // Immediate responsive multi-finger tap (< 500ms)
                        if dt < 0.50 {
                            if count >= 3 {
                                return Some(TouchGesture::ThreeFingerTap);
                            } else {
                                return Some(TouchGesture::TwoFingerTap);
                            }
                        }
                    }
                    return None;
                }

                self.max_touch_count = remaining;
                if remaining == 0 {
                    self.multi_finger_start_time = None;
                }
                let (Some((start_x, start_y)), Some(start_time)) =
                    (self.touch_start_pos, self.touch_start_time)
                else {
                    return None;
                };

                let dx = x - start_x;
                let dy = y - start_y;
                let dist = (dx * dx + dy * dy).sqrt();
                let dt = now.saturating_duration_since(start_time).as_secs_f32();

                self.touch_start_pos = None;
                self.touch_start_time = None;

                // Check for fast swipe gestures (dx or dy > 45dp in < 600ms)
                if dt < 0.6 {
                    if dx.abs() > swipe_min_px && dx.abs() > dy.abs() * 1.2 {
                        if is_portrait {
                            let in_top_pane = start_y < split_y;
                            if in_top_pane {
                                if dx < 0.0 {
                                    return Some(TouchGesture::TopPaneSwipeLeft);
                                } else {
                                    return Some(TouchGesture::TopPaneSwipeRight);
                                }
                            } else {
                                if dx < 0.0 {
                                    return Some(TouchGesture::BottomPaneSwipeLeft);
                                } else {
                                    return Some(TouchGesture::BottomPaneSwipeRight);
                                }
                            }
                        } else {
                            if dx < 0.0 {
                                return Some(TouchGesture::LandscapeSwipeLeft);
                            } else {
                                return Some(TouchGesture::LandscapeSwipeRight);
                            }
                        }
                    } else if dy.abs() > swipe_min_px && dy.abs() > dx.abs() * 1.2 {
                        if is_portrait {
                            let in_top_pane = start_y < split_y;
                            if in_top_pane {
                                if dy < 0.0 {
                                    return Some(TouchGesture::TopPaneSwipeUp);
                                } else {
                                    return Some(TouchGesture::TopPaneSwipeDown);
                                }
                            } else {
                                if dy < 0.0 {
                                    return Some(TouchGesture::BottomPaneSwipeUp);
                                } else {
                                    return Some(TouchGesture::BottomPaneSwipeDown);
                                }
                            }
                        } else {
                            if dy < 0.0 {
                                return Some(TouchGesture::LandscapeSwipeUp);
                            } else {
                                return Some(TouchGesture::LandscapeSwipeDown);
                            }
                        }
                    }
                }

                // Check for tap gestures (moved < 24dp in < 280ms)
                if dist < tap_max_move_px && dt < 0.28 {
                    if let (Some(last_time), Some((lx, ly))) =
                        (self.last_tap_time, self.last_tap_pos)
                    {
                        let tap_dist = ((x - lx).powi(2) + (y - ly).powi(2)).sqrt();
                        if now.saturating_duration_since(last_time) < Duration::from_millis(280)
                            && tap_dist < double_tap_max_dist_px
                        {
                            self.last_tap_time = None;
                            self.last_tap_pos = None;
                            self.pending_single_tap = None;
                            return Some(TouchGesture::DoubleTap { x, y });
                        }
                    }

                    self.last_tap_time = Some(now);
                    self.last_tap_pos = Some((x, y));
                    self.pending_single_tap = Some(((x, y), now));
                }

                None
            }
            TouchPhase::Cancelled => {
                let was_scrubbing = self.is_scrubbing;
                self.active_touches.clear();
                self.max_touch_count = 0;
                self.multi_finger_start_time = None;
                self.touch_start_pos = None;
                self.touch_start_time = None;
                self.pending_single_tap = None;
                self.is_scrubbing = false;
                if was_scrubbing {
                    Some(TouchGesture::ScrubCancel)
                } else {
                    None
                }
            }
        }
    }

    /// Check if a pending single tap has expired
    pub fn update_pending_tap(&mut self) -> Option<TouchGesture> {
        self.update_pending_tap_at(Instant::now())
    }

    pub(crate) fn update_pending_tap_at(&mut self, now: Instant) -> Option<TouchGesture> {
        if let Some(((x, y), tap_time)) = self.pending_single_tap
            && now.saturating_duration_since(tap_time) >= Duration::from_millis(280)
        {
            self.pending_single_tap = None;
            return Some(TouchGesture::SingleTap { x, y });
        }
        None
    }

    /// Clear all in-flight touch state and pending delayed taps (e.g. when launching external picker or suspending)
    pub fn reset(&mut self) {
        self.touch_start_pos = None;
        self.touch_start_time = None;
        self.current_pos = None;
        self.is_scrubbing = false;
        self.last_tap_time = None;
        self.last_tap_pos = None;
        self.pending_single_tap = None;
        self.active_touches.clear();
        self.max_touch_count = 0;
        self.multi_finger_start_time = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_touch(phase: TouchPhase, id: u64, x: f64, y: f64) -> TouchEvent {
        TouchEvent { id, phase, x, y }
    }

    #[test]
    fn test_two_finger_tap() {
        let mut controller = TouchGestureController::new();

        // 2-finger tap
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 300.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 1, 600.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Ended, 0, 300.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        let g1 = controller.handle_touch(
            &make_touch(TouchPhase::Ended, 1, 600.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(g1, Some(TouchGesture::TwoFingerTap));
    }

    #[test]
    fn test_three_finger_tap() {
        let mut controller = TouchGestureController::new();

        // 3-finger tap
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 200.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 1, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 2, 800.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Ended, 0, 200.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        controller.handle_touch(
            &make_touch(TouchPhase::Ended, 1, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        let g = controller.handle_touch(
            &make_touch(TouchPhase::Ended, 2, 800.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(g, Some(TouchGesture::ThreeFingerTap));
    }

    #[test]
    fn test_top_pane_swipe_up_down() {
        let mut controller = TouchGestureController::new();

        // Swipe up in top pane (start y=500, end y=300 < split_y=1200)
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        let g = controller.handle_touch(
            &make_touch(TouchPhase::Ended, 0, 500.0, 300.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(g, Some(TouchGesture::TopPaneSwipeUp));

        // Swipe down in top pane (start y=300, end y=500 < split_y=1200)
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 500.0, 300.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        let g_down = controller.handle_touch(
            &make_touch(TouchPhase::Ended, 0, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(g_down, Some(TouchGesture::TopPaneSwipeDown));
    }

    #[test]
    fn test_high_dpi_double_tap_and_swipe() {
        let mut controller = TouchGestureController::new();
        controller.set_scale_factor(3.0);

        // On a 3.0x DPI screen, a 45px finger roll during a tap (~15dp) is still a tap,
        // and a second tap 90px away (~30dp) registers as a DoubleTap.
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 500.0, 1500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Ended, 0, 530.0, 1530.0),
                1080.0,
                2400.0,
                true,
                1200.0,
            ),
            None
        );

        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 590.0, 1530.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        let dbl = controller.handle_touch(
            &make_touch(TouchPhase::Ended, 0, 595.0, 1535.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(
            dbl,
            Some(TouchGesture::DoubleTap {
                x: 595.0,
                y: 1535.0
            })
        );

        // At 3.0x DPI, swipe_min_px = 135px: 50px drag is NOT a swipe, 150px drag IS a swipe
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 550.0, 500.0),
                1080.0,
                2400.0,
                true,
                1200.0,
            ),
            None
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Ended, 0, 550.0, 500.0),
                1080.0,
                2400.0,
                true,
                1200.0,
            ),
            None
        );

        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 500.0, 500.0),
            1080.0,
            2400.0,
            true,
            1200.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Ended, 0, 650.0, 500.0),
                1080.0,
                2400.0,
                true,
                1200.0,
            ),
            Some(TouchGesture::TopPaneSwipeRight)
        );
    }

    #[test]
    fn test_bottom_edge_horizontal_scrub_and_end() {
        let mut controller = TouchGestureController::new();
        let w = 1000.0;
        let h = 2000.0;

        // Exact boundary y = 0.88 * h = 1760.0 enters scrub on horizontal move >= 8px
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Started, 0, 200.0, 1760.0),
                w,
                h,
                true,
                1000.0,
            ),
            None
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 250.0, 1762.0),
                w,
                h,
                true,
                1000.0,
            ),
            Some(TouchGesture::Scrub { pct: 0.25 })
        );
        // Clamping out-of-bounds drag coordinates to [0.0, 1.0]
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 1200.0, 1762.0),
                w,
                h,
                true,
                1000.0,
            ),
            Some(TouchGesture::Scrub { pct: 1.0 })
        );
        // Lifting finger emits ScrubEnd rather than BottomPaneSwipeRight
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Ended, 0, 1200.0, 1762.0),
                w,
                h,
                true,
                1000.0,
            ),
            Some(TouchGesture::ScrubEnd)
        );

        // Just above boundary (y = 1759.0 < 0.88 * 2000) remains a normal pane swipe
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 200.0, 1759.0),
            w,
            h,
            true,
            1000.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 300.0, 1759.0),
                w,
                h,
                true,
                1000.0,
            ),
            None
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Ended, 0, 300.0, 1759.0),
                w,
                h,
                true,
                1000.0,
            ),
            Some(TouchGesture::BottomPaneSwipeRight)
        );
    }

    #[test]
    fn test_bottom_edge_scrub_cancel_and_zero_window_guard() {
        let mut controller = TouchGestureController::new();

        // Zero window dimensions must never divide by zero or enter scrub
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 100.0, 500.0),
            0.0,
            0.0,
            true,
            0.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 200.0, 500.0),
                0.0,
                0.0,
                true,
                0.0,
            ),
            None
        );
        controller.reset();

        // Second finger touching down mid-scrub emits ScrubCancel immediately
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 100.0, 1900.0),
            1000.0,
            2000.0,
            true,
            1000.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 300.0, 1900.0),
                1000.0,
                2000.0,
                true,
                1000.0,
            ),
            Some(TouchGesture::Scrub { pct: 0.3 })
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Started, 1, 500.0, 1900.0),
                1000.0,
                2000.0,
                true,
                1000.0,
            ),
            Some(TouchGesture::ScrubCancel)
        );
        controller.reset();

        // Cancelled touch mid-scrub (e.g. Android OS navigation bar gesture) emits ScrubCancel
        controller.handle_touch(
            &make_touch(TouchPhase::Started, 0, 100.0, 1900.0),
            1000.0,
            2000.0,
            true,
            1000.0,
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Moved, 0, 400.0, 1900.0),
                1000.0,
                2000.0,
                true,
                1000.0,
            ),
            Some(TouchGesture::Scrub { pct: 0.4 })
        );
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Cancelled, 0, 400.0, 1900.0),
                1000.0,
                2000.0,
                true,
                1000.0,
            ),
            Some(TouchGesture::ScrubCancel)
        );
    }

    #[test]
    fn test_deterministic_tap_timeout_and_duration_thresholds() {
        let mut controller = TouchGestureController::new();
        let t0 = Instant::now();

        // Quick tap down/up queues a delayed SingleTap until >= 280ms
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Started, 0, 300.0, 400.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                t0,
            ),
            None
        );
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Ended, 0, 302.0, 401.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                t0 + Duration::from_millis(50),
            ),
            None
        );
        assert_eq!(
            controller.update_pending_tap_at(t0 + Duration::from_millis(250)),
            None
        );
        assert_eq!(
            controller.update_pending_tap_at(t0 + Duration::from_millis(330)),
            Some(TouchGesture::SingleTap { x: 302.0, y: 401.0 })
        );
        // Fires once
        assert_eq!(
            controller.update_pending_tap_at(t0 + Duration::from_millis(500)),
            None
        );

        // Slow press (>= 280ms) does not register as a tap
        let t1 = t0 + Duration::from_secs(2);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 0, 300.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t1,
        );
        controller.handle_touch_at(
            &make_touch(TouchPhase::Ended, 0, 300.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t1 + Duration::from_millis(300),
        );
        assert_eq!(
            controller.update_pending_tap_at(t1 + Duration::from_secs(1)),
            None
        );

        // Slow drag (>= 600ms) is not a swipe
        let t2 = t0 + Duration::from_secs(5);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 0, 100.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t2,
        );
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Ended, 0, 300.0, 400.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                t2 + Duration::from_millis(650),
            ),
            None
        );

        // Slow 2-finger hold (>= 500ms) is not a TwoFingerTap
        let t3 = t0 + Duration::from_secs(8);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 0, 100.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t3,
        );
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 1, 200.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t3 + Duration::from_millis(10),
        );
        controller.handle_touch_at(
            &make_touch(TouchPhase::Ended, 0, 100.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t3 + Duration::from_millis(550),
        );
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Ended, 1, 200.0, 400.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                t3 + Duration::from_millis(560),
            ),
            None
        );

        // Second tap far apart (> 42dp) is not a DoubleTap
        let t4 = t0 + Duration::from_secs(12);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 0, 100.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t4,
        );
        controller.handle_touch_at(
            &make_touch(TouchPhase::Ended, 0, 100.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t4 + Duration::from_millis(40),
        );
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 0, 200.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            t4 + Duration::from_millis(100),
        );
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Ended, 0, 200.0, 400.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                t4 + Duration::from_millis(140),
            ),
            None
        );
    }

    #[test]
    fn test_landscape_portrait_directions_nan_guards_and_stale_pointer_eviction() {
        let mut controller = TouchGestureController::new();
        controller.set_scale_factor(-2.0);
        controller.set_scale_factor(f32::NAN);

        // Non-finite inputs return None without poisoning state
        assert_eq!(
            controller.handle_touch(
                &make_touch(TouchPhase::Started, 0, f64::NAN, 100.0),
                1920.0,
                1080.0,
                false,
                540.0,
            ),
            None
        );

        let t0 = Instant::now();
        // Landscape 4-direction swipes
        let landscape_cases = [
            (
                (500.0, 400.0),
                (350.0, 400.0),
                TouchGesture::LandscapeSwipeLeft,
            ),
            (
                (500.0, 400.0),
                (650.0, 400.0),
                TouchGesture::LandscapeSwipeRight,
            ),
            (
                (500.0, 500.0),
                (500.0, 350.0),
                TouchGesture::LandscapeSwipeUp,
            ),
            (
                (500.0, 350.0),
                (500.0, 500.0),
                TouchGesture::LandscapeSwipeDown,
            ),
        ];
        for (idx, (start, end, expected)) in landscape_cases.into_iter().enumerate() {
            let base = t0 + Duration::from_secs(idx as u64 + 1);
            controller.handle_touch_at(
                &make_touch(TouchPhase::Started, 0, start.0, start.1),
                1920.0,
                1080.0,
                false,
                540.0,
                base,
            );
            assert_eq!(
                controller.handle_touch_at(
                    &make_touch(TouchPhase::Ended, 0, end.0, end.1),
                    1920.0,
                    1080.0,
                    false,
                    540.0,
                    base + Duration::from_millis(80),
                ),
                Some(expected)
            );
        }

        // Portrait bottom pane vertical swipes
        for (idx, (sy, ey, expected)) in [
            (1500.0, 1350.0, TouchGesture::BottomPaneSwipeUp),
            (1350.0, 1500.0, TouchGesture::BottomPaneSwipeDown),
        ]
        .into_iter()
        .enumerate()
        {
            let base = t0 + Duration::from_secs(10 + idx as u64);
            controller.handle_touch_at(
                &make_touch(TouchPhase::Started, 0, 400.0, sy),
                1080.0,
                2400.0,
                true,
                1200.0,
                base,
            );
            assert_eq!(
                controller.handle_touch_at(
                    &make_touch(TouchPhase::Ended, 0, 400.0, ey),
                    1080.0,
                    2400.0,
                    true,
                    1200.0,
                    base + Duration::from_millis(80),
                ),
                Some(expected)
            );
        }

        // Stale pointer eviction: pointer 0 started > 30s ago is evicted when pointer 1 starts
        let stale_start = t0 + Duration::from_secs(20);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 99, 200.0, 300.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            stale_start,
        );
        let fresh_start = stale_start + Duration::from_secs(35);
        controller.handle_touch_at(
            &make_touch(TouchPhase::Started, 1, 200.0, 400.0),
            1080.0,
            2400.0,
            true,
            1200.0,
            fresh_start,
        );
        assert_eq!(
            controller.handle_touch_at(
                &make_touch(TouchPhase::Ended, 1, 350.0, 400.0),
                1080.0,
                2400.0,
                true,
                1200.0,
                fresh_start + Duration::from_millis(60),
            ),
            Some(TouchGesture::TopPaneSwipeRight)
        );
    }
}
