use std::collections::HashMap;
use std::time::{Duration, Instant};
use winit::event::{Touch, TouchPhase};

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
    Scrub { pct: f32 },
    ScrubEnd,
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

    // Multi-touch tracking
    active_touches: HashMap<u64, (f32, f32, Instant)>,
    max_touch_count: usize,
    two_finger_start_time: Option<Instant>,
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
            active_touches: HashMap::new(),
            max_touch_count: 0,
            two_finger_start_time: None,
        }
    }

    pub fn handle_touch(
        &mut self,
        touch: &Touch,
        window_width: f32,
        window_height: f32,
        is_portrait: bool,
        split_y: f32,
    ) -> Option<TouchGesture> {
        let x = touch.location.x as f32;
        let y = touch.location.y as f32;

        match touch.phase {
            TouchPhase::Started => {
                self.active_touches.insert(touch.id, (x, y, Instant::now()));
                let touch_count = self.active_touches.len();
                if touch_count > self.max_touch_count {
                    self.max_touch_count = touch_count;
                }

                if touch_count == 2 {
                    self.two_finger_start_time = Some(Instant::now());
                    self.pending_single_tap = None;
                    self.is_scrubbing = false;
                } else if touch_count == 1 {
                    self.touch_start_pos = Some((x, y));
                    self.touch_start_time = Some(Instant::now());
                    self.current_pos = Some((x, y));

                    // Bottom 12% zone is the timeline scrubber zone
                    if window_height > 0.0 && y > window_height * 0.88 {
                        self.is_scrubbing = true;
                        let pct = (x / window_width).clamp(0.0, 1.0);
                        return Some(TouchGesture::Scrub { pct });
                    } else {
                        self.is_scrubbing = false;
                    }
                }
                None
            }
            TouchPhase::Moved => {
                if let Some(entry) = self.active_touches.get_mut(&touch.id) {
                    entry.0 = x;
                    entry.1 = y;
                }
                self.current_pos = Some((x, y));
                if self.is_scrubbing && self.max_touch_count == 1 && window_width > 0.0 {
                    let pct = (x / window_width).clamp(0.0, 1.0);
                    return Some(TouchGesture::Scrub { pct });
                }
                None
            }
            TouchPhase::Ended => {
                self.active_touches.remove(&touch.id);
                let remaining = self.active_touches.len();

                if self.is_scrubbing && self.max_touch_count == 1 {
                    self.is_scrubbing = false;
                    self.touch_start_pos = None;
                    self.touch_start_time = None;
                    self.max_touch_count = remaining;
                    return Some(TouchGesture::ScrubEnd);
                }

                // Check if a 2-finger touch gesture has concluded (all fingers lifted)
                if self.max_touch_count >= 2 {
                    if remaining == 0 {
                        let dt = self.two_finger_start_time.map(|t| t.elapsed().as_secs_f32()).unwrap_or(1.0);
                        self.max_touch_count = 0;
                        self.two_finger_start_time = None;
                        self.touch_start_pos = None;
                        self.touch_start_time = None;
                        self.pending_single_tap = None;

                        // Immediate responsive 2-finger tap (< 500ms)
                        if dt < 0.50 {
                            return Some(TouchGesture::TwoFingerTap);
                        }
                    }
                    return None;
                }

                self.max_touch_count = remaining;
                let (Some((start_x, start_y)), Some(start_time)) = (self.touch_start_pos, self.touch_start_time) else {
                    return None;
                };

                let dx = x - start_x;
                let dy = y - start_y;
                let dist = (dx * dx + dy * dy).sqrt();
                let dt = start_time.elapsed().as_secs_f32();

                self.touch_start_pos = None;
                self.touch_start_time = None;

                // Check for fast swipe gestures (dx or dy > 50px in < 600ms)
                if dt < 0.6 {
                    if dx.abs() > 50.0 && dx.abs() > dy.abs() * 1.2 {
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
                    } else if dy.abs() > 50.0 && dy.abs() > dx.abs() * 1.2 {
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

                // Check for tap gestures (moved < 25px in < 250ms)
                if dist < 25.0 && dt < 0.25 {
                    let now = Instant::now();
                    if let (Some(last_time), Some((lx, ly))) = (self.last_tap_time, self.last_tap_pos) {
                        let tap_dist = ((x - lx).powi(2) + (y - ly).powi(2)).sqrt();
                        if now.duration_since(last_time) < Duration::from_millis(250) && tap_dist < 35.0 {
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
                self.active_touches.clear();
                self.max_touch_count = 0;
                self.two_finger_start_time = None;
                self.touch_start_pos = None;
                self.touch_start_time = None;
                self.is_scrubbing = false;
                None
            }
        }
    }

    /// Check if a pending single tap has expired
    pub fn update_pending_tap(&mut self) -> Option<TouchGesture> {
        if let Some(((x, y), tap_time)) = self.pending_single_tap {
            if tap_time.elapsed() >= Duration::from_millis(250) {
                self.pending_single_tap = None;
                return Some(TouchGesture::SingleTap { x, y });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;

    fn make_touch(phase: TouchPhase, id: u64, x: f64, y: f64) -> Touch {
        Touch {
            device_id: unsafe { std::mem::zeroed() },
            phase,
            location: PhysicalPosition::new(x, y),
            force: None,
            id,
        }
    }

    #[test]
    fn test_two_finger_tap() {
        let mut controller = TouchGestureController::new();

        // 2-finger tap
        controller.handle_touch(&make_touch(TouchPhase::Started, 0, 300.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        controller.handle_touch(&make_touch(TouchPhase::Started, 1, 600.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        controller.handle_touch(&make_touch(TouchPhase::Ended, 0, 300.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        let g1 = controller.handle_touch(&make_touch(TouchPhase::Ended, 1, 600.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        assert_eq!(g1, Some(TouchGesture::TwoFingerTap));
    }

    #[test]
    fn test_top_pane_swipe_up_down() {
        let mut controller = TouchGestureController::new();

        // Swipe up in top pane (start y=500, end y=300 < split_y=1200)
        controller.handle_touch(&make_touch(TouchPhase::Started, 0, 500.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        let g = controller.handle_touch(&make_touch(TouchPhase::Ended, 0, 500.0, 300.0), 1080.0, 2400.0, true, 1200.0);
        assert_eq!(g, Some(TouchGesture::TopPaneSwipeUp));

        // Swipe down in top pane (start y=300, end y=500 < split_y=1200)
        controller.handle_touch(&make_touch(TouchPhase::Started, 0, 500.0, 300.0), 1080.0, 2400.0, true, 1200.0);
        let g_down = controller.handle_touch(&make_touch(TouchPhase::Ended, 0, 500.0, 500.0), 1080.0, 2400.0, true, 1200.0);
        assert_eq!(g_down, Some(TouchGesture::TopPaneSwipeDown));
    }
}
