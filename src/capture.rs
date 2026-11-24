//! Screen capture with DXGI Desktop Duplication, which also sees borderless games.

/// BGRA pixels, `stride` bytes per row.
pub struct Frame {
    pub bgra: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

#[cfg(windows)]
mod dxgi {
    use super::Frame;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;
    use windows::core::Interface;

    fn err(e: windows::core::Error) -> String {
        e.message().to_string()
    }

    /// Desktop rectangles (left, top, right, bottom) of the monitors, with their outputs.
    fn outputs() -> windows::core::Result<Vec<(IDXGIAdapter1, IDXGIOutput1, [i32; 4])>> {
        // SAFETY: plain DXGI factory and enumeration calls; the loops end at the first error (DXGI_ERROR_NOT_FOUND).
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut found = Vec::new();
            let mut a = 0;
            while let Ok(adapter) = factory.EnumAdapters1(a) {
                let mut o = 0;
                while let Ok(output) = adapter.EnumOutputs(o) {
                    let desc = output.GetDesc()?;
                    if desc.AttachedToDesktop.as_bool() {
                        let r = desc.DesktopCoordinates;
                        found.push((adapter.clone(), output.cast()?, [r.left, r.top, r.right, r.bottom]));
                    }
                    o += 1;
                }
                a += 1;
            }
            Ok(found)
        }
    }

    pub fn monitors() -> Vec<[i32; 4]> {
        outputs().map(|o| o.into_iter().map(|(_, _, r)| r).collect()).unwrap_or_default()
    }

    /// One picture of `m` (desktop pixels) through GDI: unlike the first frames of a new
    /// duplication, which can be black, it is always the composed desktop.
    pub fn grab_gdi(m: [i32; 4]) -> Result<Frame, String> {
        use windows::Win32::Graphics::Gdi::*;
        use windows::core::w;
        let (w, h) = (m[2] - m[0], m[3] - m[1]);
        let mut bgra = vec![0u8; (w * h * 4).max(0) as usize];
        // SAFETY: a DISPLAY DC covers the whole virtual screen; every object made here is
        // released below, and GetDIBits writes at most w * h * 4 bytes into `bgra`.
        let lines = unsafe {
            let screen = CreateDCW(w!("DISPLAY"), None, None, None);
            let mem = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bitmap.into());
            let copied = BitBlt(mem, 0, 0, w, h, Some(screen), m[0], m[1], SRCCOPY | CAPTUREBLT);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    // Negative: rows top to bottom.
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let lines = if copied.is_ok() { GetDIBits(mem, bitmap, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS) } else { 0 };
            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
            let _ = DeleteDC(screen);
            lines
        };
        if lines != h {
            return Err("the screen could not be copied".into());
        }
        Ok(Frame { bgra, width: w as usize, height: h as usize, stride: w as usize * 4 })
    }

    pub struct Capture {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        output: IDXGIOutput1,
        dup: Option<IDXGIOutputDuplication>,
        staging: ID3D11Texture2D,
        src: D3D11_BOX,
        width: usize,
        height: usize,
    }

    impl Capture {
        /// Captures `region` (desktop pixels) of the monitor that holds its top-left corner.
        pub fn new(region: [i32; 4]) -> Result<Capture, String> {
            let all = outputs().map_err(err)?;
            let (adapter, output, m) = all
                .iter()
                .find(|(_, _, m)| region[0] >= m[0] && region[0] < m[2] && region[1] >= m[1] && region[1] < m[3])
                .or(all.first())
                .cloned()
                .ok_or("no monitor found")?;
            let clip = |v: i32, lo: i32, hi: i32| (v.clamp(lo, hi) - lo) as u32;
            let src = D3D11_BOX {
                left: clip(region[0], m[0], m[2]),
                top: clip(region[1], m[1], m[3]),
                right: clip(region[2], m[0], m[2]),
                bottom: clip(region[3], m[1], m[3]),
                front: 0,
                back: 1,
            };
            if src.right <= src.left || src.bottom <= src.top {
                return Err("the region is outside every monitor".into());
            }
            let (width, height) = ((src.right - src.left) as usize, (src.bottom - src.top) as usize);
            // SAFETY: D3D11 device and texture creation with valid descriptors and out pointers.
            unsafe {
                let (mut device, mut context) = (None, None);
                D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context))
                    .map_err(err)?;
                let (device, context) = (device.ok_or("no Direct3D device")?, context.ok_or("no Direct3D context")?);
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: width as u32,
                    Height: height as u32,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut staging = None;
                device.CreateTexture2D(&desc, None, Some(&mut staging)).map_err(err)?;
                let mut c = Capture { device, context, output, dup: None, staging: staging.ok_or("no staging texture")?, src, width, height };
                c.open()?;
                Ok(c)
            }
        }

        fn open(&mut self) -> Result<(), String> {
            // SAFETY: duplication of an output of the device's own adapter.
            // DuplicateOutput1 asks for 8-bit BGRA, so HDR desktops are converted; older Windows lacks it.
            let dup = unsafe {
                match self.output.cast::<IDXGIOutput5>() {
                    Ok(o5) => o5.DuplicateOutput1(&self.device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
                    Err(_) => self.output.DuplicateOutput(&self.device),
                }
            };
            self.dup = Some(dup.map_err(err)?);
            Ok(())
        }

        /// The region of the newest screen image; None when the screen did not change.
        pub fn grab(&mut self, timeout_ms: u32) -> Result<Option<Frame>, String> {
            if self.dup.is_none() {
                // Lost before (a UAC prompt, a mode change): try again each frame.
                if self.open().is_err() {
                    return Ok(None);
                }
            }
            let dup = self.dup.as_ref().ok_or("no duplication")?;
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            // SAFETY: valid out pointers; every acquired frame is released below.
            match unsafe { dup.AcquireNextFrame(timeout_ms, &mut info, &mut resource) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    self.dup = None;
                    return Ok(None);
                }
                Err(e) => return Err(err(e)),
            }
            // Only the mouse pointer moved: no new desktop image, and copying it can give black.
            if info.LastPresentTime == 0 {
                // SAFETY: releases the frame acquired just above.
                let _ = unsafe { dup.ReleaseFrame() };
                return Ok(None);
            }
            // SAFETY: the acquired frame is a texture on our device; the staging texture is CPU-readable
            // and Map gives RowPitch bytes for each of `height` rows.
            unsafe {
                let copied = resource.ok_or_else(|| "no frame".to_string()).and_then(|r| r.cast::<ID3D11Texture2D>().map_err(err)).map(|tex| {
                    self.context.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &tex, 0, Some(&self.src));
                });
                let _ = dup.ReleaseFrame();
                copied?;
                let mut map = D3D11_MAPPED_SUBRESOURCE::default();
                self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(err)?;
                let stride = self.width * 4;
                let mut bgra = vec![0; stride * self.height];
                for y in 0..self.height {
                    let row = std::slice::from_raw_parts((map.pData as *const u8).add(y * map.RowPitch as usize), stride);
                    bgra[y * stride..][..stride].copy_from_slice(row);
                }
                self.context.Unmap(&self.staging, 0);
                Ok(Some(Frame { bgra, width: self.width, height: self.height, stride }))
            }
        }
    }
}

#[cfg(windows)]
pub use dxgi::{Capture, grab_gdi, monitors};

#[cfg(not(windows))]
pub struct Capture;

#[cfg(not(windows))]
impl Capture {
    pub fn new(_region: [i32; 4]) -> Result<Capture, String> {
        Err("screen capture needs Windows".into())
    }

    pub fn grab(&mut self, _timeout_ms: u32) -> Result<Option<Frame>, String> {
        Ok(None)
    }
}

#[cfg(not(windows))]
pub fn monitors() -> Vec<[i32; 4]> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn grab_gdi(_m: [i32; 4]) -> Result<Frame, String> {
    Err("screen capture needs Windows".into())
}

fn contains(m: &[i32; 4], (x, y): (i32, i32)) -> bool {
    x >= m[0] && x < m[2] && y >= m[1] && y < m[3]
}

/// The monitor that holds `point`, else the primary monitor (the one at 0,0).
pub fn monitor_at(point: (i32, i32)) -> Option<[i32; 4]> {
    pick(&monitors(), point)
}

fn pick(all: &[[i32; 4]], point: (i32, i32)) -> Option<[i32; 4]> {
    all.iter().find(|m| contains(m, point)).or(all.iter().find(|m| contains(m, (0, 0)))).or(all.first()).copied()
}

/// The monitor after `m`, left to right, wrapping around.
pub fn next_monitor(m: [i32; 4]) -> [i32; 4] {
    after(monitors(), m)
}

fn after(mut all: Vec<[i32; 4]>, m: [i32; 4]) -> [i32; 4] {
    all.sort_by_key(|r| (r[0], r[1]));
    all.iter().position(|r| *r == m).and_then(|i| all.get((i + 1) % all.len())).copied().unwrap_or(m)
}

/// One full image of monitor `m`.
pub fn screenshot(m: [i32; 4]) -> Result<Frame, String> {
    grab_gdi(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_choice() {
        // Primary at 0,0, a second screen to its left, a third to its right.
        let (primary, left, right) = ([0, 0, 2560, 1440], [-1920, 0, 0, 1080], [2560, 0, 5120, 1440]);
        let all = vec![primary, right, left];
        assert_eq!(pick(&all, (441, 60)), Some(primary));
        assert_eq!(pick(&all, (-100, 500)), Some(left));
        assert_eq!(pick(&all, (9000, 9000)), Some(primary), "off-screen regions open on the primary monitor");
        assert_eq!(pick(&[], (0, 0)), None);
        assert_eq!(after(all.clone(), left), primary);
        assert_eq!(after(all.clone(), right), left, "wraps around");
        assert_eq!(after(vec![primary], primary), primary);
    }
}
