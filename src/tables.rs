//! Table entries: the named objects that entities refer to.
//!
//! Only the name of a block header is read so far -- enough to say which block
//! a block insertion inserts.

use crate::bits::{BitError, BitReader, TextValue};
use crate::objects::{open_object, skip_extended_data, ObjectEntry, ObjectError};
use crate::version::Version;

/// The type code of a block header.
pub const TYPE_BLOCK_HEADER: u16 = 49;

/// The type code of a dimension style.
pub const TYPE_DIMSTYLE: u16 = 69;

/// Reads the name of the block header at `entry`; `Ok(None)` when the object
/// is of another kind.
///
/// The name is the first field after the data every object has. It is in the
/// main stream before R2007 and in the string stream from R2007.
pub fn read_block_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_BLOCK_HEADER)
}

/// Reads the name of the dimension style at `entry`; `Ok(None)` when the
/// object is of another kind.
pub fn read_dimstyle_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_DIMSTYLE)
}

fn read_table_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
    type_code: u16,
) -> Result<Option<TextValue>, ObjectError> {
    let (frame, mut r, layout) = open_object(objects, entry, version)?;
    if frame.type_code != type_code {
        return Ok(None);
    }
    if version < Version::R2010 {
        r.raw_long()?; // the size of the main data in bits
    }
    if r.handle()?.value != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "table entry handle",
        });
    }
    skip_extended_data(&mut r)?;
    r.bit_long()?; // the number of persistent reactors
    if version >= Version::R2004 {
        r.bit()?; // no extension dictionary handle is stored
    }
    if version >= Version::R2013 {
        r.bit()?; // binary data is kept in the data store
    }
    let name = match layout.strings {
        Some((from, to)) => {
            let mut s = r.clone();
            s.seek(from);
            let name = s.text(true)?;
            if s.position() > to {
                return Err(ObjectError::Unexpected {
                    what: "table entry string length",
                });
            }
            name
        }
        None => r.text(false)?,
    };
    Ok(Some(name))
}

/// The type code of a layer.
pub const TYPE_LAYER: u16 = 51;

/// The type code of a linetype.
pub const TYPE_LINETYPE: u16 = 57;

/// Reads the name of the linetype at `entry`; `Ok(None)` when the object is of
/// another kind.
pub fn read_linetype_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_LINETYPE)
}

/// A colour as a table entry states it (R2000 and later): the way it is given
/// and its value. R2000 states an AutoCAD colour number. From R2004 a long
/// holds the way in its top byte -- `0xC0` by layer, `0xC1` by block, `0xC2` a
/// true colour in the low 24 bits, `0xC3` a colour number in the low byte,
/// `0xC8` none -- and the value in the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableColor {
    /// `0xC0` by layer, `0xC1` by block, `0xC2` true colour, `0xC3` colour
    /// number, `0xC8` none; for R2000 always `0xC3`.
    pub method: u8,
    /// The colour number, or the 24-bit RGB value of a true colour.
    pub value: u32,
}

/// A layer: a table entry that entities refer to by handle.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The layer's handle.
    pub handle: u64,
    /// The layer name.
    pub name: TextValue,
    /// The flags as stored: frozen `1`, off `2`, frozen in new viewports `4`,
    /// locked `8`, plot `16`; bits 5 to 9 (`0x03E0`) hold the line weight code.
    pub flags: u16,
    /// The layer colour.
    pub color: TableColor,
    /// The layer control object that owns the entry.
    pub owner: Option<u64>,
    /// The external reference block the layer comes from, if it is dependent.
    pub xref_block: Option<u64>,
    /// The plot style.
    pub plotstyle: Option<u64>,
    /// The material (R2007 and later).
    pub material: Option<u64>,
    /// The linetype.
    pub linetype: Option<u64>,
}

impl Layer {
    /// The line weight code, as stored in bits 5 to 9 of the flags.
    pub fn lineweight(&self) -> u8 {
        ((self.flags & 0x03E0) >> 5) as u8
    }
}

/// Reads the layer at `entry`; `Ok(None)` when the object is of another kind.
///
/// Fields: name, flags (short), colour; handles: the owning layer control,
/// reactors, the extension dictionary, the external reference block, the plot
/// style, the material (R2007 and later), the linetype. The main stream must
/// end exactly where the colour does.
pub fn read_layer(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<Layer>, ObjectError> {
    let (frame, mut r, layout) = open_object(objects, entry, version)?;
    if frame.type_code != TYPE_LAYER {
        return Ok(None);
    }
    if version < Version::R2010 {
        r.raw_long()?; // the size of the main data in bits
    }
    if r.handle()?.value != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "layer handle",
        });
    }
    skip_extended_data(&mut r)?;
    let reactors = r.bit_long()?;
    let xdictionary_stored = version < Version::R2004 || !r.bit()?;
    if version >= Version::R2013 {
        r.bit()?; // binary data is kept in the data store
    }
    let mut strings = layout.strings.map(|(from, _)| {
        let mut s = r.clone();
        s.seek(from);
        s
    });
    let name = text_value(&mut r, &mut strings)?;
    if version >= Version::R2007 {
        r.bit_short()?; // the external reference index plus one
    } else {
        r.bit()?; // the 64-bit of the DXF flags
        r.bit_short()?; // the external reference index plus one
        r.bit()?; // the layer depends on an external reference
    }
    let flags = r.bit_short()? as u16;
    let color = if version >= Version::R2004 {
        r.bit_short()?; // always zero
        let rgb = r.bit_long()? as u32;
        let names = r.raw_char()?;
        if names & 1 != 0 {
            text_value(&mut r, &mut strings)?; // the colour name
        }
        if names & 2 != 0 {
            text_value(&mut r, &mut strings)?; // the colour book name
        }
        TableColor {
            method: (rgb >> 24) as u8,
            value: rgb & 0x00FF_FFFF,
        }
    } else {
        TableColor {
            method: 0xC3,
            value: r.bit_short()? as u16 as u32,
        }
    };
    if r.position() != layout.main_end {
        return Err(ObjectError::Unexpected {
            what: "layer main data length",
        });
    }
    let mut h = r.clone();
    h.seek(layout.handle_start);
    let holder = entry.handle;
    let next = |h: &mut BitReader<'_>| -> Result<Option<u64>, ObjectError> {
        Ok(h.handle()?.resolve(holder).filter(|&v| v != 0))
    };
    let owner = next(&mut h)?;
    let reactor_count = usize::try_from(reactors)
        .ok()
        .filter(|&n| n <= h.remaining() / 8)
        .ok_or(ObjectError::Unexpected {
            what: "reactor count",
        })?;
    for _ in 0..reactor_count {
        next(&mut h)?;
    }
    if xdictionary_stored {
        next(&mut h)?;
    }
    let xref_block = next(&mut h)?;
    let plotstyle = next(&mut h)?;
    let material = if version >= Version::R2007 {
        next(&mut h)?
    } else {
        None
    };
    let linetype = next(&mut h)?;
    Ok(Some(Layer {
        handle: entry.handle,
        name,
        flags,
        color,
        owner,
        xref_block,
        plotstyle,
        material,
        linetype,
    }))
}

/// Reads a text from the string stream when there is one, otherwise from the
/// main stream.
fn text_value(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
) -> Result<TextValue, BitError> {
    match strings {
        Some(s) => s.text(true),
        None => r.text(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(flags: u16) -> Layer {
        Layer {
            handle: 0x10,
            name: TextValue::Ansi(b"0".to_vec()),
            flags,
            color: TableColor {
                method: 0xC3,
                value: 7,
            },
            owner: None,
            xref_block: None,
            plotstyle: None,
            material: None,
            linetype: None,
        }
    }

    #[test]
    fn the_line_weight_code_is_bits_five_to_nine_of_the_flags() {
        // Plot flag (16) and the default line weight code (31).
        assert_eq!(layer(0x03F0).lineweight(), 31);
        // Frozen (1) and locked (8) leave the code alone.
        assert_eq!(layer(0x0009 | (8 << 5)).lineweight(), 8);
        assert_eq!(layer(0).lineweight(), 0);
    }
}
