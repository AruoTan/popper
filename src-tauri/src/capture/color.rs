use super::{CaptureColorFormat, CapturePoint};
use image::RgbaImage;

pub fn sample(
    image: &RgbaImage,
    point: CapturePoint,
    format: CaptureColorFormat,
) -> Result<String, String> {
    if image.width() == 0
        || image.height() == 0
        || point.x < 0
        || point.y < 0
        || point.x as u32 > image.width()
        || point.y as u32 > image.height()
    {
        return Err("取色位置超出屏幕范围".into());
    }
    let pixel = image.get_pixel(
        (point.x as u32).min(image.width() - 1),
        (point.y as u32).min(image.height() - 1),
    );
    Ok(format.format([pixel[0], pixel[1], pixel[2]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_frozen_pixels_in_the_requested_format_including_the_screen_edges() {
        let image = RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([x as u8 * 255, y as u8 * 255, 35, 255])
        });
        for (point, hex, rgb) in [
            ((0, 0), "#000023", "0, 0, 35"),
            ((1, 0), "#FF0023", "255, 0, 35"),
            ((0, 1), "#00FF23", "0, 255, 35"),
            ((1, 1), "#FFFF23", "255, 255, 35"),
            ((2, 2), "#FFFF23", "255, 255, 35"),
        ] {
            for (format, expected) in [
                (CaptureColorFormat::Hex, hex),
                (CaptureColorFormat::Rgb, rgb),
            ] {
                assert_eq!(
                    sample(
                        &image,
                        CapturePoint {
                            x: point.0,
                            y: point.1
                        },
                        format
                    )
                    .unwrap(),
                    expected
                );
            }
        }
        for (x, y) in [(-1, 0), (0, -1), (3, 0), (0, 3), (i32::MAX, i32::MAX)] {
            assert!(sample(&image, CapturePoint { x, y }, CaptureColorFormat::Hex).is_err());
        }
        assert!(sample(
            &RgbaImage::new(0, 0),
            CapturePoint { x: 0, y: 0 },
            CaptureColorFormat::Hex
        )
        .is_err());
    }

    #[test]
    fn accepts_only_supported_color_formats() {
        for format in ["hex", "rgb"] {
            assert!(
                serde_json::from_value::<CaptureColorFormat>(serde_json::json!(format)).is_ok()
            );
        }
        for format in ["HEX", "hsl", "", "rgba"] {
            assert!(
                serde_json::from_value::<CaptureColorFormat>(serde_json::json!(format)).is_err()
            );
        }
    }
}
