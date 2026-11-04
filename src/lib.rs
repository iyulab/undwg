//! A pure-Rust reader for the DWG file format, written from the public file
//! specification and real files.
//!
//! The reader is built layer by layer, each layer checked on its own before
//! the next one is trusted. So far two layers exist: [`detect_version`] says
//! **which version of the format a file is**, and [`read_header`] reads the
//! **file header** that says where the rest of the file is. A file of a
//! version this reader does not read, or a header whose checksum does not
//! match, is an explicit error that names what it found -- never a partial
//! read.
//!
//! ```
//! use undwg::{detect_version, Version};
//!
//! let header = b"AC1018\0\0\0\0\0\0\x01";
//! assert_eq!(detect_version(header), Ok(Version::R2004));
//! ```

#![doc(html_root_url = "https://docs.rs/undwg")]

mod bits;
mod checksum;
mod classes;
mod entities;
mod header;
mod lz;
mod objects;
mod sections;
mod tables;
mod version;

pub use bits::{BitError, BitReader, EndOfData, HandleRef, TextValue};
pub use classes::{read_classes, ClassDef, ClassError};
pub use entities::{
    read_entity, Arc, Attribute, Circle, CommonEntity, Dimension, DimensionKind, Ellipse, Entity,
    Face3d, Geometry, Insert, Line, LwPolyline, LwVertex, Mtext, Point, References, Solid, Spline,
    Text, TYPE_3DFACE, TYPE_ARC, TYPE_ATTDEF, TYPE_ATTRIB, TYPE_CIRCLE, TYPE_DIMENSION_ALIGNED,
    TYPE_DIMENSION_ANGULAR_2LINE, TYPE_DIMENSION_ANGULAR_3POINT, TYPE_DIMENSION_DIAMETER,
    TYPE_DIMENSION_LINEAR, TYPE_DIMENSION_ORDINATE, TYPE_DIMENSION_RADIUS, TYPE_ELLIPSE,
    TYPE_INSERT, TYPE_LINE, TYPE_LWPOLYLINE, TYPE_MTEXT, TYPE_POINT, TYPE_SOLID, TYPE_SPLINE,
    TYPE_TEXT,
};
pub use header::{
    read_header, FileHeader, HeaderError, HeaderR2000, HeaderR2004, SectionLocator,
    SECTION_PAGE_MAP_BIAS,
};
pub use lz::LzError;
pub use objects::{read_object_frame, read_object_map, ObjectEntry, ObjectError, ObjectFrame};
pub use sections::{
    read_data_page, read_directory, read_section, DataPageHeader, PageMapEntry, SectionDescription,
    SectionDirectory, SectionError, SectionPage,
};
pub use tables::{read_block_name, read_dimstyle_name, TYPE_BLOCK_HEADER, TYPE_DIMSTYLE};
pub use version::{detect_version, Error, Unsupported, Version};
