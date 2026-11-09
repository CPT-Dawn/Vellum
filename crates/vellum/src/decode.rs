//! Decoding a wallpaper file into pixels. JPEG, PNG and WebP are built in;
//! AVIF and JXL come with the `avif` and `jxl` cargo features (ADR 0002).

use std::path::Path;

use image::RgbaImage;

/// Decodes the wallpaper at `file`, whatever its extension says.
pub fn load(file: &Path) -> Result<RgbaImage, String> {
    let fail = |what: &str, err: &dyn std::fmt::Display| {
        format!("cannot {what} {}: {err}", file.display())
    };
    #[cfg(feature = "jxl")]
    if is_jxl(file) {
        let file = std::fs::File::open(file).map_err(|e| fail("open", &e))?;
        let decoder = jxl_oxide::integration::JxlDecoder::new(std::io::BufReader::new(file))
            .map_err(|e| fail("decode", &e))?;
        return image::DynamicImage::from_decoder(decoder)
            .map(|image| image.into_rgba8())
            .map_err(|e| fail("decode", &e));
    }
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(file)
        .map_err(|e| fail("open", &e))?
        .with_guessed_format()
        .map_err(|e| fail("read", &e))?
        .into_decoder()
        .map_err(|e| fail("decode", &e))?;
    // Phone photos are often stored sideways with an EXIF Orientation tag.
    let orientation = decoder.orientation().ok();
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| fail("decode", &e))?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Ok(image.into_rgba8())
}

/// Whether `file` starts with a JPEG XL signature (bare codestream or
/// ISO BMFF container).
#[cfg(feature = "jxl")]
fn is_jxl(file: &Path) -> bool {
    use std::io::Read;
    const CONTAINER: [u8; 12] = [
        0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a,
    ];
    let mut head = [0; 12];
    let Ok(mut f) = std::fs::File::open(file) else {
        return false;
    };
    let n = f.read(&mut head).unwrap_or(0);
    head[..n].starts_with(&[0xff, 0x0a]) || head[..n] == CONTAINER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[cfg(any(feature = "jxl", feature = "avif"))]
    #[track_caller]
    fn assert_red_16x8(image: &RgbaImage) {
        assert_eq!(image.dimensions(), (16, 8));
        let [r, g, b, a] = image.get_pixel(8, 4).0;
        assert!(
            r > 245 && g < 10 && b < 10 && a == 255,
            "{:?}",
            [r, g, b, a]
        );
    }

    #[test]
    fn a_jpeg_is_turned_upright_by_its_exif_orientation() {
        // Stored 16x8, red then blue, tagged "rotate 90° clockwise".
        let image = load(&fixture("red-blue-orient6.jpg")).unwrap();
        assert_eq!(image.dimensions(), (8, 16));
        let [r, _, b, _] = image.get_pixel(4, 2).0;
        assert!(r > 200 && b < 60, "top should be red: {r} {b}");
        let [r, _, b, _] = image.get_pixel(4, 13).0;
        assert!(b > 200 && r < 60, "bottom should be blue: {r} {b}");
    }

    #[cfg(feature = "jxl")]
    #[test]
    fn jxl_decodes_with_the_jxl_feature() {
        assert_red_16x8(&load(&fixture("red.jxl")).unwrap());
    }

    #[cfg(feature = "avif")]
    #[test]
    fn avif_decodes_with_the_avif_feature() {
        assert_red_16x8(&load(&fixture("red.avif")).unwrap());
    }

    #[cfg(not(feature = "jxl"))]
    #[test]
    fn jxl_is_not_built_in_by_default() {
        let err = load(&fixture("red.jxl")).unwrap_err();
        assert!(err.contains("red.jxl"), "{err}");
    }

    #[cfg(not(feature = "avif"))]
    #[test]
    fn avif_is_not_built_in_by_default() {
        let err = load(&fixture("red.avif")).unwrap_err();
        assert!(err.contains("red.avif"), "{err}");
    }
}
