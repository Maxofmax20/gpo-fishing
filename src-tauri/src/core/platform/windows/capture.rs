use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::core::platform::capture_bounds::{contained_offset, CaptureLayout};
use crate::core::platform::{Capture, PlatformError, Result};
use crate::core::types::{Frame, PxRect};

pub struct GdiCapture;

impl GdiCapture {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GdiCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture for GdiCapture {
    fn grab(&self, rect: PxRect) -> Result<Frame> {
        let layout = CaptureLayout::new(rect.x, rect.y, rect.w, rect.h)
            .map_err(|error| PlatformError::Capture(error.into()))?;
        let w = rect.w;
        let h = rect.h;
        unsafe {
            let desktop_origin = (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            );
            let desktop_size = (
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            );
            if contained_offset(
                (rect.x, rect.y, rect.w, rect.h),
                desktop_origin,
                desktop_size,
            )
            .is_none()
            {
                return Err(PlatformError::Capture(
                    "capture rectangle crosses the virtual desktop bounds".into(),
                ));
            }
            let mut buf = layout
                .allocate()
                .map_err(|error| PlatformError::Capture(error.into()))?;
            let screen = GetDC(Some(HWND::default()));
            if screen.is_invalid() {
                return Err(PlatformError::Capture("GetDC".into()));
            }
            let mem = CreateCompatibleDC(Some(screen));
            if mem.is_invalid() {
                ReleaseDC(Some(HWND::default()), screen);
                return Err(PlatformError::Capture("CreateCompatibleDC".into()));
            }
            let bmp = CreateCompatibleBitmap(screen, w, h);
            if bmp.is_invalid() {
                let _ = DeleteDC(mem);
                ReleaseDC(Some(HWND::default()), screen);
                return Err(PlatformError::Capture("CreateCompatibleBitmap".into()));
            }
            let old = SelectObject(mem, bmp.into());
            if old.is_invalid() || old.0 as isize == -1 {
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                ReleaseDC(Some(HWND::default()), screen);
                return Err(PlatformError::Capture("SelectObject".into()));
            }
            let ok = BitBlt(mem, 0, 0, w, h, Some(screen), rect.x, rect.y, SRCCOPY).is_ok();

            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
                Some(buf.as_mut_ptr() as *mut _),
                &mut info,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            ReleaseDC(Some(HWND::default()), screen);

            if !ok || lines != h {
                return Err(PlatformError::Capture("BitBlt/GetDIBits".into()));
            }
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            Ok(Frame::new(layout.width, layout.height, buf))
        }
    }
}
