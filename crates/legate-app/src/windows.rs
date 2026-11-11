//! Pure window placement from `src/main/windows.ts`.
//!
//! Opening `BrowserWindow`s is the Tauri shell's job. These helpers are the
//! decisions that do not touch a window object. Terminal resize ownership lives
//! in `legate_pty` (`PtyHost::claim_terminal` / `resize_claimed`).

/// Most windows the app will open, including the first one.
pub const MAX_WINDOWS: usize = 6;
/// Shown when a seventh window is refused.
pub const WINDOW_CAP_MESSAGE: &str = "Six windows are open. Close one first.";
/// How far a new window sits down and right of the one it came from.
pub const CASCADE_OFFSET: i64 = 32;

/// A rectangle in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// A window's requested size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: i64,
    pub height: i64,
}

/// `None` when a window may open; the refusal message when `open_count` is already at the cap.
pub fn refuse_window(open_count: usize) -> Option<&'static str> {
    if open_count >= MAX_WINDOWS {
        Some(WINDOW_CAP_MESSAGE)
    } else {
        None
    }
}

/// Place a window 32px down-right of `from`, at `size`, kept inside `work_area`.
pub fn cascade_bounds(from: Bounds, size: Size, work_area: Bounds) -> Bounds {
    let width = clamp(size.width, 1, work_area.width.max(1));
    let height = clamp(size.height, 1, work_area.height.max(1));
    let max_x = work_area.x + work_area.width - width;
    let max_y = work_area.y + work_area.height - height;
    Bounds {
        x: clamp(from.x + CASCADE_OFFSET, work_area.x, max_x),
        y: clamp(from.y + CASCADE_OFFSET, work_area.y, max_y),
        width,
        height,
    }
}

/// Terminal ids that currently have a window of their own.
#[derive(Debug, Default, Clone)]
pub struct PoppedOutRegistry {
    ids: Vec<String>,
}

impl PoppedOutRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `id`. False when it was already open in its own window.
    pub fn add(&mut self, id: impl Into<String>) -> bool {
        let id = id.into();
        if self.ids.iter().any(|existing| existing == &id) {
            return false;
        }
        self.ids.push(id);
        true
    }

    /// Forget `id`. Missing ids are ignored.
    pub fn remove(&mut self, id: &str) {
        self.ids.retain(|existing| existing != id);
    }

    /// True when `id` has its own window.
    pub fn has(&self, id: &str) -> bool {
        self.ids.iter().any(|existing| existing == id)
    }

    /// Ids in the order they were opened.
    pub fn list(&self) -> Vec<String> {
        self.ids.clone()
    }
}

fn clamp(value: i64, min: i64, max: i64) -> i64 {
    if max < min {
        return min;
    }
    value.clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::{
        Bounds, CASCADE_OFFSET, MAX_WINDOWS, PoppedOutRegistry, Size, WINDOW_CAP_MESSAGE,
        cascade_bounds, refuse_window,
    };

    #[test]
    fn refuse_window_allows_up_to_six_and_names_the_seventh() {
        assert_eq!(refuse_window(0), None);
        assert_eq!(refuse_window(MAX_WINDOWS - 1), None);
        assert_eq!(refuse_window(MAX_WINDOWS), Some(WINDOW_CAP_MESSAGE));
        assert_eq!(refuse_window(MAX_WINDOWS + 1), Some(WINDOW_CAP_MESSAGE));
    }

    #[test]
    fn cascade_bounds_is_32px_down_right_at_the_asked_size() {
        assert_eq!(
            cascade_bounds(
                Bounds {
                    x: 40,
                    y: 80,
                    width: 1200,
                    height: 700
                },
                Size {
                    width: 1200,
                    height: 700
                },
                Bounds {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080
                }
            ),
            Bounds {
                x: 40 + CASCADE_OFFSET,
                y: 80 + CASCADE_OFFSET,
                width: 1200,
                height: 700
            }
        );
    }

    #[test]
    fn cascade_bounds_keeps_the_window_inside_the_work_area() {
        assert_eq!(
            cascade_bounds(
                Bounds {
                    x: 100,
                    y: 100,
                    width: 1400,
                    height: 820
                },
                Size {
                    width: 1400,
                    height: 820
                },
                Bounds {
                    x: 0,
                    y: 0,
                    width: 1500,
                    height: 900
                }
            ),
            Bounds {
                x: 100,
                y: 80,
                width: 1400,
                height: 820
            }
        );
    }

    #[test]
    fn cascade_bounds_shrinks_to_the_work_area_and_stays_on_it() {
        assert_eq!(
            cascade_bounds(
                Bounds {
                    x: 120,
                    y: 60,
                    width: 900,
                    height: 600
                },
                Size {
                    width: 900,
                    height: 600
                },
                Bounds {
                    x: 100,
                    y: 50,
                    width: 800,
                    height: 500
                }
            ),
            Bounds {
                x: 100,
                y: 50,
                width: 800,
                height: 500
            }
        );
    }

    #[test]
    fn popped_out_registry_adds_and_refuses_a_second_window() {
        let mut popped = PoppedOutRegistry::new();
        assert!(popped.add("a"));
        assert!(!popped.add("a"));
        assert!(popped.has("a"));
        assert_eq!(popped.list(), vec!["a".to_string()]);
        assert!(popped.add("b"));
        assert_eq!(popped.list(), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn popped_out_registry_removes_an_id() {
        let mut popped = PoppedOutRegistry::new();
        popped.add("a");
        popped.add("b");
        popped.remove("a");
        assert!(!popped.has("a"));
        assert_eq!(popped.list(), vec!["b".to_string()]);
        popped.remove("missing");
        assert_eq!(popped.list(), vec!["b".to_string()]);
        assert!(popped.add("a"));
    }
}
