//! A pure-Rust reader for the DWG file format, written from the public file
//! specification and real files.
//!
//! The reader is built layer by layer, each layer checked on its own before
//! the next one is trusted. This first layer answers one question and
//! answers it exactly: **which version of the format is this file?** A file
//! of a version this reader does not read is an explicit error that names
//! what it found -- never a partial read.
//!
//! ```
//! use undwg::{detect_version, Version};
//!
//! let header = b"AC1018\0\0\0\0\0\0\x01";
//! assert_eq!(detect_version(header), Ok(Version::R2004));
//! ```

#![doc(html_root_url = "https://docs.rs/undwg")]

mod version;

pub use version::{detect_version, Error, Unsupported, Version};
