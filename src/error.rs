use thiserror::Error;

/// I/O errors that can occur when reading from remote storage
#[derive(Debug, Clone, Error)]
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

/// Errors related to format detection and validation
#[derive(Debug, Clone, Error)]
pub enum FormatError {
    /// I/O error while reading the file
    #[error("I/O error: {0}")]
    Io(#[from] IoError),

    /// TIFF parsing error
    #[error("TIFF error: {0}")]
    Tiff(#[from] TiffError),

    /// File format is not supported (should map to HTTP 415)
    #[error("Unsupported format: {reason}")]
    UnsupportedFormat { reason: String },
}

/// Errors that can occur when parsing TIFF files
#[derive(Debug, Clone, Error)]
pub enum TiffError {
    /// I/O error while reading the file
    #[error("I/O error: {0}")]
    Io(#[from] IoError),

    /// Invalid TIFF magic bytes (not II or MM)
    #[error("Invalid TIFF magic bytes: expected 0x4949 (II) or 0x4D4D (MM), got 0x{0:04X}")]
    InvalidMagic(u16),

    /// Invalid TIFF version number
    #[error("Invalid TIFF version: expected 42 (TIFF) or 43 (BigTIFF), got {0}")]
    InvalidVersion(u16),

    /// Invalid BigTIFF offset byte size (must be 8)
    #[error("Invalid BigTIFF offset byte size: expected 8, got {0}")]
    InvalidBigTiffOffsetSize(u16),

    /// File is too small to contain a valid TIFF header
    #[error("File too small: need at least {required} bytes, got {actual}")]
    FileTooSmall { required: u64, actual: u64 },

    /// Invalid IFD offset (points outside file or to invalid location)
    #[error("Invalid IFD offset: {0}")]
    InvalidIfdOffset(u64),

    /// Required tag is missing from IFD
    #[error("Missing required tag: {0}")]
    MissingTag(&'static str),

    /// Tag has unexpected type or count
    #[error("Invalid tag value for {tag}: {message}")]
    InvalidTagValue { tag: &'static str, message: String },

    /// Unsupported compression scheme
    #[error("Unsupported compression: {0} (only JPEG is supported)")]
    UnsupportedCompression(String),

    /// File uses strips instead of tiles
    #[error("Unsupported organization: file uses strips instead of tiles")]
    StripOrganization,

    /// Unknown field type in IFD entry
    #[error("Unknown field type: {0}")]
    UnknownFieldType(u16),
}
