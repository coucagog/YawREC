// ============================================================
// YAWREC · capture/window.rs
// Énumération des fenêtres visibles + coordonnées en temps réel.
// Windows only ; stubs no-op sur les autres plateformes.
//
// Notes sur les types windows 0.61 :
//   HWND  = *mut c_void  → converti en i64 via usize
//   LPARAM = isize
//   BOOL  = windows::core::BOOL (windows_core re-exporté)
// ============================================================

use serde::Serialize;

/// Fenêtre visible retournée par list_windows().
#[derive(Debug, Clone, Serialize)]
pub struct WindowInfo {
    pub hwnd: i64,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

// ============================================================
// Implémentation Windows
// ============================================================
#[cfg(target_os = "windows")]
mod platform {
    use super::WindowInfo;
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                IsWindowVisible, GWL_EXSTYLE, GWL_STYLE, WS_EX_TOOLWINDOW, WS_MINIMIZE,
            },
        },
    };

    /// Retourne toutes les fenêtres visibles, titrées, non-outil, non-minimisées,
    /// d'au moins 100×100 px.
    pub fn list_windows() -> Vec<WindowInfo> {
        let mut results: Vec<WindowInfo> = Vec::new();
        let ptr = &mut results as *mut Vec<WindowInfo> as isize;
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(ptr));
        }
        results
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = &mut *(lparam.0 as *mut Vec<WindowInfo>);

        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let text_len = GetWindowTextLengthW(hwnd);
        if text_len == 0 {
            return BOOL(1);
        }

        // Exclure les fenêtres-outils (ne pas apparaître dans l'alt-tab)
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }

        // Exclure les fenêtres minimisées (rect invalide)
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if style & WS_MINIMIZE.0 != 0 {
            return BOOL(1);
        }

        let mut buf = vec![0u16; (text_len + 1) as usize];
        let chars = GetWindowTextW(hwnd, &mut buf);
        if chars == 0 {
            return BOOL(1);
        }
        let name = String::from_utf16_lossy(&buf[..chars as usize]);

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return BOOL(1);
        }

        let w = (rect.right - rect.left) as u32;
        let h = (rect.bottom - rect.top) as u32;
        if w < 100 || h < 100 {
            return BOOL(1);
        }

        // HWND est *mut c_void dans windows 0.61 → on passe par usize pour i64
        list.push(WindowInfo {
            hwnd: hwnd.0 as usize as i64,
            name,
            width: w,
            height: h,
        });

        BOOL(1) // continuer l'énumération
    }

    /// Retourne (left, top, width, height) de la fenêtre en coordonnées bureau.
    /// Retourne None si la fenêtre n'existe plus ou est inaccessible.
    pub fn get_window_rect(hwnd: i64) -> Option<(i32, i32, u32, u32)> {
        let handle = HWND(hwnd as usize as *mut core::ffi::c_void);
        let mut rect = RECT::default();
        unsafe {
            if GetWindowRect(handle, &mut rect).is_ok() {
                let w = (rect.right - rect.left) as u32;
                let h = (rect.bottom - rect.top) as u32;
                Some((rect.left, rect.top, w, h))
            } else {
                None
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use platform::{get_window_rect, list_windows};

// ============================================================
// Stubs non-Windows
// ============================================================
#[cfg(not(target_os = "windows"))]
pub fn list_windows() -> Vec<WindowInfo> { vec![] }

#[cfg(not(target_os = "windows"))]
pub fn get_window_rect(_hwnd: i64) -> Option<(i32, i32, u32, u32)> { None }
