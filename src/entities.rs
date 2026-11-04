//! Entities: the objects that are drawn.
//!
//! An entity begins with data every entity has -- its handle, extended data,
//! a flag for a stored graphic, colour, linetype scale and so on -- and then
//! the fields of its own kind. This module reads the common data and the
//! three simplest kinds: lines, circles and arcs. Any other kind is left
//! alone (`Ok(None)`), never guessed at.
//!
//! Only the main bit stream is read; the strings and handle references that
//! later versions keep in separate streams are not needed for geometry.

use crate::bits::{BitError, BitReader};
use crate::objects::{open_object, ObjectEntry, ObjectError};
use crate::version::Version;

/// The type code of an arc.
pub const TYPE_ARC: u16 = 17;
/// The type code of a circle.
pub const TYPE_CIRCLE: u16 = 18;
/// The type code of a line.
pub const TYPE_LINE: u16 = 19;

/// What every entity states about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct CommonEntity {
    /// The entity's handle, as it states it.
    pub handle: u64,
    /// Where the entity lives: `0` with an owner reference, `1` paper space,
    /// `2` model space (`3` is not used).
    pub entity_mode: u8,
    /// How many persistent reactors are attached.
    pub reactor_count: i32,
    /// The colour number with its flags, as stored.
    pub color: u16,
    /// The linetype scale.
    pub linetype_scale: f64,
    /// `0` by layer, `1` by block, `2` continuous, `3` a linetype handle is stored.
    pub linetype_flags: u8,
    /// `0` by layer, `1` by block, `3` a plot style handle is stored.
    pub plotstyle_flags: u8,
    /// Bit 0: `1` when the entity is invisible.
    pub invisible: i16,
    /// The line weight code (R2000 and later).
    pub lineweight: u8,
}

/// A line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The start point.
    pub start: [f64; 3],
    /// The end point.
    pub end: [f64; 3],
    /// The thickness.
    pub thickness: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
}

/// A circle.
#[derive(Debug, Clone, PartialEq)]
pub struct Circle {
    /// The centre.
    pub center: [f64; 3],
    /// The radius.
    pub radius: f64,
    /// The thickness.
    pub thickness: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
}

/// An arc.
#[derive(Debug, Clone, PartialEq)]
pub struct Arc {
    /// The centre.
    pub center: [f64; 3],
    /// The radius.
    pub radius: f64,
    /// The thickness.
    pub thickness: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The start angle, in radians.
    pub start_angle: f64,
    /// The end angle, in radians.
    pub end_angle: f64,
}

/// The geometry of an entity of a kind this reader reads.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Geometry {
    /// A line.
    Line(Line),
    /// A circle.
    Circle(Circle),
    /// An arc.
    Arc(Arc),
}

/// An entity: its common data and its geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// What every entity states.
    pub common: CommonEntity,
    /// The entity's own fields.
    pub geometry: Geometry,
}

/// Reads the common entity data. The reader is positioned just after the
/// object type.
fn read_common(r: &mut BitReader<'_>, version: Version) -> Result<CommonEntity, BitError> {
    let before_r2010 = version < Version::R2010;
    if before_r2010 {
        // The size of the object data in bits, not counting the handles.
        r.raw_long()?;
    }
    let handle = r.handle()?.value;

    // Extended data: sizes, each followed by an application handle and that
    // many bytes, until a size of zero.
    let mut size = r.bit_short()?;
    while size > 0 {
        r.handle()?;
        for _ in 0..size {
            r.raw_char()?;
        }
        size = r.bit_short()?;
    }

    // A stored graphic, skipped.
    if r.bit()? {
        let bytes = if before_r2010 {
            u64::from(r.raw_long()?)
        } else {
            r.bit_long_long()?
        };
        // Bounded by what is left: a size that cannot fit is damaged data.
        if bytes.saturating_mul(8) > r.remaining() as u64 {
            return Err(BitError::End(crate::bits::EndOfData {
                at_bit: r.position(),
                needed_bits: usize::try_from(bytes.saturating_mul(8)).unwrap_or(usize::MAX),
            }));
        }
        for _ in 0..bytes {
            r.raw_char()?;
        }
    }

    let entity_mode = r.bits(2)? as u8;
    let reactor_count = r.bit_long()?;
    if version >= Version::R2004 {
        r.bit()?; // no extension dictionary handle is stored
    }
    if version >= Version::R2013 {
        r.bit()?; // binary data is kept in the data store
    }
    if version < Version::R2004 {
        r.bit()?; // no previous/next links (from R2004 the flag is implied, not stored)
    }

    let color = if version >= Version::R2004 {
        let flags = r.bit_short()? as u16;
        if flags & 0x8000 != 0 {
            r.bit_long()?; // an RGB value
        }
        if flags & 0x2000 != 0 {
            r.bit_long()?; // a transparency
        }
        flags
    } else {
        r.bit_short()? as u16
    };
    let linetype_scale = r.bit_double()?;
    let linetype_flags = r.bits(2)? as u8;
    let plotstyle_flags = r.bits(2)? as u8;
    if version >= Version::R2007 {
        r.bits(2)?; // material flags
        r.raw_char()?; // shadow flags
    }
    if version >= Version::R2010 {
        r.bit()?; // has a full visual style
        r.bit()?; // has a face visual style
        r.bit()?; // has an edge visual style
    }
    let invisible = r.bit_short()?;
    let lineweight = r.raw_char()?;
    Ok(CommonEntity {
        handle,
        entity_mode,
        reactor_count,
        color,
        linetype_scale,
        linetype_flags,
        plotstyle_flags,
        invisible,
        lineweight,
    })
}

fn read_line(r: &mut BitReader<'_>) -> Result<Line, BitError> {
    let z_is_zero = r.bit()?;
    let x1 = r.raw_double()?;
    let x2 = r.bit_double_with_default(x1)?;
    let y1 = r.raw_double()?;
    let y2 = r.bit_double_with_default(y1)?;
    let (z1, z2) = if z_is_zero {
        (0.0, 0.0)
    } else {
        let z1 = r.raw_double()?;
        (z1, r.bit_double_with_default(z1)?)
    };
    Ok(Line {
        start: [x1, y1, z1],
        end: [x2, y2, z2],
        thickness: r.bit_thickness()?,
        extrusion: r.bit_extrusion()?,
    })
}

fn read_circle(r: &mut BitReader<'_>) -> Result<Circle, BitError> {
    Ok(Circle {
        center: r.point_3bd()?,
        radius: r.bit_double()?,
        thickness: r.bit_thickness()?,
        extrusion: r.bit_extrusion()?,
    })
}

fn read_arc(r: &mut BitReader<'_>) -> Result<Arc, BitError> {
    Ok(Arc {
        center: r.point_3bd()?,
        radius: r.bit_double()?,
        thickness: r.bit_thickness()?,
        extrusion: r.bit_extrusion()?,
        start_angle: r.bit_double()?,
        end_angle: r.bit_double()?,
    })
}

/// Reads the entity at `entry` if it is a line, a circle or an arc;
/// `Ok(None)` for any other kind of object.
///
/// `objects` is the assembled `AcDb:AcDbObjects` section (for R2000 the
/// whole file). The object's frame must be intact; the handle it states must
/// be the handle the object map gives.
pub fn read_entity(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<Entity>, ObjectError> {
    let (frame, mut r) = open_object(objects, entry, version)?;
    if !matches!(frame.type_code, TYPE_ARC | TYPE_CIRCLE | TYPE_LINE) {
        return Ok(None);
    }
    let common = read_common(&mut r, version)?;
    if common.handle != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "entity handle",
        });
    }
    let geometry = match frame.type_code {
        TYPE_LINE => Geometry::Line(read_line(&mut r)?),
        TYPE_CIRCLE => Geometry::Circle(read_circle(&mut r)?),
        _ => Geometry::Arc(read_arc(&mut r)?),
    };
    Ok(Some(Entity { common, geometry }))
}
