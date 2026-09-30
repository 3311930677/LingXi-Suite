//! Screen capture utilities using Win32 GDI BitBlt.
//!
//! Provides full-screen and region capture, returning RGBA pixel data that
//! can be encoded to PNG for use by the agent or widget frontends.

/// A captured screen image as raw RGBA pixels.
pub struct CapturedImage {
    pub width: i32,
    pub height: i32,
    /// RGBA pixel data, row-major, top-to-bottom, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

#[cfg(windows)]
pub fn capture_screen() -> Result<CapturedImage, String> {
    capture_region(0, 0, 0, 0)
}

#[cfg(windows)]
pub fn capture_region(x: i32, y: i32, width: i32, height: i32) -> Result<CapturedImage, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    if screen_w <= 0 || screen_h <= 0 {
        return Err("无法获取屏幕尺寸".to_string());
    }

    let w = if width > 0 {
        width.min(screen_w)
    } else {
        screen_w
    };
    let h = if height > 0 {
        height.min(screen_h)
    } else {
        screen_h
    };
    let off_x = x.clamp(0, screen_w - 1);
    let off_y = y.clamp(0, screen_h - 1);

    let hwnd = HWND(std::ptr::null_mut());
    let hdc_screen = unsafe { GetDC(hwnd) };
    if hdc_screen.is_invalid() {
        return Err("GetDC 返回空句柄".to_string());
    }

    let result = (|| {
        let hdc_mem = unsafe { CreateCompatibleDC(hdc_screen) };
        if hdc_mem.is_invalid() {
            return Err("CreateCompatibleDC 失败".to_string());
        }
        let bmp = unsafe { CreateCompatibleBitmap(hdc_screen, w, h) };
        if bmp.is_invalid() {
            let _ = unsafe { DeleteDC(hdc_mem) };
            return Err("CreateCompatibleBitmap 失败".to_string());
        }
        let old_bmp = unsafe { SelectObject(hdc_mem, bmp) };

        let bit_result = unsafe { BitBlt(hdc_mem, 0, 0, w, h, hdc_screen, off_x, off_y, SRCCOPY) };
        if bit_result.is_err() {
            unsafe {
                let _ = SelectObject(hdc_mem, old_bmp);
                let _ = DeleteObject(bmp);
                let _ = DeleteDC(hdc_mem);
            }
            return Err("BitBlt 失败".to_string());
        }

        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB = 0
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default()],
        };

        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let got = unsafe {
            GetDIBits(
                hdc_mem,
                bmp,
                0,
                h as u32,
                Some(rgba.as_mut_ptr() as *mut _),
                &mut bi,
                DIB_RGB_COLORS,
            )
        };
        if got == 0 {
            unsafe {
                let _ = SelectObject(hdc_mem, old_bmp);
                let _ = DeleteObject(bmp);
                let _ = DeleteDC(hdc_mem);
            }
            return Err("GetDIBits 失败".to_string());
        }

        // GDI returns BGRA; convert to RGBA for PNG encoding.
        for chunk in rgba.as_chunks_mut::<4>().0 {
            chunk.swap(0, 2);
        }

        unsafe {
            let _ = SelectObject(hdc_mem, old_bmp);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(hdc_mem);
        }
        Ok(CapturedImage {
            width: w,
            height: h,
            rgba,
        })
    })();

    unsafe {
        let _ = ReleaseDC(hwnd, hdc_screen);
    }
    result
}

#[cfg(windows)]
pub fn encode_png(img: &CapturedImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, img.width as u32, img.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG write_header: {e}"))?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| format!("PNG write_image_data: {e}"))?;
    }
    Ok(buf)
}

#[cfg(windows)]
pub fn capture_screen_as_data_url() -> Result<String, String> {
    let img = capture_screen()?;
    let png = encode_png(&img)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
    Ok(format!("data:image/png;base64,{}", b64))
}

#[cfg(windows)]
pub fn capture_region_as_data_url(x: i32, y: i32, w: i32, h: i32) -> Result<String, String> {
    let img = capture_region(x, y, w, h)?;
    let png = encode_png(&img)?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png);
    Ok(format!("data:image/png;base64,{}", b64))
}

#[cfg(windows)]
pub fn read_pixel(x: i32, y: i32) -> Result<(u8, u8, u8), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};

    let hwnd = HWND(std::ptr::null_mut());
    let hdc = unsafe { GetDC(hwnd) };
    if hdc.is_invalid() {
        return Err("GetDC 失败".to_string());
    }
    let color = unsafe { GetPixel(hdc, x, y) };
    unsafe {
        let _ = ReleaseDC(hwnd, hdc);
    }
    // COLORREF is 0x00BBGGRR
    Ok((
        (color.0 & 0xFF) as u8,
        ((color.0 >> 8) & 0xFF) as u8,
        ((color.0 >> 16) & 0xFF) as u8,
    ))
}

#[cfg(not(windows))]
pub fn capture_screen() -> Result<CapturedImage, String> {
    Err("capture_screen 仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn capture_screen_as_data_url() -> Result<String, String> {
    Err("capture_screen 仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn capture_region_as_data_url(_x: i32, _y: i32, _w: i32, _h: i32) -> Result<String, String> {
    Err("capture_region 仅支持 Windows".to_string())
}

#[cfg(not(windows))]
pub fn read_pixel(_x: i32, _y: i32) -> Result<(u8, u8, u8), String> {
    Err("read_pixel 仅支持 Windows".to_string())
}
