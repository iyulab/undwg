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
use crate::r2007::{read_directory_r2007, read_section_r2007};
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
    /// The template (which holds `MEASUREMENT`) is not what the format documents.
    Template,
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
            DrawingError::Template => {
                write!(f, "the template section is not what the format documents")
            }
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
    /// The code page number the file header declares (see [`CodePage`](crate::CodePage)).
    code_page: u16,
    /// The bytes object positions count from: the whole file for R2000, the
    /// assembled object section from R2004.
    objects: Cow<'a, [u8]>,
    entries: Vec<ObjectEntry>,
    classes: Vec<ClassDef>,
    variables: HeaderVariables,
    measurement: Option<u16>,
}

/// `MEASUREMENT` from the template: a description (a length, then that much
/// text), then the variable as an unsigned short. The length counts bytes up to
/// R2004 and characters of two bytes from R2007.
fn read_measurement(template: &[u8], version: Version) -> Result<u16, DrawingError> {
    let unit = if version >= Version::R2007 { 2 } else { 1 };
    let length = template
        .get(..2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or(DrawingError::Template)?;
    let at = length
        .checked_mul(unit)
        .and_then(|n| n.checked_add(2))
        .ok_or(DrawingError::Template)?;
    template
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(DrawingError::Template)
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
                let measurement = header
                    .section(bytes, 4)?
                    .map(|t| read_measurement(t, Version::R2000))
                    .transpose()?;
                Ok(Drawing {
                    version: Version::R2000,
                    code_page: header.code_page,
                    objects: Cow::Borrowed(bytes),
                    entries: read_object_map(map)?,
                    classes: read_classes(classes, Version::R2000)?,
                    variables: read_header_variables(variables, Version::R2000)?,
                    measurement,
                })
            }
            FileHeader::R2007(header) => {
                let directory = read_directory_r2007(bytes, &header)?;
                let get = |name: &'static str| {
                    read_section_r2007(bytes, &directory, name)?.ok_or(DrawingError::Missing(name))
                };
                let map = get("AcDb:Handles")?;
                let classes = get("AcDb:Classes")?;
                let variables = get("AcDb:Header")?;
                let measurement = read_section_r2007(bytes, &directory, "AcDb:Template")?
                    .map(|t| read_measurement(&t, Version::R2007))
                    .transpose()?;
                Ok(Drawing {
                    version: Version::R2007,
                    code_page: header.code_page,
                    objects: Cow::Owned(get("AcDb:AcDbObjects")?),
                    entries: read_object_map(&map)?,
                    classes: read_classes(&classes, Version::R2007)?,
                    variables: read_header_variables(&variables, Version::R2007)?,
                    measurement,
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
                let measurement = read_section(bytes, &directory, "AcDb:Template")?
                    .map(|t| read_measurement(&t, version))
                    .transpose()?;
                Ok(Drawing {
                    version,
                    code_page: header.code_page,
                    objects: Cow::Owned(get("AcDb:AcDbObjects")?),
                    entries: read_object_map(&map)?,
                    classes: read_classes(&classes, version)?,
                    variables: read_header_variables(&variables, version)?,
                    measurement,
                })
            }
        }
    }

    /// The version the file is.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The code page number the file header declares. Text before R2007 is bytes in this page; which page a
    /// number names is [`CodePage::of`](crate::CodePage::of).
    pub fn code_page(&self) -> u16 {
        self.code_page
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

    /// `MEASUREMENT` (0 English, 1 metric), which the file keeps in its template
    /// rather than in the header variables; `None` when the file has no template.
    pub fn measurement(&self) -> Option<u16> {
        self.measurement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurement_follows_the_description() {
        let old = Version::R2004;
        // No description, metric.
        assert_eq!(read_measurement(&[0, 0, 1, 0], old), Ok(1));
        // A two-byte description, English.
        assert_eq!(read_measurement(&[2, 0, 0xAA, 0xBB, 0, 0], old), Ok(0));
        // From R2007 the length counts characters of two bytes: one character, metric.
        assert_eq!(read_measurement(&[1, 0, 0, 0, 1, 0], Version::R2013), Ok(1));
        // A description longer than the section, or no room for the variable.
        assert_eq!(
            read_measurement(&[9, 0, 1, 0], old),
            Err(DrawingError::Template)
        );
        assert_eq!(
            read_measurement(&[0, 0, 1], old),
            Err(DrawingError::Template)
        );
        assert_eq!(read_measurement(&[], old), Err(DrawingError::Template));
    }
}
