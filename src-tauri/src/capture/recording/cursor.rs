//! Draw the system cursor against captured pixels so monochrome XOR cursors work too.
use crate::capture::CaptureRect;
use std::mem::size_of;
use windows::Win32::{Graphics::Gdi::*, UI::WindowsAndMessaging::*};

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct Info(ICONINFO);
impl Drop for Info {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.hbmColor.into());
            let _ = DeleteObject(self.0.hbmMask.into());
        }
    }
}
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
fn clip(
    width: u32,
    height: u32,
    x: i64,
    y: i64,
    cursor_width: u32,
    cursor_height: u32,
) -> Option<CaptureRect> {
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + i64::from(cursor_width)).min(i64::from(width));
    let bottom = (y + i64::from(cursor_height)).min(i64::from(height));
    (right > left && bottom > top).then_some(CaptureRect {
        x: left as i32,
        y: top as i32,
        width: (right - left).max(0) as u32,
        height: (bottom - top).max(0) as u32,
    })
}
pub fn draw(
    image: &mut image::RgbaImage,
    screen: CaptureRect,
    rect: CaptureRect,
) -> Result<(), String> {
    unsafe { draw_native(image, screen, rect) }.map_err(|e| format!("无法录制鼠标：{e}"))
}
unsafe fn draw_native(
    image: &mut image::RgbaImage,
    screen: CaptureRect,
    rect: CaptureRect,
) -> windows::core::Result<()> {
    let mut cursor = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    GetCursorInfo(&mut cursor)?;
    if cursor.flags != CURSOR_SHOWING {
        return Ok(());
    }
    let icon = Icon(CopyIcon(HICON(cursor.hCursor.0))?);
    let mut info = Info(ICONINFO::default());
    GetIconInfo(icon.0, &mut info.0)?;
    let mut bitmap = BITMAP::default();
    let color = !info.0.hbmColor.0.is_null();
    let handle = if color {
        info.0.hbmColor
    } else {
        info.0.hbmMask
    };
    if GetObjectW(
        handle.into(),
        size_of::<BITMAP>() as i32,
        Some((&mut bitmap as *mut BITMAP).cast()),
    ) == 0
    {
        return Err(windows::core::Error::from_win32());
    }
    let width = bitmap.bmWidth;
    let height = if color {
        bitmap.bmHeight
    } else {
        bitmap.bmHeight / 2
    };
    if width <= 0 || height <= 0 {
        return Err(windows::core::Error::from_win32());
    }
    let x = i64::from(cursor.ptScreenPos.x)
        - i64::from(screen.x)
        - i64::from(rect.x)
        - i64::from(info.0.xHotspot);
    let y = i64::from(cursor.ptScreenPos.y)
        - i64::from(screen.y)
        - i64::from(rect.y)
        - i64::from(info.0.yHotspot);
    let Some(area) = clip(
        image.width(),
        image.height(),
        x,
        y,
        width as u32,
        height as u32,
    ) else {
        return Ok(());
    };
    let dc = CreateCompatibleDC(None);
    if dc.0.is_null() {
        return Err(windows::core::Error::from_win32());
    }
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: area.width as i32,
            biHeight: -(area.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let dib = match CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0) {
        Ok(dib) => dib,
        Err(error) => {
            let _ = DeleteDC(dc);
            return Err(error);
        }
    };
    let previous = SelectObject(dc, dib.into());
    let canvas = Canvas {
        dc,
        bitmap: dib,
        previous,
    };
    if previous.0.is_null() || previous.0 as isize == -1 {
        return Err(windows::core::Error::from_win32());
    }
    let pixels = std::slice::from_raw_parts_mut(
        bits.cast::<u8>(),
        area.width as usize * area.height as usize * 4,
    );
    for (index, destination) in pixels.chunks_exact_mut(4).enumerate() {
        let source = image.get_pixel(
            area.x as u32 + index as u32 % area.width,
            area.y as u32 + index as u32 / area.width,
        );
        destination.copy_from_slice(&[source[2], source[1], source[0], 255]);
    }
    DrawIconEx(
        canvas.dc,
        (x - i64::from(area.x)) as i32,
        (y - i64::from(area.y)) as i32,
        icon.0,
        width,
        height,
        0,
        None,
        DI_NORMAL,
    )?;
    if !GdiFlush().as_bool() {
        return Err(windows::core::Error::from_win32());
    }
    for (index, source) in pixels.chunks_exact(4).enumerate() {
        image.put_pixel(
            area.x as u32 + index as u32 % area.width,
            area.y as u32 + index as u32 / area.width,
            image::Rgba([source[2], source[1], source[0], 255]),
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clips_cursor_edges_without_moving_the_hotspot() {
        assert_eq!(
            clip(100, 80, -5, -7, 32, 32),
            Some(CaptureRect {
                x: 0,
                y: 0,
                width: 27,
                height: 25
            })
        );
        assert_eq!(
            clip(100, 80, 95, 75, 32, 32),
            Some(CaptureRect {
                x: 95,
                y: 75,
                width: 5,
                height: 5
            })
        );
        assert!(clip(100, 80, -40, 0, 32, 32).is_none());
        assert!(clip(100, 80, 100, 0, 32, 32).is_none());
    }
}
