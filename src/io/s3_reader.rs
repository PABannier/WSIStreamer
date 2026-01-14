use async_trait::async_trait;
use aws_sdk_s3::Client;
use bytes::Bytes;

use super::RangeReader;
use crate::error::IoError;

/// S3-backed implementation of RangeReader.
///
/// Reads byte ranges from objects in S3 or S3-compatible storage (MinIO, GCS, etc.)
/// using HTTP range requests. The object size is fetched once on creation via HEAD.
#[derive(Clone)]
pub struct S3RangeReader {
    client: Client,
    bucket: String,
    key: String,
    size: u64,
    identifier: String,
}

impl S3RangeReader {
    /// Create a new S3RangeReader for the given bucket and key.
    ///
    /// This performs a HEAD request to determine the object size.
    /// Returns an error if the object does not exist or is inaccessible.
    pub async fn new(client: Client, bucket: String, key: String) -> Result<Self, IoError> {
        let head = client
            .head_object()
            .bucket(&bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| {
                // Check if this is a 404 Not Found error
                // The HeadObjectError has an is_not_found() method that we can use
                let is_not_found = e
                    .as_service_error()
                    .map(|se| se.is_not_found())
                    .unwrap_or(false);

                if is_not_found {
                    return IoError::NotFound(format!("s3://{}/{}", bucket, key));
                }

                // Also check for 404 status code in the raw response
                let status_is_404 = e
                    .raw_response()
                    .map(|r| r.status().as_u16() == 404)
                    .unwrap_or(false);

                if status_is_404 {
                    return IoError::NotFound(format!("s3://{}/{}", bucket, key));
                }

                // Fallback: check the error string for common patterns
                let err_str = e.to_string();
                if err_str.contains("NotFound")
                    || err_str.contains("NoSuchKey")
                    || err_str.contains("404")
                {
                    return IoError::NotFound(format!("s3://{}/{}", bucket, key));
                }

                IoError::S3(err_str)
            })?;

        let size = head.content_length().unwrap_or(0) as u64;
        let identifier = format!("s3://{}/{}", bucket, key);

        Ok(Self {
            client,
            bucket,
            key,
            size,
            identifier,
        })
    }

    /// Get the bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Get the object key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

#[async_trait]
impl RangeReader for S3RangeReader {
    async fn read_exact_at(&self, offset: u64, len: usize) -> Result<Bytes, IoError> {
        // Validate range bounds
        if offset + len as u64 > self.size {
            return Err(IoError::RangeOutOfBounds {
                offset,
                requested: len as u64,
                size: self.size,
            });
        }

        // Handle zero-length reads
        if len == 0 {
            return Ok(Bytes::new());
        }

        // Build range header: "bytes=start-end" (inclusive on both ends)
        let range = format!("bytes={}-{}", offset, offset + len as u64 - 1);

        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&self.key)
            .range(range)
            .send()
            .await
            .map_err(|e| IoError::S3(e.to_string()))?;

        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| IoError::Connection(e.to_string()))?
            .into_bytes();

        Ok(data)
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn identifier(&self) -> &str {
        &self.identifier
    }
}

/// Create an S3 client with optional custom endpoint and region.
///
/// Use a custom endpoint for S3-compatible services like MinIO:
/// ```ignore
/// let client = create_s3_client(Some("http://localhost:9000"), "us-east-1").await;
/// ```
///
/// For AWS S3, pass `None` to use the default endpoint:
/// ```ignore
/// let client = create_s3_client(None, "us-east-1").await;
/// ```
pub async fn create_s3_client(endpoint_url: Option<&str>, region: &str) -> Client {
    let region = aws_config::Region::new(region.to_string());
    let mut config_loader =
        aws_config::defaults(aws_config::BehaviorVersion::latest()).region(region);

    if let Some(endpoint) = endpoint_url {
        config_loader = config_loader.endpoint_url(endpoint);
    }

    let sdk_config = config_loader.load().await;

    // For S3-compatible services, we often need to use path-style addressing
    let s3_config = if endpoint_url.is_some() {
        aws_sdk_s3::config::Builder::from(&sdk_config)
            .force_path_style(true)
            .build()
    } else {
        aws_sdk_s3::config::Builder::from(&sdk_config).build()
    };

    Client::from_conf(s3_config)
}

#[cfg(test)]
mod tests {
    // Integration tests require a running S3-compatible service (e.g., MinIO)
    // and are not included in unit tests. See tests/integration/ for E2E tests.
}
