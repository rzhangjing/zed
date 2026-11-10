//! The map's native window (Windows only).
//!
//! gpui has no custom wgpu surface on Windows (`gpui::PaintSurface` only takes a macOS
//! `CVPixelBuffer`), and it does not hand out its own device, so the GPU map cannot draw inside
//! gpui's render pass. It draws into a window of its own instead: the window owns a wgpu surface
//! and presents every frame itself, while gpui keeps the frame, the buttons and the status line
//! around it.
//!
//! The map window is a **popup owned by the Zed window**, not a child window: gpui presents its
//! window through DirectComposition, whose visuals are composited above HWND children — a child
//! window would be invisible behind the gpui frame. An owned `WS_POPUP` sits above its owner, so
//! the GPU frame shows, and gpui still owns the rest of the window.
//!
//! The map is transparent to the mouse: `WM_NCHITTEST` answers `HTTRANSPARENT`, so every click,
//! drag and wheel event goes to the Zed window beneath (both windows are on the same UI thread,
//! which is what `HTTRANSPARENT` requires), and the window's own input handling is unchanged.
//!
//! Create, move and destroy it on the UI thread — `HTTRANSPARENT` only passes through to windows
//! of the same thread. The HWND value is `Send`, so the engine thread creates the wgpu surface
//! and presents from there; see `crate::gpu::GpuMap`.

#![cfg(target_os = "windows")]

use std::num::NonZeroIsize;
use std::sync::OnceLock;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, Win32WindowHandle, WindowHandle, WindowsDisplayHandle,
};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetWindowPos, CS_HREDRAW,
    CS_VREDRAW, HTTRANSPARENT, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, WM_ERASEBKGND,
    WM_NCHITTEST, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    WS_VISIBLE,
};

/// The child window class, registered once per process.
const CLASS_NAME: PCWSTR = w!("RustGisMapChild");

/// A native window covering the map area of a host window, owned by it.
///
/// Dropping it destroys the window; do that on the thread that created it (the UI thread).
pub struct MapWindow {
    hwnd: HWND,
}

/// The map window: same API as before the popup change, kept under the old name for the host.
pub type MapChildWindow = MapWindow;

impl MapWindow {
    /// Create the map window as a click-through, non-activating popup owned by `owner` (an HWND
    /// as `isize`), at the screen rectangle (`x`, `y`, `width`, `height`) in physical pixels.
    pub fn create(owner: isize, x: i32, y: i32, width: i32, height: i32) -> Result<Self, String> {
        if owner == 0 {
            return Err("the host window has no HWND".to_string());
        }
        let hinstance = install_class()?;
        let owner = HWND(owner as *mut core::ffi::c_void);
        // SAFETY: `owner` is a live HWND of this thread (the caller passes gpui's), the class is
        // registered, and the rectangle is in physical pixels.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!(""),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                width.max(1),
                height.max(1),
                Some(owner),
                None,
                Some(hinstance),
                None,
            )
        }
        .map_err(|error| format!("could not create the map window: {error}"))?;
        log::info!(
            "[rustgis] map window {hwnd:?} {width}x{height} at ({x}, {y}), owned by {owner:?}"
        );
        Ok(MapWindow { hwnd })
    }

    /// The HWND as `isize`: `Send`, for the engine thread's wgpu surface.
    pub fn hwnd(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// Move and resize the map window, in screen pixels. `SWP_SHOWWINDOW` makes it visible again
    /// after a hide (a GPU failure that recovered).
    pub fn set_bounds(&self, x: i32, y: i32, width: i32, height: i32) -> Result<(), String> {
        // SAFETY: `self.hwnd` is this struct's live window; the flags keep it out of the
        // activation and z-order management (it stays above its owner).
        unsafe {
            SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                width.max(1),
                height.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        }
        .map_err(|error| format!("could not move the map window: {error}"))
    }
}

impl Drop for MapWindow {
    fn drop(&mut self) {
        // SAFETY: the window was created by this thread and is destroyed once (the struct owns
        // it). A failure means it is already gone, which is what we want anyway.
        unsafe {
            if let Err(error) = DestroyWindow(self.hwnd) {
                log::warn!("[rustgis] could not destroy the map window: {error}");
            }
        }
    }
}

/// The screen position of `hwnd`'s client area origin, in physical pixels.
pub fn client_origin(hwnd: isize) -> Option<(i32, i32)> {
    if hwnd == 0 {
        return None;
    }
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `hwnd` is the host window, alive for the duration of this call.
    if unsafe { ClientToScreen(HWND(hwnd as *mut core::ffi::c_void), &mut point) }.as_bool() {
        Some((point.x, point.y))
    } else {
        None
    }
}

/// A borrowed `HasWindowHandle` for the HWND value, for `wgpu`'s
/// `create_surface_unsafe(SurfaceTargetUnsafe::from_window(..))` on the engine thread.
pub struct MapWindowHandle(pub isize);

impl HasWindowHandle for MapWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let Some(hwnd) = NonZeroIsize::new(self.0) else {
            return Err(HandleError::Unavailable);
        };
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
        // SAFETY: the HWND is a live window of the process; the returned handle only borrows it.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// wgpu 30 asks for a display handle beside the window one on some native paths ("No
/// `DisplayHandle` is available" otherwise): Windows' is a unit value.
impl HasDisplayHandle for MapWindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: a Windows display handle carries no borrowed data (it is a unit marker).
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Windows(WindowsDisplayHandle::new())) })
    }
}

/// Register the window class once, returning the module instance the window is created with.
fn install_class() -> Result<HINSTANCE, String> {
    static INSTANCE: OnceLock<Result<isize, String>> = OnceLock::new();
    let installed = INSTANCE.get_or_init(|| {
        // SAFETY: `GetModuleHandleW(None)` asks for this process's module; no pointer is kept.
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| format!("could not get the process module: {error}"))?;
        let hinstance = HINSTANCE(module.0);
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // SAFETY: `class` outlives the call; the class name is a `'static` wide string.
        let atom = unsafe { RegisterClassW(&class) };
        if atom == 0 {
            // A second copy of zed in the same process would already have registered it.
            let error = windows::core::Error::from_thread();
            log::debug!("[rustgis] map child class registration returned 0: {error}");
        }
        Ok(module.0 as isize)
    });
    match installed {
        Ok(value) => Ok(HINSTANCE(*value as *mut core::ffi::c_void)),
        Err(error) => Err(error.clone()),
    }
}

/// The child's window procedure: click-through, never painted by GDI (wgpu presents into it).
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The mouse belongs to the parent: gpui's hit test, drag and wheel handling stay as they
        // are, even though the map's child window covers that part of the client area.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        // wgpu paints every pixel: no GDI background, no flicker.
        WM_ERASEBKGND => LRESULT(1),
        _ => {
            // SAFETY: pass every other message to the default procedure.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}
