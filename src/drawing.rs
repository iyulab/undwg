//! An opened DWG file: the sections every reader of its contents works from.
//!
//! [`Drawing::open`] checks the version, the file header, the class
//! definitions, the header variables and the object map, and keeps the bytes
//! the objects are read from. It reads no object itself; the functions that
//! do ([`read_entity`](crate::read_entity), [`read_layer`](crate::read_layer)
//! and the others) take the pieces this type holds.

use std::borrow::Cow;
use std::fmt;

use crate::classes::{read_classes, ClassDef, ClassError};
use crate::header::{read_header, FileHeader, HeaderError};
use crate::objects::{read_object_map, ObjectEntry, ObjectError};
use crate::sections::{read_directory, read_section, SectionError};
use crate::variables::{read_header_variables, HeaderVariables, VariablesError};
use crate::version::Version;

/// Why a file could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrawingError {
    /// The file header could not be read, or the version is not one this
    /// reader reads.
    Header(HeaderError),
    /// A section could not be found or assembled.
    Section(SectionError),
    /// A section the format requires is not in the file.
    Missing(&'static str),
    /// The class definitions could not be read.
    Classes(ClassError),
    /// The header variables could not be read.
    Variables(VariablesError),
    /// The object map could not be read.
    ObjectMap(ObjectError),
}

impl fmt::Display for DrawingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawingError::Header(e) => e.fmt(f),
            DrawingError::Section(e) => e.fmt(f),
            DrawingError::Missing(what) => write!(f, "the file has no {what} section"),
            DrawingError::Classes(e) => e.fmt(f),
            DrawingError::Variables(e) => e.fmt(f),
            DrawingError::ObjectMap(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DrawingError {}

impl From<HeaderError> for DrawingError {
    fn from(e: HeaderError) -> Self {
        DrawingError::Header(e)
    }
}

impl From<SectionError> for DrawingError {
    fn from(e: SectionError) -> Self {
        DrawingError::Section(e)
    }
}

impl From<ClassError> for DrawingError {
    fn from(e: ClassError) -> Self {
        DrawingError::Classes(e)
    }
}

impl From<VariablesError> for DrawingError {
    fn from(e: VariablesError) -> Self {
        DrawingError::Variables(e)
    }
}

impl From<ObjectError> for DrawingError {
    fn from(e: ObjectError) -> Self {
        DrawingError::ObjectMap(e)
    }
}

/// A DWG file whose container has been read.
#[derive(Debug, Clone)]
pub struct Drawing<'a> {
    version: Version,
    /// The bytes object positions count from: the whole file for R2000, the
    /// assembled object section from R2004.
    objects: Cow<'a, [u8]>,
    entries: Vec<ObjectEntry>,
    classes: Vec<ClassDef>,
    variables: HeaderVariables,
}

impl<'a> Drawing<'a> {
    /// Opens the file in `bytes`.
    pub fn open(bytes: &'a [u8]) -> Result<Drawing<'a>, DrawingError> {
        match read_header(bytes)? {
            FileHeader::R2000(header) => {
                let map = header
                    .section(bytes, 2)?
                    .ok_or(DrawingError::Missing("object map"))?;
                let classes = header
                    .section(bytes, 1)?
                    .ok_or(DrawingError::Missing("class"))?;
                let variables = header
                    .section(bytes, 0)?
                    .ok_or(DrawingError::Missing("header variables"))?;
                Ok(Drawing {
                    version: Version::R2000,
                    objects: Cow::Borrowed(bytes),
                    entries: read_object_map(map)?,
                    classes: read_classes(classes, Version::R2000)?,
                    variables: read_header_variables(variables, Version::R2000)?,
                })
            }
            FileHeader::R2004(version, header) => {
                let directory = read_directory(bytes, &header)?;
                let get = |name: &'static str| {
                    read_section(bytes, &directory, name)?.ok_or(DrawingError::Missing(name))
                };
                let map = get("AcDb:Handles")?;
                let classes = get("AcDb:Classes")?;
                let variables = get("AcDb:Header")?;
                Ok(Drawing {
                    version,
                    objects: Cow::Owned(get("AcDb:AcDbObjects")?),
                    entries: read_object_map(&map)?,
                    classes: read_classes(&classes, version)?,
                    variables: read_header_variables(&variables, version)?,
                })
            }
        }
    }

    /// The version the file is.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The bytes objects are read from, with the entries of
    /// [`entries`](Self::entries) counting positions in them.
    pub fn objects(&self) -> &[u8] {
        &self.objects
    }

    /// The objects the file lists, in the order its object map gives them
    /// (ascending handles). An entry may point outside [`objects`](Self::objects);
    /// a reader skips it and says so.
    pub fn entries(&self) -> &[ObjectEntry] {
        &self.entries
    }

    /// The class definitions.
    pub fn classes(&self) -> &[ClassDef] {
        &self.classes
    }

    /// The header variables.
    pub fn variables(&self) -> &HeaderVariables {
        &self.variables
    }
}
