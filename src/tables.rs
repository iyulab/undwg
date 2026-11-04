//! Table entries: the named objects that entities refer to.
//!
//! Only the name of a block header is read so far -- enough to say which block
//! a block insertion inserts.

use crate::bits::TextValue;
use crate::objects::{open_object, skip_extended_data, ObjectEntry, ObjectError};
use crate::version::Version;

/// The type code of a block header.
pub const TYPE_BLOCK_HEADER: u16 = 49;

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
    let (frame, mut r, layout) = open_object(objects, entry, version)?;
    if frame.type_code != TYPE_BLOCK_HEADER {
        return Ok(None);
    }
    if version < Version::R2010 {
        r.raw_long()?; // the size of the main data in bits
    }
    if r.handle()?.value != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "block header handle",
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
                    what: "block header string length",
                });
            }
            name
        }
        None => r.text(false)?,
    };
    Ok(Some(name))
}
