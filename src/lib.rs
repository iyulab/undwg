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
mod codepage;
mod convert;
mod drawing;
mod entities;
mod header;
mod lz;
mod multileader;
mod objects;
mod r2007;
mod sections;
mod tables;
mod variables;
mod version;

pub use bits::{BitError, BitReader, EndOfData, HandleRef, TextValue};
pub use classes::{read_classes, ClassDef, ClassError};
pub use codepage::CodePage;
pub use convert::{read_database, DatabaseError};
pub use drawing::{Drawing, DrawingError};
pub use entities::{
    read_entity, read_entity_links, read_entity_with_classes, Arc, Attribute, Circle, CommonEntity,
    Dimension, DimensionKind, Ellipse, Entity, EntityLinks, Face3d, Geometry, Hatch, HatchEdge,
    HatchGradient, HatchPath, HatchPatternLine, Image, InfiniteLine, Insert, Leader, Line,
    LwPolyline, LwVertex, Mline, MlineVertex, Mtext, Point, Polyline2d, Polyline3d, PolylineMesh,
    PolylinePface, References, Solid, Spline, Text, Tolerance, Vertex2d, Vertex3d, VertexFace,
    Viewport, TYPE_3DFACE, TYPE_ARC, TYPE_ATTDEF, TYPE_ATTRIB, TYPE_CIRCLE, TYPE_DIMENSION_ALIGNED,
    TYPE_DIMENSION_ANGULAR_2LINE, TYPE_DIMENSION_ANGULAR_3POINT, TYPE_DIMENSION_DIAMETER,
    TYPE_DIMENSION_LINEAR, TYPE_DIMENSION_ORDINATE, TYPE_DIMENSION_RADIUS, TYPE_ELLIPSE,
    TYPE_INSERT, TYPE_LEADER, TYPE_LINE, TYPE_LWPOLYLINE, TYPE_MLINE, TYPE_MTEXT, TYPE_POINT,
    TYPE_POLYLINE_2D, TYPE_POLYLINE_3D, TYPE_POLYLINE_MESH, TYPE_POLYLINE_PFACE, TYPE_RAY,
    TYPE_SOLID, TYPE_SPLINE, TYPE_TEXT, TYPE_TOLERANCE, TYPE_TRACE, TYPE_VERTEX_2D, TYPE_VERTEX_3D,
    TYPE_VERTEX_MESH, TYPE_VERTEX_PFACE, TYPE_VERTEX_PFACE_FACE, TYPE_VIEWPORT, TYPE_XLINE,
};
pub use header::{
    read_header, FileHeader, HeaderError, HeaderR2000, HeaderR2004, SectionLocator,
    SECTION_PAGE_MAP_BIAS,
};
pub use lz::LzError;
pub use multileader::{
    MultiLeader, MultiLeaderArrowHead, MultiLeaderBlock, MultiLeaderBreak, MultiLeaderColor,
    MultiLeaderContent, MultiLeaderLine, MultiLeaderRoot, MultiLeaderText,
};
pub use objects::{
    read_object_frame, read_object_map, ExtendedData, ObjectEntry, ObjectError, ObjectFrame,
};
pub use r2007::{
    read_directory_r2007, read_header_r2007, read_section_r2007, Directory2007, HeaderR2007,
    PageMapEntry2007, Section2007, SectionPage2007, PAGES_START,
};
pub use sections::{
    read_data_page, read_directory, read_section, DataPageHeader, PageMapEntry, SectionDescription,
    SectionDirectory, SectionError, SectionPage,
};
pub use tables::{
    read_appid_name, read_block_control, read_block_header, read_block_name, read_dim_style,
    read_dimstyle_name, read_image_definition, read_layer, read_layout, read_linetype_name,
    read_mleader_style, read_mline_style, read_mlinestyle_name, read_style_name, BlockHeader,
    DimStyle, ImageDefinition, Layer, MleaderStyle, MlineElement, MlineStyle, PlotLayout,
    TableColor, TYPE_APPID, TYPE_BLOCK_CONTROL, TYPE_BLOCK_HEADER, TYPE_DIMSTYLE, TYPE_LAYER,
    TYPE_LINETYPE, TYPE_MLINESTYLE, TYPE_STYLE,
};
pub use variables::{
    read_header_variables, ControlHandles, DimensionVariables, ExtendedVariables, HeaderColor,
    HeaderVariables, SpaceVariables, Timestamp, VariablesError,
};
pub use version::{detect_version, Error, Unsupported, Version};
