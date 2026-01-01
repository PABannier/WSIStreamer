//! S3-backed slide source implementation.
//!
//! This module provides an implementation of `SlideSource` that creates
//! `S3RangeReader` instances for slides stored in S3 or S3-compatible storage.

use async_trait::async_trait;
use aws_sdk_s3::Client;

use crate::error::IoError;
use crate::io::S3RangeReader;

use super::{SlideListResult, SlideSource};

// =============================================================================
// Slide Extension Filtering
// =============================================================================

/// Supported slide file extensions (case-insensitive).
const SLIDE_EXTENSIONS: &[&str] = &[".svs", ".tif", ".tiff"];

/// Check if a file path has a supported slide extension.
fn is_slide_file(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    SLIDE_EXTENSIONS.iter().any(|ext| path_lower.ends_with(ext))
}

/// S3-backed implementation of `SlideSource`.
///
/// Creates `S3RangeReader` instances for slides stored in an S3 bucket.
/// The slide ID is used as the object key within the bucket.
///
/// # Example
///
/// ```ignore
/// use wsi_streamer::slide::S3SlideSource;
/// use wsi_streamer::io::create_s3_client;
///
/// let client = create_s3_client(None).await;
/// let source = S3SlideSource::new(client, "my-bucket".to_string());
///
/// // The slide ID "slides/example.svs" becomes the S3 key
/// let reader = source.create_reader("slides/example.svs").await?;
/// ```
#[derive(Clone)]
pub struct S3SlideSource {
    client: Client,
    bucket: String,
}

impl S3SlideSource {
    /// Create a new S3SlideSource for the given bucket.
    ///
    /// # Arguments
    /// * `client` - AWS S3 client to use for requests
    /// * `bucket` - S3 bucket name containing the slides
    pub fn new(client: Client, bucket: String) -> Self {
        Self { client, bucket }
    }

    /// Get the bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

#[async_trait]
impl SlideSource for S3SlideSource {
    type Reader = S3RangeReader;

    async fn create_reader(&self, slide_id: &str) -> Result<Self::Reader, IoError> {
        S3RangeReader::new(
            self.client.clone(),
            self.bucket.clone(),
            slide_id.to_string(),
        )
        .await
    }

    async fn list_slides(
        &self,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<SlideListResult, IoError> {
        let mut request = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .max_keys(limit as i32);

        if let Some(token) = cursor {
            request = request.continuation_token(token);
        }

        let response = request
            .send()
            .await
            .map_err(|e| IoError::S3(e.to_string()))?;

        let slides: Vec<String> = response
            .contents()
            .iter()
            .filter_map(|obj| obj.key())
            .filter(|key| is_slide_file(key))
            .map(|s| s.to_string())
            .collect();

        Ok(SlideListResult {
            slides,
            next_cursor: response.next_continuation_token().map(|s| s.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_smithy_runtime::client::http::hyper_014::HyperClientBuilder;
    use hyper_rustls::HttpsConnectorBuilder;

    #[test]
    fn test_s3_slide_source_bucket() {
        // We can't test actual S3 operations without credentials,
        // but we can test the basic structure
        let https_connector = HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .enable_http2()
            .build();
        let http_client = HyperClientBuilder::new().build(https_connector);
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .http_client(http_client)
            .build();
        let client = aws_sdk_s3::Client::from_conf(config);
        let source = S3SlideSource::new(client, "test-bucket".to_string());
        assert_eq!(source.bucket(), "test-bucket");
    }

    #[test]
    fn test_is_slide_file_svs() {
        assert!(is_slide_file("slide.svs"));
        assert!(is_slide_file("path/to/slide.svs"));
        assert!(is_slide_file("SLIDE.SVS"));
        assert!(is_slide_file("path/to/SLIDE.Svs"));
    }

    #[test]
    fn test_is_slide_file_tif() {
        assert!(is_slide_file("slide.tif"));
        assert!(is_slide_file("path/to/slide.tif"));
        assert!(is_slide_file("SLIDE.TIF"));
    }

    #[test]
    fn test_is_slide_file_tiff() {
        assert!(is_slide_file("slide.tiff"));
        assert!(is_slide_file("path/to/slide.tiff"));
        assert!(is_slide_file("SLIDE.TIFF"));
    }

    #[test]
    fn test_is_slide_file_non_slide() {
        assert!(!is_slide_file("image.jpg"));
        assert!(!is_slide_file("document.pdf"));
        assert!(!is_slide_file("slide.svs.backup"));
        assert!(!is_slide_file("slide_svs"));
        assert!(!is_slide_file(""));
        assert!(!is_slide_file("no_extension"));
    }
}
