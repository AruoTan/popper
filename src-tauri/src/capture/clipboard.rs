use super::bitmap::{dib_to_bmp, encode_dib};
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io::Cursor;
use windows::{
    core::w,
    Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND},
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
                OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{CF_DIB, CF_DIBV5},
        },
    },
};

pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
fn open(owner: Option<HWND>) -> Result<Clipboard, String> {
    for attempt in 0..6 {
        if unsafe { OpenClipboard(owner) }.is_ok() {
            return Ok(Clipboard);
        }
        if attempt < 5 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    Err("剪贴板正被占用，请重试".into())
}
struct Memory(HGLOBAL);
impl Memory {
    fn new(bytes: &[u8]) -> Result<Self, String> {
        let handle =
            unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(|e| e.to_string())?;
        let allocation = Self(handle);
        let pointer = unsafe { GlobalLock(handle) };
        if pointer.is_null() {
            return Err("无法分配剪贴板图像".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
            let _ = GlobalUnlock(handle);
        }
        Ok(allocation)
    }
    fn transfer(mut self, format: u32) -> Result<(), String> {
        unsafe { SetClipboardData(format, Some(HANDLE(self.0 .0))) }.map_err(|e| e.to_string())?;
        self.0 = HGLOBAL::default();
        Ok(())
    }
}
impl Drop for Memory {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = GlobalFree(Some(self.0));
            }
        }
    }
}
pub fn png(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(bytes.into_inner())
}
pub fn decode(bytes: &[u8], format: ImageFormat) -> Result<RgbaImage, String> {
    if bytes.len() > MAX_BYTES {
        return Err("图片体积过大".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_BYTES as u64);
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "图片格式无效或尺寸过大".to_owned())?
        .into_rgba8();
    if image.width() == 0
        || image.height() == 0
        || u64::from(image.width()) * u64::from(image.height()) > MAX_PIXELS
    {
        return Err("图片像素数量过大".into());
    }
    Ok(image)
}
pub fn write(owner: HWND, image: &RgbaImage, png_bytes: &[u8]) -> Result<(), String> {
    let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    if png_format == 0 {
        return Err("无法注册剪贴板图像格式".into());
    }
    let encoded = Memory::new(png_bytes)?;
    let dib = Memory::new(&encode_dib(image))?;
    let _guard = open(Some(owner))?;
    unsafe { EmptyClipboard() }.map_err(|e| e.to_string())?;
    encoded.transfer(png_format)?;
    dib.transfer(u32::from(CF_DIBV5.0))
}
fn copy(format: u32) -> Result<Vec<u8>, String> {
    let handle = unsafe { GetClipboardData(format) }.map_err(|e| e.to_string())?;
    let global = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(global) };
    if size == 0 || size > MAX_BYTES {
        return Err("剪贴板图片体积无效".into());
    }
    let pointer = unsafe { GlobalLock(global) };
    if pointer.is_null() {
        return Err("无法读取剪贴板图片".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) }.to_vec();
    unsafe {
        let _ = GlobalUnlock(global);
    }
    Ok(bytes)
}
pub fn read() -> Result<RgbaImage, String> {
    let _guard = open(None)?;
    let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    if png_format != 0 && unsafe { IsClipboardFormatAvailable(png_format) }.is_ok() {
        if let Ok(bytes) = copy(png_format) {
            if let Ok(image) = decode(&bytes, ImageFormat::Png) {
                return Ok(image);
            }
        }
    }
    for format in [u32::from(CF_DIBV5.0), u32::from(CF_DIB.0)] {
        if unsafe { IsClipboardFormatAvailable(format) }.is_ok() {
            if let Ok(bytes) = copy(format).and_then(|bytes| dib_to_bmp(&bytes)) {
                if let Ok(image) = decode(&bytes, ImageFormat::Bmp) {
                    return Ok(image);
                }
            }
        }
    }
    Err("剪贴板中没有可用的图片".into())
}
