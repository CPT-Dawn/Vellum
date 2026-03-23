use image::GenericImageView;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub(crate) struct ImagePreflight {
    pub(crate) exists: bool,
    pub(crate) readable: bool,
    pub(crate) dimensions: Option<(u32, u32)>,
    pub(crate) decode_error: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ImagePipeline;

impl ImagePipeline {
    pub(crate) fn inspect(&self, path: &Path) -> ImagePreflight {
        let mut preflight = ImagePreflight::default();

        if !path.exists() {
            return preflight;
        }

        preflight.exists = true;

        match image::ImageReader::open(path) {
            Ok(reader) => {
                preflight.readable = true;
                match reader.decode() {
                    Ok(image) => {
                        preflight.dimensions = Some(image.dimensions());
                    }
                    Err(err) => {
                        preflight.decode_error = Some(err.to_string());
                    }
                }
            }
            Err(err) => {
                preflight.decode_error = Some(err.to_string());
            }
        }

        preflight
    }
}

#[cfg(test)]
mod tests {
    use super::ImagePipeline;

    #[test]
    fn inspect_nonexistent_path_reports_missing() {
        let pipeline = ImagePipeline;
        let sample = std::env::temp_dir().join("vellum-preflight-missing-image.png");

        let report = pipeline.inspect(&sample);
        assert!(!report.exists);
        assert!(!report.readable);
        assert!(report.dimensions.is_none());
    }

    #[test]
    fn inspect_invalid_image_reports_decode_error() {
        let pipeline = ImagePipeline;
        let sample = std::env::temp_dir().join("vellum-preflight-invalid-image.bin");
        let write_result = std::fs::write(&sample, b"not-an-image");
        assert!(write_result.is_ok());

        let report = pipeline.inspect(&sample);
        assert!(report.exists);
        assert!(report.readable);
        assert!(report.dimensions.is_none());
        assert!(report.decode_error.is_some());

        let _ = std::fs::remove_file(sample);
    }
}
