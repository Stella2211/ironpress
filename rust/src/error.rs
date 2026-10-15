use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum CompressError {
    IoError(std::io::Error),
    DecodeError(String),
    EncodeError(String),
    UnsupportedFormat(String),
    InvalidParameter(String),
    /// Image dimensions exceed the decode pixel limit (decompression bomb guard).
    ImageTooLarge(String),
}

impl CompressError {
    /// FFI error code for this error. Size-limit violations share `-5` with
    /// the input byte-size limit; everything else is a generic engine error.
    pub fn ffi_code(&self) -> i32 {
        match self {
            Self::ImageTooLarge(_) => -5,
            _ => -10,
        }
    }
}

impl fmt::Display for CompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IoError(e) => write!(f, "IO error: {e}"),
            Self::DecodeError(msg) => write!(f, "Decode error: {msg}"),
            Self::EncodeError(msg) => write!(f, "Encode error: {msg}"),
            Self::UnsupportedFormat(fmt_name) => write!(f, "Unsupported format: {fmt_name}"),
            Self::InvalidParameter(msg) => write!(f, "Invalid parameter: {msg}"),
            Self::ImageTooLarge(msg) => write!(f, "Image too large: {msg}"),
        }
    }
}

impl From<std::io::Error> for CompressError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<image::ImageError> for CompressError {
    fn from(e: image::ImageError) -> Self {
        Self::DecodeError(e.to_string())
    }
}
