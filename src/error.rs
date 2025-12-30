use thiserror::Error;

/// I/O errors that can occur when reading from remote storage
#[derive(Debug, Error)]
pub enum IoError {
    /// Error from S3 or S3-compatible storage
    #[error("S3 error: {0}")]
    S3(String),

    /// Requested range exceeds resource bounds
    #[error("Range out of bounds: requested {requested} bytes at offset {offset}, size is {size}")]
    RangeOutOfBounds {
        offset: u64,
        requested: u64,
        size: u64,
    },

    /// Network or connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Object not found
    #[error("Object not found: {0}")]
    NotFound(String),
}
