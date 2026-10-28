use image::RgbaImage;

pub fn encode_dib(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = vec![0; 124 + image.as_raw().len()];
    for (offset, value) in [
        (0, 124u32),
        (4, image.width()),
        (8, (-(image.height() as i32)) as u32),
        (16, 3),
        (20, image.as_raw().len() as u32),
        (40, 0x00ff0000),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    for (source, destination) in image
        .as_raw()
        .chunks_exact(4)
        .zip(bytes[124..].chunks_exact_mut(4))
    {
        destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
    bytes
}
pub fn dib_to_bmp(dib: &[u8]) -> Result<Vec<u8>, String> {
    let read = |offset: usize| -> Result<u32, String> {
        dib.get(offset..offset + 4)
            .and_then(|slice| slice.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| "位图头无效".into())
    };
    let header = read(0)? as usize;
    if !matches!(header, 40 | 108 | 124) || dib.len() < header {
        return Err("位图头不受支持".into());
    }
    let bits = u16::from_le_bytes([dib[14], dib[15]]);
    let compression = read(16)?;
    let colors = read(32)? as usize;
    let colors = if colors == 0 && bits <= 8 {
        1usize << bits
    } else {
        colors
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let offset = header
        .checked_add(masks)
        .and_then(|v| colors.checked_mul(4).and_then(|c| v.checked_add(c)))
        .ok_or("位图尺寸溢出")?;
    if offset >= dib.len() {
        return Err("位图像素缺失".into());
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((14 + offset) as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Ok(bmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dib_v5_preserves_alpha_and_top_down_pixel_order() {
        let image = RgbaImage::from_raw(1, 2, vec![12, 34, 56, 78, 90, 80, 70, 255]).unwrap();
        let bmp = dib_to_bmp(&encode_dib(&image)).unwrap();
        assert_eq!(
            image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
                .unwrap()
                .into_rgba8(),
            image
        );
    }
    #[test]
    fn reads_external_bottom_up_24_bit_rows_with_padding() {
        let mut dib = vec![0u8; 48];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..48].copy_from_slice(&[255, 0, 0, 0, 0, 0, 255, 0]);
        let bmp = dib_to_bmp(&dib).unwrap();
        let decoded = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.into_raw(), vec![255, 0, 0, 255, 0, 0, 255, 255]);
    }
    #[test]
    fn corrupt_bitmap_headers_are_rejected_without_allocation() {
        assert!(dib_to_bmp(&[]).is_err());
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(dib_to_bmp(&dib).is_err());
    }
}
