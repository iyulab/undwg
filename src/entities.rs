//! Entities: the objects that are drawn.
//!
//! An entity begins with data every entity has -- its handle, extended data,
//! a flag for a stored graphic, colour, linetype scale and so on -- and then
//! the fields of its own kind. This module reads the common data and the
//! simplest kinds: lines, circles, arcs, points, ellipses, lightweight
//! polylines and block insertions. Any other kind is left
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
/// The type code of a point.
pub const TYPE_POINT: u16 = 27;
/// The type code of a block insertion.
pub const TYPE_INSERT: u16 = 7;
/// The type code of an ellipse.
pub const TYPE_ELLIPSE: u16 = 35;
/// The type code of a lightweight polyline.
pub const TYPE_LWPOLYLINE: u16 = 77;

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

/// A point.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// The position.
    pub position: [f64; 3],
    /// The thickness.
    pub thickness: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The angle of the x axis, in radians.
    pub x_axis_angle: f64,
}

/// An ellipse or an elliptical arc. The centre and the major axis vector are
/// in world coordinates, though the ellipse is planar and has an extrusion.
#[derive(Debug, Clone, PartialEq)]
pub struct Ellipse {
    /// The centre.
    pub center: [f64; 3],
    /// The semi-major axis as a vector from the centre.
    pub major_axis: [f64; 3],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The ratio of the minor axis to the major axis.
    pub axis_ratio: f64,
    /// The starting angle, as an eccentric anomaly in radians.
    pub start_angle: f64,
    /// The ending angle, as an eccentric anomaly in radians.
    pub end_angle: f64,
}

/// A block insertion. The block it inserts is named by a handle in the
/// object's handle stream, which this reader does not read.
#[derive(Debug, Clone, PartialEq)]
pub struct Insert {
    /// The insertion point.
    pub position: [f64; 3],
    /// The scale along each axis.
    pub scale: [f64; 3],
    /// The rotation, in radians.
    pub rotation: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// Whether attributes follow the insertion.
    pub has_attributes: bool,
}

/// One vertex of a lightweight polyline.
#[derive(Debug, Clone, PartialEq)]
pub struct LwVertex {
    /// The vertex, in the polyline's own plane.
    pub point: [f64; 2],
    /// The bulge of the segment that leaves the vertex (`0` when none is stated).
    pub bulge: f64,
    /// The width at the start of the segment that leaves the vertex (`0` when
    /// the polyline states none of its own).
    pub start_width: f64,
    /// The width at the end of that segment.
    pub end_width: f64,
}

/// A lightweight polyline: vertices in one plane, each with the bulge and the
/// widths of the segment leaving it.
#[derive(Debug, Clone, PartialEq)]
pub struct LwPolyline {
    /// The polyline flag word, as stored.
    pub flags: i16,
    /// Whether the last vertex joins back to the first (flag bit 512).
    pub closed: bool,
    /// The width every segment has when no vertex states a width of its own
    /// (`0` when none is stated).
    pub const_width: f64,
    /// The elevation of the plane (`0` when none is stated).
    pub elevation: f64,
    /// The thickness (`0` when none is stated).
    pub thickness: f64,
    /// The normal of the plane (`0, 0, 1` when none is stated).
    pub normal: [f64; 3],
    /// The vertices in order.
    pub vertices: Vec<LwVertex>,
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
    /// A point.
    Point(Point),
    /// A lightweight polyline.
    LwPolyline(LwPolyline),
    /// A block insertion.
    Insert(Insert),
    /// An ellipse.
    Ellipse(Ellipse),
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

/// A count the stream states, bounded by what is left to read: every item
/// takes at least one bit, so a count above the bits left is damaged data and
/// must not allocate.
fn bounded_count(r: &mut BitReader<'_>, what: &'static str) -> Result<usize, BitError> {
    let n = r.bit_long()?;
    usize::try_from(n)
        .ok()
        .filter(|&n| n <= r.remaining())
        .ok_or(BitError::TooLong { form: what })
}

fn read_lwpolyline(r: &mut BitReader<'_>, version: Version) -> Result<LwPolyline, BitError> {
    let flags = r.bit_short()?;
    let bits = flags as u16;
    let const_width = if bits & 4 != 0 { r.bit_double()? } else { 0.0 };
    let elevation = if bits & 8 != 0 { r.bit_double()? } else { 0.0 };
    let thickness = if bits & 2 != 0 { r.bit_double()? } else { 0.0 };
    let normal = if bits & 1 != 0 {
        r.point_3bd()?
    } else {
        [0.0, 0.0, 1.0]
    };
    let count = bounded_count(r, "polyline vertex count")?;
    let bulge_count = if bits & 16 != 0 {
        bounded_count(r, "polyline bulge count")?
    } else {
        0
    };
    let id_count = if version >= Version::R2010 && bits & 1024 != 0 {
        bounded_count(r, "polyline vertex id count")?
    } else {
        0
    };
    let width_count = if bits & 32 != 0 {
        bounded_count(r, "polyline width count")?
    } else {
        0
    };

    let mut vertices: Vec<LwVertex> = Vec::with_capacity(count);
    for i in 0..count {
        let point = if i == 0 {
            [r.raw_double()?, r.raw_double()?]
        } else {
            // Each later vertex is a difference-coded double pair: the previous
            // vertex stands in for what the stream leaves out.
            let previous = vertices[i - 1].point;
            let x = r.bit_double_with_default(previous[0])?;
            let y = r.bit_double_with_default(previous[1])?;
            [x, y]
        };
        vertices.push(LwVertex {
            point,
            bulge: 0.0,
            start_width: 0.0,
            end_width: 0.0,
        });
    }
    for i in 0..bulge_count {
        let bulge = r.bit_double()?;
        if let Some(v) = vertices.get_mut(i) {
            v.bulge = bulge;
        }
    }
    for _ in 0..id_count {
        r.bit_long()?; // a vertex identifier, not needed for geometry
    }
    for i in 0..width_count {
        let (start, end) = (r.bit_double()?, r.bit_double()?);
        if let Some(v) = vertices.get_mut(i) {
            v.start_width = start;
            v.end_width = end;
        }
    }
    Ok(LwPolyline {
        flags,
        closed: bits & 512 != 0,
        const_width,
        elevation,
        thickness,
        normal,
        vertices,
    })
}

fn read_ellipse(r: &mut BitReader<'_>) -> Result<Ellipse, BitError> {
    Ok(Ellipse {
        center: r.point_3bd()?,
        major_axis: r.point_3bd()?,
        extrusion: r.point_3bd()?,
        axis_ratio: r.bit_double()?,
        start_angle: r.bit_double()?,
        end_angle: r.bit_double()?,
    })
}

fn read_insert(r: &mut BitReader<'_>) -> Result<Insert, BitError> {
    let position = r.point_3bd()?;
    // How the scale is stored: all ones, a value with the other two defaulted
    // to one, one value for all three, or one value and two defaulting to it.
    let scale = match r.bits(2)? {
        3 => [1.0, 1.0, 1.0],
        1 => [
            1.0,
            r.bit_double_with_default(1.0)?,
            r.bit_double_with_default(1.0)?,
        ],
        2 => {
            let x = r.raw_double()?;
            [x, x, x]
        }
        _ => {
            let x = r.raw_double()?;
            [
                x,
                r.bit_double_with_default(x)?,
                r.bit_double_with_default(x)?,
            ]
        }
    };
    Ok(Insert {
        position,
        scale,
        rotation: r.bit_double()?,
        extrusion: r.point_3bd()?,
        has_attributes: r.bit()?,
    })
}

fn read_point(r: &mut BitReader<'_>) -> Result<Point, BitError> {
    Ok(Point {
        position: r.point_3bd()?,
        thickness: r.bit_thickness()?,
        extrusion: r.bit_extrusion()?,
        x_axis_angle: r.bit_double()?,
    })
}

/// Reads the entity at `entry` if it is a line, a circle, an arc, a point, an
/// ellipse, a lightweight polyline or a block insertion;
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
    if !matches!(
        frame.type_code,
        TYPE_ARC
            | TYPE_CIRCLE
            | TYPE_LINE
            | TYPE_POINT
            | TYPE_LWPOLYLINE
            | TYPE_INSERT
            | TYPE_ELLIPSE
    ) {
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
        TYPE_POINT => Geometry::Point(read_point(&mut r)?),
        TYPE_CIRCLE => Geometry::Circle(read_circle(&mut r)?),
        TYPE_LWPOLYLINE => Geometry::LwPolyline(read_lwpolyline(&mut r, version)?),
        TYPE_INSERT => Geometry::Insert(read_insert(&mut r)?),
        TYPE_ELLIPSE => Geometry::Ellipse(read_ellipse(&mut r)?),
        _ => Geometry::Arc(read_arc(&mut r)?),
    };
    Ok(Some(Entity { common, geometry }))
}
