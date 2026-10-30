//! Error types for LZMS decompression.
//!
//! An error can have a context: a byte offset in the input, a structure name and details.
//!
//! # Example
//!
//! ```
//! match lzms::decompress(b"\x01\x02", 100) {
//!     Ok(_) => println!("decompressed"),
//!     Err(e) => eprintln!("Error: {e}"),
//! }
//! ```

use crate::prelude::*;
use core::fmt;

/// Result type alias for lzms operations.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Information about the location of an error.
#[derive(Debug, Clone, Default)]
pub struct ErrorContext {
    /// Byte offset in the input where the error occurred (if known).
    pub offset: Option<u64>,
    /// Name of the structure that the decoder was reading (if known).
    pub structure: Option<String>,
    /// Additional details about the error.
    pub details: Option<String>,
}

impl ErrorContext {
    /// Return `true` if the context has no information.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offset.is_none() && self.structure.is_none() && self.details.is_none()
    }
}

impl fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(offset) = self.offset {
            parts.push(format!("offset {offset:#x}"));
        }
        if let Some(structure) = &self.structure {
            parts.push(format!("in {structure}"));
        }
        if let Some(details) = &self.details {
            parts.push(details.clone());
        }
        write!(f, "{}", parts.join(", "))
    }
}

/// An error from decompressing LZMS data.
#[derive(Debug)]
pub struct Error {
    /// The error kind.
    pub kind: ErrorKind,
    /// Optional context about where the error occurred.
    pub context: Option<ErrorContext>,
}

/// The kind of error that occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A Compression API buffer does not start with the `0x0a51e5c0` signature.
    InvalidSignature([u8; 4]),
    /// The input ended before the decoder read all data.
    UnexpectedEnd,
    /// A value in the input is not valid.
    InvalidData(String),
    /// The input uses a feature that the crate does not support.
    Unsupported(String),
    /// The output is larger than the limit allows.
    LimitExceeded {
        /// The size that the input needs, in bytes.
        needed: u64,
        /// The limit, in bytes.
        limit: u64,
    },
}

impl Error {
    /// Create a new error with the given kind.
    #[must_use]
    pub fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            context: None,
        }
    }

    /// Add offset context.
    #[must_use]
    pub fn at_offset(mut self, offset: u64) -> Self {
        self.context
            .get_or_insert_with(ErrorContext::default)
            .offset = Some(offset);
        self
    }

    /// Add structure name context.
    #[must_use]
    pub fn in_structure(mut self, name: impl Into<String>) -> Self {
        self.context
            .get_or_insert_with(ErrorContext::default)
            .structure = Some(name.into());
        self
    }

    /// Add details.
    #[must_use]
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.context
            .get_or_insert_with(ErrorContext::default)
            .details = Some(details.into());
        self
    }

    /// Get the context if present.
    #[must_use]
    pub fn context(&self) -> Option<&ErrorContext> {
        self.context.as_ref()
    }

    /// Get the error kind.
    #[must_use]
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Create an `UnexpectedEnd` error.
    #[must_use]
    pub fn unexpected_end() -> Self {
        Self::new(ErrorKind::UnexpectedEnd)
    }

    /// Create an `InvalidData` error.
    pub fn invalid_data(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidData(msg.into()))
    }

    /// Create an `Unsupported` error.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported(msg.into()))
    }

    /// Create a `LimitExceeded` error.
    #[must_use]
    pub fn limit_exceeded(needed: u64, limit: u64) -> Self {
        Self::new(ErrorKind::LimitExceeded { needed, limit })
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::InvalidSignature(sig) => {
                write!(f, "Invalid signature {sig:02x?} (expected 0a 51 e5 c0)")
            }
            ErrorKind::UnexpectedEnd => write!(f, "Unexpected end of the compressed data"),
            ErrorKind::InvalidData(msg) => write!(f, "Invalid LZMS data: {msg}"),
            ErrorKind::Unsupported(msg) => write!(f, "Unsupported LZMS feature: {msg}"),
            ErrorKind::LimitExceeded { needed, limit } => {
                write!(
                    f,
                    "The output needs {needed} bytes, more than the limit of {limit}"
                )
            }
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(ctx) = &self.context
            && !ctx.is_empty()
        {
            write!(f, " ({ctx})")?;
        }
        Ok(())
    }
}

impl core::error::Error for Error {}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self::new(kind)
    }
}
