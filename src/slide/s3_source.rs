//! S3-backed slide source implementation.
//!
//! This module provides an implementation of `SlideSource` that creates
//! `S3RangeReader` instances for slides stored in S3 or S3-compatible storage.

use async_trait::async_trait;
use aws_sdk_s3::Client;

use crate::error::IoError;
use crate::io::S3RangeReader;

use super::SlideSource;

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
        S3RangeReader::new(self.client.clone(), self.bucket.clone(), slide_id.to_string()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_slide_source_bucket() {
        // We can't test actual S3 operations without credentials,
        // but we can test the basic structure
        let client = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version_latest()
                .build(),
        );
        let source = S3SlideSource::new(client, "test-bucket".to_string());
        assert_eq!(source.bucket(), "test-bucket");
    }
}
