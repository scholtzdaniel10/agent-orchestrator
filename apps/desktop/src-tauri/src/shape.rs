//! Shaped windows, as Winamp skins did it: the window is cut to a union of rectangles
//! (`SetWindowRgn`), so outside the shape there is no window at all, to see or to click.
//! The Crabd theme uses it for its silhouette; every other theme clears it.

use serde::Deserialize;

/// A rectangle in physical pixels from the window's client area's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ShapeRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The rectangles worth sending to the system: the kept ones with any size, and the cut ones
/// (holes) that overlap something kept. Nothing kept means "no shape".
pub fn usable(keep: &[ShapeRect], cut: &[ShapeRect]) -> (Vec<ShapeRect>, Vec<ShapeRect>) {
    let keep: Vec<ShapeRect> = keep
        .iter()
        .copied()
        .filter(|rect| rect.w > 0 && rect.h > 0)
        .collect();
    let overlaps = |a: &ShapeRect, b: &ShapeRect| {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    };
    let cut = cut
        .iter()
        .copied()
        .filter(|rect| rect.w > 0 && rect.h > 0 && keep.iter().any(|kept| overlaps(kept, rect)))
        .collect();
    (keep, cut)
}

/// Cuts the window to `keep` minus `cut`; an empty `keep` gives back the plain rectangle.
#[cfg(windows)]
pub fn apply(window: &tauri::Window, keep: &[ShapeRect], cut: &[ShapeRect]) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, CombineRgn, CreateRectRgn, DeleteObject, HGDIOBJ, HRGN, RGN_DIFF, RGN_OR,
        SetWindowRgn,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let (keep, cut) = usable(keep, cut);
    let raw = window.hwnd().map_err(|err| err.to_string())?.0 as isize;
    let hwnd = HWND(raw as _);
    // SAFETY: plain GDI and window calls on this window's own handle. The region handed to
    // SetWindowRgn belongs to the system afterwards; every other region is deleted here.
    unsafe {
        if keep.is_empty() {
            SetWindowRgn(hwnd, None, true);
            return Ok(());
        }
        // Regions count from the window's corner; the page counts from the client area's.
        let mut outer = RECT::default();
        GetWindowRect(hwnd, &mut outer).map_err(|err| err.to_string())?;
        let mut origin = POINT::default();
        let _ = ClientToScreen(hwnd, &mut origin);
        let (dx, dy) = (origin.x - outer.left, origin.y - outer.top);
        let region = |rect: &ShapeRect| {
            CreateRectRgn(
                rect.x + dx,
                rect.y + dy,
                rect.x + dx + rect.w,
                rect.y + dy + rect.h,
            )
        };
        let shape: HRGN = CreateRectRgn(0, 0, 0, 0);
        for rect in &keep {
            let part = region(rect);
            CombineRgn(Some(shape), Some(shape), Some(part), RGN_OR);
            let _ = DeleteObject(HGDIOBJ(part.0));
        }
        for rect in &cut {
            let part = region(rect);
            CombineRgn(Some(shape), Some(shape), Some(part), RGN_DIFF);
            let _ = DeleteObject(HGDIOBJ(part.0));
        }
        if SetWindowRgn(hwnd, Some(shape), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(shape.0));
            return Err("Windows refused the window shape.".into());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn apply(
    _window: &tauri::Window,
    _keep: &[ShapeRect],
    _cut: &[ShapeRect],
) -> Result<(), String> {
    Ok(())
}

/// The `WMSZ_*` code for a compass edge (`n`, `se`, ...) of Legate's own resize grips.
pub fn size_edge(edge: &str) -> Option<usize> {
    Some(match edge {
        "w" => 1,
        "e" => 2,
        "n" => 3,
        "nw" => 4,
        "ne" => 5,
        "s" => 6,
        "sw" => 7,
        "se" => 8,
        _ => return None,
    })
}

/// Starts the system's own resize from `edge`, following the mouse from where it is: a
/// shaped window has no borders of its own to grab. Runs on the window's thread.
#[cfg(windows)]
pub fn start_resize(window: &tauri::Window, edge: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SC_SIZE, WM_SYSCOMMAND};

    let code = size_edge(edge).ok_or_else(|| format!("unknown edge: {edge}"))?;
    let raw = window.hwnd().map_err(|err| err.to_string())?.0 as isize;
    let hwnd = HWND(raw as _);
    // SAFETY: plain window messages to this window's own handle.
    unsafe {
        let _ = ReleaseCapture();
        PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM(SC_SIZE as usize + code),
            LPARAM(0),
        )
        .map_err(|err| err.to_string())
    }
}

#[cfg(not(windows))]
pub fn start_resize(_window: &tauri::Window, _edge: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ShapeRect, usable};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> ShapeRect {
        ShapeRect { x, y, w, h }
    }

    #[test]
    fn empty_parts_and_stray_holes_are_dropped() {
        let (keep, cut) = usable(
            &[rect(0, 0, 100, 100), rect(10, 10, 0, 50)],
            &[
                rect(20, 20, 10, 10),
                rect(500, 500, 10, 10),
                rect(30, 30, 10, 0),
            ],
        );
        assert_eq!(keep, vec![rect(0, 0, 100, 100)]);
        assert_eq!(cut, vec![rect(20, 20, 10, 10)]);
    }

    #[test]
    fn nothing_kept_means_no_shape() {
        let (keep, cut) = usable(&[], &[rect(0, 0, 10, 10)]);
        assert!(keep.is_empty());
        assert!(cut.is_empty());
    }

    #[test]
    fn every_grip_edge_has_its_system_code() {
        use super::size_edge;
        assert_eq!(size_edge("w"), Some(1));
        assert_eq!(size_edge("se"), Some(8));
        assert_eq!(size_edge("up"), None);
    }
}
