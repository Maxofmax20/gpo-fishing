use parking_lot::Mutex;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_DESC, DXGI_OUTDUPL_FRAME_INFO,
};

use crate::core::platform::capture_bounds::{contained_offset, mapped_rows, CaptureLayout};
use crate::core::platform::{Capture, PlatformError, Result};
use crate::core::types::{Frame, PxRect};

use super::capture::GdiCapture;

const FIRST_FRAME_TIMEOUT_MS: u32 = 250;

struct Session {
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    staging: ID3D11Texture2D,
    origin: (i32, i32),
    size: (i32, i32),
    has_frame: bool,
}

unsafe impl Send for Session {}

pub struct DxgiCapture {
    session: Mutex<Option<Session>>,
    fallback: GdiCapture,
}

impl DxgiCapture {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            fallback: GdiCapture::new(),
        }
    }

    fn open(rect: PxRect) -> std::result::Result<Session, String> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| e.to_string())?;
            let mut adapter_index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
                adapter_index += 1;
                let mut output_index = 0;
                while let Ok(output) = adapter.EnumOutputs(output_index) {
                    output_index += 1;
                    let Ok(desc) = output.GetDesc() else { continue };
                    let dc = desc.DesktopCoordinates;
                    let (Some(output_w), Some(output_h)) =
                        (dc.right.checked_sub(dc.left), dc.bottom.checked_sub(dc.top))
                    else {
                        continue;
                    };
                    if contained_offset(
                        (rect.x, rect.y, rect.w, rect.h),
                        (dc.left, dc.top),
                        (output_w, output_h),
                    )
                    .is_none()
                    {
                        continue;
                    }
                    let base: IDXGIAdapter = adapter.cast().map_err(|e| e.to_string())?;
                    let mut device: Option<ID3D11Device> = None;
                    let mut context: Option<ID3D11DeviceContext> = None;
                    D3D11CreateDevice(
                        Some(&base),
                        D3D_DRIVER_TYPE_UNKNOWN,
                        HMODULE::default(),
                        D3D11_CREATE_DEVICE_FLAG(0),
                        None,
                        D3D11_SDK_VERSION,
                        Some(&mut device),
                        None,
                        Some(&mut context),
                    )
                    .map_err(|e| e.to_string())?;
                    let (Some(device), Some(context)) = (device, context) else {
                        return Err("D3D11CreateDevice returned nothing".into());
                    };
                    let output1: IDXGIOutput1 = output.cast().map_err(|e| e.to_string())?;
                    let dup = output1
                        .DuplicateOutput(&device)
                        .map_err(|e| e.to_string())?;
                    let dd: DXGI_OUTDUPL_DESC = dup.GetDesc();
                    // Rotated duplication surfaces use a different coordinate space.
                    // Let GDI handle these until an explicit rotation transform exists.
                    if dd.Rotation != DXGI_MODE_ROTATION_IDENTITY {
                        return Err("rotated output requires GDI capture".into());
                    }
                    let w = i32::try_from(dd.ModeDesc.Width).map_err(|_| "DXGI width overflow")?;
                    let h =
                        i32::try_from(dd.ModeDesc.Height).map_err(|_| "DXGI height overflow")?;
                    CaptureLayout::new(dc.left, dc.top, w, h)?;
                    if contained_offset((rect.x, rect.y, rect.w, rect.h), (dc.left, dc.top), (w, h))
                        .is_none()
                    {
                        return Err("capture rectangle crosses the duplication surface".into());
                    }
                    let tex_desc = D3D11_TEXTURE2D_DESC {
                        Width: w as u32,
                        Height: h as u32,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                    };
                    let mut staging: Option<ID3D11Texture2D> = None;
                    device
                        .CreateTexture2D(&tex_desc, None, Some(&mut staging))
                        .map_err(|e| e.to_string())?;
                    let Some(staging) = staging else {
                        return Err("CreateTexture2D returned nothing".into());
                    };
                    return Ok(Session {
                        context,
                        dup,
                        staging,
                        origin: (dc.left, dc.top),
                        size: (w, h),
                        has_frame: false,
                    });
                }
            }
            Err("no output contains the capture rect".into())
        }
    }

    fn grab_dxgi(&self, rect: PxRect) -> std::result::Result<Frame, String> {
        let mut guard = self.session.lock();
        let needs_open = guard.as_ref().map_or(true, |s| {
            contained_offset((rect.x, rect.y, rect.w, rect.h), s.origin, s.size).is_none()
        });
        if needs_open {
            *guard = Some(Self::open(rect)?);
        }
        let s = guard.as_mut().expect("session");
        let result = unsafe { Self::read(s, rect) };
        if result.is_err() {
            *guard = None;
        }
        result
    }

    unsafe fn read(s: &mut Session, rect: PxRect) -> std::result::Result<Frame, String> {
        let layout = CaptureLayout::new(rect.x, rect.y, rect.w, rect.h)?;
        let local = contained_offset((rect.x, rect.y, rect.w, rect.h), s.origin, s.size)
            .ok_or("capture rectangle is outside the duplication surface")?;
        // Allocate before mapping so allocation failure cannot leave the texture mapped.
        let mut rgba = layout.allocate()?;
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let timeout = if s.has_frame {
            0
        } else {
            FIRST_FRAME_TIMEOUT_MS
        };
        match s.dup.AcquireNextFrame(timeout, &mut info, &mut resource) {
            Ok(()) => {
                let copied = resource
                    .as_ref()
                    .ok_or_else(|| "AcquireNextFrame returned no resource".to_string())
                    .and_then(|r| r.cast::<ID3D11Texture2D>().map_err(|e| e.to_string()))
                    .map(|tex| s.context.CopyResource(&s.staging, &tex));
                let _ = s.dup.ReleaseFrame();
                copied?;
                s.has_frame = true;
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                if !s.has_frame {
                    return Err("no desktop frame yet".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        s.context
            .Map(&s.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| e.to_string())?;
        // Always unmap, including invalid driver pitch/null-pointer error paths.
        let copied = (|| -> std::result::Result<(), String> {
            if mapped.pData.is_null() {
                return Err("DXGI mapped a null pixel buffer".into());
            }
            let rows = mapped_rows(layout, local, s.size, mapped.RowPitch as usize)?;
            let base = mapped.pData as *const u8;
            for row in 0..rows.rows {
                // mapped_rows has checked the final byte, texture bounds, pitch,
                // and isize::MAX before any pointer/slice is constructed here.
                let src = base.add(rows.first_byte + row * rows.pitch);
                let src_row = std::slice::from_raw_parts(src, rows.row_bytes);
                let dst_row = &mut rgba[row * rows.row_bytes..(row + 1) * rows.row_bytes];
                for (d, sp) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
                    d[0] = sp[2];
                    d[1] = sp[1];
                    d[2] = sp[0];
                    d[3] = 255;
                }
            }
            Ok(())
        })();
        s.context.Unmap(&s.staging, 0);
        copied?;
        Ok(Frame::new(layout.width, layout.height, rgba))
    }
}

impl Default for DxgiCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture for DxgiCapture {
    fn grab(&self, rect: PxRect) -> Result<Frame> {
        CaptureLayout::new(rect.x, rect.y, rect.w, rect.h)
            .map_err(|error| PlatformError::Capture(error.into()))?;
        match self.grab_dxgi(rect) {
            Ok(f) => Ok(f),
            Err(e) => {
                tracing::debug!("dxgi capture: {e}; falling back to GDI");
                self.fallback.grab(rect)
            }
        }
    }
}
