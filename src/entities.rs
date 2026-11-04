//! Entities: the objects that are drawn.
//!
//! An entity begins with data every entity has -- its handle, extended data,
//! a flag for a stored graphic, colour, linetype scale and so on -- and then
//! the fields of its own kind. This module reads the common data and the
//! simplest kinds: lines, circles, arcs, points, ellipses, lightweight
//! polylines, block insertions, solids and 3D faces. Any other kind is left
//! alone (`Ok(None)`), never guessed at.
//!
//! An object's data is laid out in streams that follow one another: the main
//! stream, then (from R2007) the strings, then the handle references. The
//! main stream and the handle stream are read here; a kind's fields must fill
//! the main stream exactly and the handles must reach the end of the data,
//! which is what makes a wrongly positioned read show up.

use crate::bits::{BitError, BitReader, TextValue};
use crate::classes::ClassDef;
use crate::objects::{open_object, read_extended_data, ExtendedData, ObjectEntry, ObjectError};
use crate::version::Version;

/// The type code of an arc.
pub const TYPE_ARC: u16 = 17;
/// The type code of a circle.
pub const TYPE_CIRCLE: u16 = 18;
/// The type code of a line.
pub const TYPE_LINE: u16 = 19;
/// The type code of a point.
pub const TYPE_POINT: u16 = 27;
/// The type code of a text.
pub const TYPE_TEXT: u16 = 1;
/// A type code no object has: it stands for hatches, which have no fixed type
/// code of their own but are named by a class of the drawing.
const TYPE_HATCH: u16 = 0x7001;
const TYPE_IMAGE: u16 = 0x7002;

/// The type code of a trace.
pub const TYPE_TRACE: u16 = 32;
/// The type code of a ray.
pub const TYPE_RAY: u16 = 40;
/// The type code of an infinite line.
pub const TYPE_XLINE: u16 = 41;
/// The type code of a geometric tolerance.
pub const TYPE_TOLERANCE: u16 = 46;
/// The type code of a vertex of a 2D polyline.
pub const TYPE_VERTEX_2D: u16 = 10;
/// The type code of a vertex of a 3D polyline.
pub const TYPE_VERTEX_3D: u16 = 11;
/// The type code of a 2D polyline.
pub const TYPE_POLYLINE_2D: u16 = 15;
/// The type code of a 3D polyline.
pub const TYPE_POLYLINE_3D: u16 = 16;
/// The type code of a spline.
pub const TYPE_SPLINE: u16 = 36;
/// The type code of an ordinate dimension.
pub const TYPE_DIMENSION_ORDINATE: u16 = 20;
/// The type code of a linear (rotated) dimension.
pub const TYPE_DIMENSION_LINEAR: u16 = 21;
/// The type code of an aligned dimension.
pub const TYPE_DIMENSION_ALIGNED: u16 = 22;
/// The type code of a three-point angular dimension.
pub const TYPE_DIMENSION_ANGULAR_3POINT: u16 = 23;
/// The type code of a two-line angular dimension.
pub const TYPE_DIMENSION_ANGULAR_2LINE: u16 = 24;
/// The type code of a radius dimension.
pub const TYPE_DIMENSION_RADIUS: u16 = 25;
/// The type code of a diameter dimension.
pub const TYPE_DIMENSION_DIAMETER: u16 = 26;
/// The type code of a multiline text.
pub const TYPE_MTEXT: u16 = 44;
/// The type code of an attribute.
pub const TYPE_ATTRIB: u16 = 2;
/// The type code of an attribute definition.
pub const TYPE_ATTDEF: u16 = 3;
/// The type code of a block insertion.
pub const TYPE_INSERT: u16 = 7;
/// The type code of an ellipse.
pub const TYPE_ELLIPSE: u16 = 35;
/// The type code of a 3D face.
pub const TYPE_3DFACE: u16 = 28;
/// The type code of a solid.
pub const TYPE_SOLID: u16 = 31;
/// The type code of a lightweight polyline.
pub const TYPE_LWPOLYLINE: u16 = 77;
/// The type code of a viewport.
pub const TYPE_VIEWPORT: u16 = 34;
/// The type code of a leader.
pub const TYPE_LEADER: u16 = 45;
/// The type code of a multiline.
pub const TYPE_MLINE: u16 = 47;

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
    /// The 24-bit RGB value of a true colour (R2004 and later, colour flag `0x8000`).
    pub rgb: Option<u32>,
    /// The transparency as stored (R2004 and later, colour flag `0x2000`): a
    /// long with the type in the top byte and a value in the last byte. Real
    /// files state `1` (by block) and `2` (a value) there; the specification
    /// lists `3` for a value.
    pub transparency: Option<u32>,
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
    /// The extended data attached to the entity, one item per application.
    pub extended_data: Vec<ExtendedData>,
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

/// A flat filled area of three or four corners (a 2D solid).
#[derive(Debug, Clone, PartialEq)]
pub struct Solid {
    /// The thickness.
    pub thickness: f64,
    /// The z of every corner.
    pub elevation: f64,
    /// The four corners in the plane (the third and fourth are the same for
    /// a triangle).
    pub corners: [[f64; 2]; 4],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
}

/// A 3D face of three or four corners.
#[derive(Debug, Clone, PartialEq)]
pub struct Face3d {
    /// The four corners (the fourth is the third for a triangle).
    pub corners: [[f64; 3]; 4],
    /// Which edges are invisible (bit 0 the first edge, up to bit 3); `0`
    /// when the face states none.
    pub invisible_edges: i16,
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

/// A line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    /// The z of the insertion point (`0` when none is stated).
    pub elevation: f64,
    /// The insertion point, in the text's own plane.
    pub insertion: [f64; 2],
    /// The second alignment point, when the text states one (the insertion
    /// point stands in for it otherwise).
    pub alignment: Option<[f64; 2]>,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The thickness.
    pub thickness: f64,
    /// The oblique angle, in radians (`0` when none is stated).
    pub oblique: f64,
    /// The rotation, in radians (`0` when none is stated).
    pub rotation: f64,
    /// The text height.
    pub height: f64,
    /// The width factor (`1` when none is stated).
    pub width_factor: f64,
    /// The text.
    pub value: TextValue,
    /// The generation flags (`0` when none are stated).
    pub generation: i16,
    /// The horizontal alignment: left `0`, centre `1`, right `2`, aligned `3`, middle `4`, fit `5`.
    pub horizontal: i16,
    /// The vertical alignment: baseline `0`, bottom `1`, middle `2`, top `3`.
    pub vertical: i16,
    /// The handle of the text style.
    pub style: u64,
}

/// One piece of a hatch boundary.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum HatchEdge {
    /// A line segment.
    Line {
        /// The first end.
        start: [f64; 2],
        /// The second end.
        end: [f64; 2],
    },
    /// A circular arc.
    Arc {
        /// The centre.
        center: [f64; 2],
        /// The radius.
        radius: f64,
        /// The start angle, in radians.
        start_angle: f64,
        /// The end angle, in radians.
        end_angle: f64,
        /// Whether the arc runs counter-clockwise.
        counter_clockwise: bool,
    },
    /// An elliptical arc.
    Ellipse {
        /// The centre.
        center: [f64; 2],
        /// The end of the major axis, relative to the centre.
        major_axis: [f64; 2],
        /// The ratio of the minor to the major axis.
        ratio: f64,
        /// The start angle, in radians.
        start_angle: f64,
        /// The end angle, in radians.
        end_angle: f64,
        /// Whether the arc runs counter-clockwise.
        counter_clockwise: bool,
    },
    /// A spline.
    Spline {
        /// The degree.
        degree: u32,
        /// Whether the spline has weights.
        rational: bool,
        /// Whether the spline is periodic.
        periodic: bool,
        /// The knots.
        knots: Vec<f64>,
        /// The control points with their weights (`1` when the spline has none).
        control_points: Vec<([f64; 2], f64)>,
        /// The fit points (R2010 and later).
        fit_points: Vec<[f64; 2]>,
        /// The tangents at the start and the end (R2010 and later).
        tangents: Option<([f64; 2], [f64; 2])>,
    },
}

/// A boundary of a hatch: a polyline, or a chain of edges.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPath {
    /// The path flags: external `1`, polyline `2`, derived `4`, textbox `8`, outermost `16`.
    pub flags: i32,
    /// The pieces of an edge path (empty for a polyline path).
    pub edges: Vec<HatchEdge>,
    /// The points of a polyline path with the bulge of the segment that leaves each (`0` when none).
    pub polyline: Vec<([f64; 2], f64)>,
    /// Whether a polyline path is closed.
    pub closed: bool,
    /// The objects the boundary was made from (an associative hatch follows them).
    pub boundary_objects: Vec<u64>,
}

/// One line of a hatch pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct HatchPatternLine {
    /// The angle, in radians.
    pub angle: f64,
    /// A point the line passes through.
    pub base_point: [f64; 2],
    /// The offset to the next parallel line.
    pub offset: [f64; 2],
    /// The lengths of the dashes and the gaps (a gap is negative).
    pub dashes: Vec<f64>,
}

/// A hatch: an area filled with a pattern or a colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Hatch {
    /// The z of the hatch plane.
    pub elevation: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The name of the pattern.
    pub name: TextValue,
    /// Whether the area is filled solid.
    pub solid_fill: bool,
    /// Whether the hatch follows the objects of its boundary.
    pub associative: bool,
    /// Whether the fill is a gradient (R2004 and later).
    pub gradient: bool,
    /// The boundaries.
    pub paths: Vec<HatchPath>,
    /// The style: odd parity `0`, outermost `1`, whole area `2`.
    pub style: i16,
    /// The pattern type: user defined `0`, predefined `1`, custom `2`.
    pub pattern_type: i16,
    /// The pattern angle, in radians (a pattern fill).
    pub angle: f64,
    /// The pattern scale or spacing (a pattern fill).
    pub scale: f64,
    /// Whether the pattern is crossed with itself (a pattern fill).
    pub double: bool,
    /// The lines of the pattern (a pattern fill).
    pub pattern_lines: Vec<HatchPatternLine>,
    /// The seed points.
    pub seed_points: Vec<[f64; 2]>,
}

/// A ray (a half line) or an infinite line: a point and the direction it runs in.
#[derive(Debug, Clone, PartialEq)]
pub struct InfiniteLine {
    /// The point the line passes through (the start of a ray).
    pub point: [f64; 3],
    /// The direction.
    pub vector: [f64; 3],
}

/// A geometric tolerance frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Tolerance {
    /// The insertion point.
    pub insertion: [f64; 3],
    /// The direction the text runs along.
    pub x_direction: [f64; 3],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The text of the frame, with its formatting codes as stored.
    pub text: TextValue,
    /// The handle of the dimension style.
    pub style: Option<u64>,
}

/// A 2D polyline: the settings of the whole line. Its vertices are objects of
/// their own that it owns; `vertices` names them.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline2d {
    /// The flags: closed `1`, curve fit `2`, spline fit `4`, 3D polyline `8`, 3D mesh `16`, mesh closed `32`, polyface `64`, continuous linetype `128`.
    pub flags: i16,
    /// The curve type: none `0`, quadratic B-spline `5`, cubic B-spline `6`, Bezier `8`.
    pub curve_type: i16,
    /// The width a vertex starts with when it states none.
    pub start_width: f64,
    /// The width a vertex ends with when it states none.
    pub end_width: f64,
    /// The thickness.
    pub thickness: f64,
    /// The elevation: the z of every vertex.
    pub elevation: f64,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The vertices the polyline owns (R2000 names the first and the last only).
    pub vertices: Vec<u64>,
    /// The sequence end that closes the vertices.
    pub sequence_end: Option<u64>,
}

/// A 3D polyline.
#[derive(Debug, Clone, PartialEq)]
pub struct Polyline3d {
    /// The curve flags: splined `1` (quadratic) or `2` (cubic).
    pub spline_flags: u8,
    /// The closing flags: closed `1`.
    pub closed_flags: u8,
    /// The vertices the polyline owns (R2000 names the first and the last only).
    pub vertices: Vec<u64>,
    /// The sequence end that closes the vertices.
    pub sequence_end: Option<u64>,
}

/// A vertex of a 2D polyline.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex2d {
    /// The vertex flags: extra vertex `1`, curve fit tangent `2`, spline vertex `8`, spline control point `16`, 3D polyline vertex `32`, 3D mesh vertex `64`.
    pub flags: u8,
    /// The point; its z is not stated (the polyline's elevation is).
    pub point: [f64; 3],
    /// The width at the start of the segment that leaves the vertex.
    pub start_width: f64,
    /// The width at its end.
    pub end_width: f64,
    /// The bulge of that segment.
    pub bulge: f64,
    /// The vertex identifier (R2010 and later).
    pub id: i32,
    /// The tangent direction.
    pub tangent: f64,
}

/// A vertex of a 3D polyline.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex3d {
    /// The vertex flags.
    pub flags: u8,
    /// The point.
    pub point: [f64; 3],
}

/// A spline: a curve given by control points and knots, or by points it passes through.
#[derive(Debug, Clone, PartialEq)]
pub struct Spline {
    /// The scenario the file states: `1` control points and knots, `2` fit points only (from R2013 the
    /// fit-point flag of `flags` is the reliable sign, see `fit_data`).
    pub scenario: u32,
    /// Whether the file states the spline by its fit points (and not by control points and knots).
    pub fit_data: bool,
    /// The spline flags (R2013 and later): fit-point method `1`, control frame shown `2`, closed `4`, knot parameter used `8`.
    pub flags: u32,
    /// How the knots are derived from fit points (R2013 and later): chord `0`, square root `1`, uniform `2`, custom `15`.
    pub knot_parameter: u32,
    /// The degree.
    pub degree: u32,
    /// The fit tolerance (fit-point splines).
    pub fit_tolerance: Option<f64>,
    /// The tangent at the start (fit-point splines).
    pub start_tangent: Option<[f64; 3]>,
    /// The tangent at the end (fit-point splines).
    pub end_tangent: Option<[f64; 3]>,
    /// Whether the weights are not all one (control-point splines).
    pub rational: bool,
    /// Whether the curve is closed (control-point splines).
    pub closed: bool,
    /// Whether the curve is periodic (control-point splines).
    pub periodic: bool,
    /// The knot tolerance (control-point splines).
    pub knot_tolerance: f64,
    /// The control point tolerance (control-point splines).
    pub control_tolerance: f64,
    /// The knots.
    pub knots: Vec<f64>,
    /// The control points.
    pub control_points: Vec<[f64; 3]>,
    /// The weight of each control point, when the file states weights.
    pub weights: Vec<f64>,
    /// The fit points.
    pub fit_points: Vec<[f64; 3]>,
}

/// What a dimension measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DimensionKind {
    /// The distance of a point from the origin along one axis.
    Ordinate,
    /// A distance along a rotated line.
    Linear,
    /// A distance along the line between the two points.
    Aligned,
    /// An angle given by three points.
    Angular3Point,
    /// An angle between two lines.
    Angular2Line,
    /// A radius.
    Radius,
    /// A diameter.
    Diameter,
}

/// A dimension: the data every kind has and the points of its own kind. The
/// points are named as the format numbers them (the DXF group numbers).
#[derive(Debug, Clone, PartialEq)]
pub struct Dimension {
    /// What it measures.
    pub kind: DimensionKind,
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The middle of the dimension text.
    pub text_midpoint: [f64; 2],
    /// The z of the text middle and the 12-point.
    pub elevation: f64,
    /// Bit 0: the text is not at its default place (the opposite of the DXF 70 bit 128).
    pub flags: u8,
    /// The text the user typed instead of the measurement: empty for the measurement, a single space to suppress it.
    pub user_text: TextValue,
    /// The rotation of the text.
    pub text_rotation: f64,
    /// The horizontal direction of the dimension.
    pub horizontal_direction: f64,
    /// The scale of the insertion of the anonymous block that draws the dimension.
    pub insert_scale: [f64; 3],
    /// The rotation of that insertion.
    pub insert_rotation: f64,
    /// The attachment point of the text (R2000 and later).
    pub attachment_point: i16,
    /// The line spacing style (R2000 and later).
    pub line_spacing_style: i16,
    /// The line spacing factor (R2000 and later).
    pub line_spacing_factor: f64,
    /// The measurement the file states (R2000 and later).
    pub measurement: f64,
    /// Whether the first and the second arrow are flipped (R2007 and later).
    pub flip_arrows: [bool; 2],
    /// The 12-point: where the text was dragged to.
    pub point12: [f64; 2],
    /// The 10-point: where the dimension line is defined.
    pub definition_point: [f64; 3],
    /// The 13-point: the first extension line origin or the first point of an angle.
    pub point13: Option<[f64; 3]>,
    /// The 14-point: the second extension line origin or the second point of an angle.
    pub point14: Option<[f64; 3]>,
    /// The 15-point: the centre of an angle, or the point on the arc of a radius or diameter.
    pub point15: Option<[f64; 3]>,
    /// The 16-point: the place on the arc a two-line angle is dimensioned at.
    pub point16: Option<[f64; 2]>,
    /// The rotation of the extension lines (linear and aligned).
    pub extension_rotation: Option<f64>,
    /// The rotation of the dimension line (linear).
    pub rotation: Option<f64>,
    /// The leader length (radius and diameter).
    pub leader_length: Option<f64>,
    /// Which axis an ordinate dimension measures: bit 0 set for x.
    pub ordinate_flags: Option<u8>,
    /// The handle of the dimension style.
    pub style: Option<u64>,
    /// The handle of the anonymous block that draws the dimension, when it has one.
    pub block: Option<u64>,
}

/// A multiline text. Its value keeps the formatting codes the file stores.
#[derive(Debug, Clone, PartialEq)]
pub struct Mtext {
    /// The insertion point.
    pub insertion: [f64; 3],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The direction the text runs along (the x axis of its frame).
    pub x_axis: [f64; 3],
    /// The width of the reference rectangle (`0` when none).
    pub rect_width: f64,
    /// The height of the reference rectangle (R2007 and later).
    pub rect_height: f64,
    /// The height of the characters.
    pub text_height: f64,
    /// How the text hangs on the insertion point: top left `1`, top centre `2`,
    /// top right `3`, middle left `4` ... bottom right `9`.
    pub attachment: i16,
    /// The drawing direction: left to right `1`, top to bottom `3`, by style `5`.
    pub drawing_direction: i16,
    /// The height of the text as laid out.
    pub extents_height: f64,
    /// The width of the text as laid out.
    pub extents_width: f64,
    /// The text.
    pub value: TextValue,
    /// The line spacing style: at least `1`, exact `2`.
    pub line_spacing_style: i16,
    /// The line spacing factor.
    pub line_spacing_factor: f64,
    /// The handle of the text style.
    pub style: u64,
}

/// An attribute (a text that belongs to a block insertion) or an attribute
/// definition (the template of one, inside a block definition): a text with a tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// The text. For a definition its value is the default value.
    pub text: Text,
    /// The tag that names the attribute.
    pub tag: TextValue,
    /// The flags: bit 0 invisible, bit 1 constant, bit 2 verify, bit 3 preset.
    pub flags: u8,
    /// Whether the position is locked (R2010 and later).
    pub position_locked: bool,
    /// The prompt, which only a definition has.
    pub prompt: Option<TextValue>,
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
    /// The handle of the block header the insertion inserts.
    pub block: u64,
    /// The attributes that follow (R2000 names the first and the last; later
    /// versions name them all).
    pub attributes: Vec<u64>,
    /// The handle of the sequence end that closes the attributes.
    pub sequence_end: Option<u64>,
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

/// The objects an entity points to, from its handle stream. Every handle is
/// absolute; a reference that names nothing (a null handle) is `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct References {
    /// The owner (the block the entity lives in), stated when the entity is not
    /// simply in model or paper space.
    pub owner: Option<u64>,
    /// The persistent reactors.
    pub reactors: Vec<u64>,
    /// The extension dictionary.
    pub xdictionary: Option<u64>,
    /// Whether the entity states the entities before and after it in its block
    /// (R2000 and later where the file keeps links); otherwise they are the
    /// handles next to its own.
    pub links_stored: bool,
    /// The entity before this one in its block, when the links are stored.
    pub previous: Option<u64>,
    /// The entity after this one in its block, when the links are stored.
    pub next: Option<u64>,
    /// The layer.
    pub layer: u64,
    /// The linetype, when the entity has one of its own.
    pub linetype: Option<u64>,
    /// The material, when the entity has one of its own.
    pub material: Option<u64>,
    /// The plot style, when the entity has one of its own.
    pub plotstyle: Option<u64>,
}

/// What the main stream says about the handles that follow.
#[derive(Debug, Clone, Copy)]
struct HandleFlags {
    xdictionary_stored: bool,
    links_stored: bool,
    material_stored: bool,
    full_style: bool,
    face_style: bool,
    edge_style: bool,
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
    /// A line of text.
    Text(Text),
    /// A multiline text.
    Mtext(Mtext),
    /// A dimension.
    Dimension(Dimension),
    /// A spline.
    Spline(Spline),
    /// A hatch.
    Hatch(Hatch),
    /// A trace: a filled area like a solid.
    Trace(Solid),
    /// A ray.
    Ray(InfiniteLine),
    /// An infinite line.
    Xline(InfiniteLine),
    /// A geometric tolerance frame.
    Tolerance(Tolerance),
    /// A 2D polyline.
    Polyline2d(Polyline2d),
    /// A 3D polyline.
    Polyline3d(Polyline3d),
    /// A vertex of a 2D polyline.
    Vertex2d(Vertex2d),
    /// A vertex of a 3D polyline.
    Vertex3d(Vertex3d),
    /// An attribute of a block insertion.
    Attrib(Attribute),
    /// An attribute definition.
    Attdef(Attribute),
    /// An ellipse.
    Ellipse(Ellipse),
    /// A flat filled area.
    Solid(Solid),
    /// A 3D face.
    Face3d(Face3d),
    /// A viewport.
    Viewport(Viewport),
    /// A raster image.
    Image(Image),
    /// A leader.
    Leader(Leader),
    /// A multiline.
    Mline(Mline),
}

/// An entity: its common data and its geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// What every entity states.
    pub common: CommonEntity,
    /// The entity's own fields.
    pub geometry: Geometry,
    /// The objects the entity points to.
    pub references: References,
}

/// Reads the common entity data. The reader is positioned just after the
/// object type.
fn read_common(
    r: &mut BitReader<'_>,
    version: Version,
) -> Result<(CommonEntity, HandleFlags), BitError> {
    let before_r2010 = version < Version::R2010;
    if before_r2010 {
        // The size of the object data in bits, not counting the handles.
        r.raw_long()?;
    }
    let handle = r.handle()?.value;

    let extended_data = read_extended_data(r)?;

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
    // Before R2004 an extension dictionary reference is always stored; from
    // R2004 a bit says it is not.
    let xdictionary_stored = version < Version::R2004 || !r.bit()?;
    if version >= Version::R2013 {
        r.bit()?; // binary data is kept in the data store
    }
    // Before R2004 a bit says the previous and next entities are not stored; from
    // R2004 they never are.
    let links_stored = version < Version::R2004 && !r.bit()?;

    let (mut rgb, mut transparency) = (None, None);
    let color = if version >= Version::R2004 {
        let flags = r.bit_short()? as u16;
        if flags & 0x8000 != 0 {
            rgb = Some(r.bit_long()? as u32 & 0x00FF_FFFF);
        }
        if flags & 0x2000 != 0 {
            transparency = Some(r.bit_long()? as u32);
        }
        flags
    } else {
        r.bit_short()? as u16
    };
    let linetype_scale = r.bit_double()?;
    let linetype_flags = r.bits(2)? as u8;
    let plotstyle_flags = r.bits(2)? as u8;
    let mut material_stored = false;
    if version >= Version::R2007 {
        material_stored = r.bits(2)? == 3; // material flags: a handle is stored
        r.raw_char()?; // shadow flags
    }
    let (mut full_style, mut face_style, mut edge_style) = (false, false, false);
    if version >= Version::R2010 {
        full_style = r.bit()?;
        face_style = r.bit()?;
        edge_style = r.bit()?;
    }
    let invisible = r.bit_short()?;
    let lineweight = r.raw_char()?;
    Ok((
        CommonEntity {
            handle,
            entity_mode,
            reactor_count,
            color,
            rgb,
            transparency,
            linetype_scale,
            linetype_flags,
            plotstyle_flags,
            invisible,
            lineweight,
            extended_data,
        },
        HandleFlags {
            xdictionary_stored,
            links_stored,
            material_stored,
            full_style,
            face_style,
            edge_style,
        },
    ))
}

/// Reads the handle stream that every entity has: the references of the
/// common data, in the order the format gives them. The reader is positioned
/// at the start of the handle stream.
fn read_references(
    h: &mut BitReader<'_>,
    common: &CommonEntity,
    flags: HandleFlags,
) -> Result<References, ObjectError> {
    let holder = common.handle;
    let next = |h: &mut BitReader<'_>| -> Result<Option<u64>, ObjectError> {
        Ok(h.handle()?.resolve(holder).filter(|&v| v != 0))
    };
    let mut refs = References::default();
    if common.entity_mode == 0 {
        refs.owner = next(h)?;
    }
    let reactors = usize::try_from(common.reactor_count)
        .ok()
        .filter(|&n| n <= h.remaining() / 8)
        .ok_or(ObjectError::Unexpected {
            what: "reactor count",
        })?;
    for _ in 0..reactors {
        if let Some(v) = next(h)? {
            refs.reactors.push(v);
        }
    }
    if flags.xdictionary_stored {
        refs.xdictionary = next(h)?;
    }
    refs.links_stored = flags.links_stored;
    if flags.links_stored {
        refs.previous = next(h)?;
        refs.next = next(h)?;
    }
    refs.layer = next(h)?.ok_or(ObjectError::Unexpected {
        what: "layer handle",
    })?;
    if common.linetype_flags == 3 {
        refs.linetype = next(h)?;
    }
    if flags.material_stored {
        refs.material = next(h)?;
    }
    if common.plotstyle_flags == 3 {
        refs.plotstyle = next(h)?;
    }
    for stored in [flags.full_style, flags.face_style, flags.edge_style] {
        if stored {
            next(h)?; // a visual style
        }
    }
    Ok(refs)
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

fn read_solid(r: &mut BitReader<'_>) -> Result<Solid, BitError> {
    let thickness = r.bit_thickness()?;
    let elevation = r.bit_double()?;
    let mut corners = [[0.0; 2]; 4];
    for corner in &mut corners {
        *corner = [r.raw_double()?, r.raw_double()?];
    }
    Ok(Solid {
        thickness,
        elevation,
        corners,
        extrusion: r.bit_extrusion()?,
    })
}

fn read_face3d(r: &mut BitReader<'_>) -> Result<Face3d, BitError> {
    let no_flags = r.bit()?;
    let z_is_zero = r.bit()?;
    let first = [
        r.raw_double()?,
        r.raw_double()?,
        if z_is_zero { 0.0 } else { r.raw_double()? },
    ];
    // Each later corner is three difference-coded doubles against the one before.
    let mut corners = [first; 4];
    for i in 1..4 {
        let previous = corners[i - 1];
        corners[i] = [
            r.bit_double_with_default(previous[0])?,
            r.bit_double_with_default(previous[1])?,
            r.bit_double_with_default(previous[2])?,
        ];
    }
    let invisible_edges = if no_flags { 0 } else { r.bit_short()? };
    Ok(Face3d {
        corners,
        invisible_edges,
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

/// Skips a colour as R2004 and later write it: a short that is always zero, the
/// RGB value, and a flag byte that says whether a colour name and a book name follow.
fn skip_color(r: &mut BitReader<'_>, strings: &mut Option<BitReader<'_>>) -> Result<(), BitError> {
    r.bit_short()?;
    r.bit_long()?;
    let flags = r.raw_char()?;
    if flags & 1 != 0 {
        text_value(r, strings)?;
    }
    if flags & 2 != 0 {
        text_value(r, strings)?;
    }
    Ok(())
}

/// Reads a hatch. The handles of the boundary objects are in the handle
/// stream; the counts of them are returned for it, path by path.
fn read_hatch(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(Hatch, Vec<usize>), BitError> {
    let mut gradient = false;
    if version >= Version::R2004 {
        gradient = r.bit_long()? != 0;
        r.bit_long()?; // reserved
        r.bit_double()?; // gradient angle
        r.bit_double()?; // gradient shift
        r.bit_long()?; // single colour gradient
        r.bit_double()?; // gradient tint
        let colors = bounded_count(r, "gradient colour count")?;
        for _ in 0..colors {
            r.bit_double()?;
            r.bit_short()?;
            r.bit_long()?;
            r.raw_char()?;
        }
        text_value(r, strings)?; // the gradient name
    }
    let elevation = r.bit_double()?;
    let extrusion = r.point_3bd()?;
    let name = text_value(r, strings)?;
    let solid_fill = r.bit()?;
    let associative = r.bit()?;
    let path_count = bounded_count(r, "hatch path count")?;
    let mut paths = Vec::with_capacity(path_count);
    let mut boundary_counts = Vec::with_capacity(path_count);
    let mut any_derived = false;
    for _ in 0..path_count {
        let flags = r.bit_long()?;
        any_derived |= flags & 4 != 0;
        let mut path = HatchPath {
            flags,
            edges: Vec::new(),
            polyline: Vec::new(),
            closed: false,
            boundary_objects: Vec::new(),
        };
        if flags & 2 == 0 {
            let segments = bounded_count(r, "hatch edge count")?;
            for _ in 0..segments {
                let kind = r.raw_char()?;
                path.edges.push(match kind {
                    1 => HatchEdge::Line {
                        start: [r.raw_double()?, r.raw_double()?],
                        end: [r.raw_double()?, r.raw_double()?],
                    },
                    2 => HatchEdge::Arc {
                        center: [r.raw_double()?, r.raw_double()?],
                        radius: r.bit_double()?,
                        start_angle: r.bit_double()?,
                        end_angle: r.bit_double()?,
                        counter_clockwise: r.bit()?,
                    },
                    3 => HatchEdge::Ellipse {
                        center: [r.raw_double()?, r.raw_double()?],
                        major_axis: [r.raw_double()?, r.raw_double()?],
                        ratio: r.bit_double()?,
                        start_angle: r.bit_double()?,
                        end_angle: r.bit_double()?,
                        counter_clockwise: r.bit()?,
                    },
                    4 => {
                        let degree =
                            u32::try_from(r.bit_long()?).map_err(|_| BitError::TooLong {
                                form: "hatch spline degree",
                            })?;
                        let rational = r.bit()?;
                        let periodic = r.bit()?;
                        let knot_count = bounded_count(r, "hatch spline knot count")?;
                        let control_count = bounded_count(r, "hatch spline control point count")?;
                        let mut knots = Vec::with_capacity(knot_count);
                        for _ in 0..knot_count {
                            knots.push(r.bit_double()?);
                        }
                        let mut control_points = Vec::with_capacity(control_count);
                        for _ in 0..control_count {
                            let point = [r.raw_double()?, r.raw_double()?];
                            let weight = if rational { r.bit_double()? } else { 1.0 };
                            control_points.push((point, weight));
                        }
                        let mut fit_points = Vec::new();
                        let mut tangents = None;
                        if version >= Version::R2010 {
                            let fit_count = bounded_count(r, "hatch spline fit point count")?;
                            for _ in 0..fit_count {
                                fit_points.push([r.raw_double()?, r.raw_double()?]);
                            }
                            tangents = Some((
                                [r.raw_double()?, r.raw_double()?],
                                [r.raw_double()?, r.raw_double()?],
                            ));
                        }
                        HatchEdge::Spline {
                            degree,
                            rational,
                            periodic,
                            knots,
                            control_points,
                            fit_points,
                            tangents,
                        }
                    }
                    _ => {
                        return Err(BitError::Unsupported {
                            form: "a hatch edge of an unknown kind",
                        })
                    }
                });
            }
        } else {
            let bulges = r.bit()?;
            path.closed = r.bit()?;
            let count = bounded_count(r, "hatch polyline point count")?;
            for _ in 0..count {
                let point = [r.raw_double()?, r.raw_double()?];
                let bulge = if bulges { r.bit_double()? } else { 0.0 };
                path.polyline.push((point, bulge));
            }
        }
        boundary_counts.push(bounded_count(r, "hatch boundary object count")?);
        paths.push(path);
    }
    let style = r.bit_short()?;
    let pattern_type = r.bit_short()?;
    let (mut angle, mut scale, mut double) = (0.0, 0.0, false);
    let mut pattern_lines = Vec::new();
    if !solid_fill {
        angle = r.bit_double()?;
        scale = r.bit_double()?;
        double = r.bit()?;
        let lines = usize::try_from(r.bit_short()?).map_err(|_| BitError::TooLong {
            form: "hatch pattern line count",
        })?;
        for _ in 0..lines {
            let line_angle = r.bit_double()?;
            let base_point = [r.bit_double()?, r.bit_double()?];
            let offset = [r.bit_double()?, r.bit_double()?];
            let dash_count = usize::try_from(r.bit_short()?).map_err(|_| BitError::TooLong {
                form: "hatch dash count",
            })?;
            let mut dashes = Vec::with_capacity(dash_count.min(r.remaining()));
            for _ in 0..dash_count {
                dashes.push(r.bit_double()?);
            }
            pattern_lines.push(HatchPatternLine {
                angle: line_angle,
                base_point,
                offset,
                dashes,
            });
        }
    }
    if any_derived {
        r.bit_double()?; // the pixel size
    }
    let seed_count = bounded_count(r, "hatch seed point count")?;
    let mut seed_points = Vec::with_capacity(seed_count);
    for _ in 0..seed_count {
        seed_points.push([r.raw_double()?, r.raw_double()?]);
    }
    Ok((
        Hatch {
            elevation,
            extrusion,
            name,
            solid_fill,
            associative,
            gradient,
            paths,
            style,
            pattern_type,
            angle,
            scale,
            double,
            pattern_lines,
            seed_points,
        },
        boundary_counts,
    ))
}

/// Reads a geometric tolerance. R13 and R14 state three more values first.
fn read_tolerance(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<Tolerance, BitError> {
    if version < Version::R2000 {
        r.bit_short()?;
        r.bit_double()?;
        r.bit_double()?;
    }
    Ok(Tolerance {
        insertion: r.point_3bd()?,
        x_direction: r.point_3bd()?,
        extrusion: r.point_3bd()?,
        text: text_value(r, strings)?,
        style: None,
    })
}

/// Reads a 2D polyline; the second value is the count of vertices it owns (R2004 and later).
fn read_polyline_2d(
    r: &mut BitReader<'_>,
    version: Version,
) -> Result<(Polyline2d, usize), BitError> {
    let flags = r.bit_short()?;
    let curve_type = r.bit_short()?;
    let start_width = r.bit_double()?;
    let end_width = r.bit_double()?;
    let thickness = r.bit_thickness()?;
    let elevation = r.bit_double()?;
    let extrusion = r.bit_extrusion()?;
    let owned = if version >= Version::R2004 {
        bounded_count(r, "vertex count")?
    } else {
        0
    };
    Ok((
        Polyline2d {
            flags,
            curve_type,
            start_width,
            end_width,
            thickness,
            elevation,
            extrusion,
            vertices: Vec::new(),
            sequence_end: None,
        },
        owned,
    ))
}

/// Reads a 3D polyline; the second value is the count of vertices it owns (R2004 and later).
fn read_polyline_3d(
    r: &mut BitReader<'_>,
    version: Version,
) -> Result<(Polyline3d, usize), BitError> {
    let spline_flags = r.raw_char()?;
    let closed_flags = r.raw_char()?;
    let owned = if version >= Version::R2004 {
        bounded_count(r, "vertex count")?
    } else {
        0
    };
    Ok((
        Polyline3d {
            spline_flags,
            closed_flags,
            vertices: Vec::new(),
            sequence_end: None,
        },
        owned,
    ))
}

/// Reads a vertex of a 2D polyline. A negative start width states both widths
/// (its absolute value) and the end width is then not stored.
fn read_vertex_2d(r: &mut BitReader<'_>, version: Version) -> Result<Vertex2d, BitError> {
    let flags = r.raw_char()?;
    let point = r.point_3bd()?;
    let mut start_width = r.bit_double()?;
    let end_width = if start_width < 0.0 {
        start_width = -start_width;
        start_width
    } else {
        r.bit_double()?
    };
    let bulge = r.bit_double()?;
    let id = if version >= Version::R2010 {
        r.bit_long()?
    } else {
        0
    };
    let tangent = r.bit_double()?;
    Ok(Vertex2d {
        flags,
        point,
        start_width,
        end_width,
        bulge,
        id,
        tangent,
    })
}

/// Reads a spline.
fn read_spline(r: &mut BitReader<'_>, version: Version) -> Result<Spline, BitError> {
    let scenario = u32::try_from(r.bit_long()?).map_err(|_| BitError::TooLong {
        form: "spline scenario",
    })?;
    if !(1..=2).contains(&scenario) {
        return Err(BitError::Unsupported {
            form: "a spline scenario other than 1 and 2",
        });
    }
    let (mut flags, mut knot_parameter) = (0, 0);
    if version >= Version::R2013 {
        flags = r.bit_long()? as u32;
        knot_parameter = r.bit_long()? as u32;
    }
    let degree = u32::try_from(r.bit_long()?).map_err(|_| BitError::TooLong {
        form: "spline degree",
    })?;
    // Up to R2010 the scenario says which data the file states; from R2013 the
    // fit-point flag does (the scenario is not trustworthy there: real files
    // state a fit-point spline with scenario 1).
    let fit_data = if version >= Version::R2013 {
        flags & 1 != 0
    } else {
        scenario == 2
    };
    let mut spline = Spline {
        scenario,
        fit_data,
        flags,
        knot_parameter,
        degree,
        fit_tolerance: None,
        start_tangent: None,
        end_tangent: None,
        rational: false,
        closed: false,
        periodic: false,
        knot_tolerance: 0.0,
        control_tolerance: 0.0,
        knots: Vec::new(),
        control_points: Vec::new(),
        weights: Vec::new(),
        fit_points: Vec::new(),
    };
    let (mut knot_count, mut control_count, mut fit_count, mut weighted) = (0, 0, 0, false);
    if fit_data {
        // From R2013 the closed flag of a fit-point spline is in the spline flags.
        spline.closed = version >= Version::R2013 && flags & 4 != 0;
        spline.fit_tolerance = Some(r.bit_double()?);
        spline.start_tangent = Some(r.point_3bd()?);
        spline.end_tangent = Some(r.point_3bd()?);
        fit_count = bounded_count(r, "spline fit point count")?;
    } else {
        spline.rational = r.bit()?;
        spline.closed = r.bit()?;
        spline.periodic = r.bit()?;
        spline.knot_tolerance = r.bit_double()?;
        spline.control_tolerance = r.bit_double()?;
        knot_count = bounded_count(r, "spline knot count")?;
        control_count = bounded_count(r, "spline control point count")?;
        weighted = r.bit()?;
    }
    for _ in 0..knot_count {
        spline.knots.push(r.bit_double()?);
    }
    for _ in 0..control_count {
        spline.control_points.push(r.point_3bd()?);
        if weighted {
            spline.weights.push(r.bit_double()?);
        }
    }
    for _ in 0..fit_count {
        spline.fit_points.push(r.point_3bd()?);
    }
    Ok(spline)
}

/// Reads a dimension: the data every kind has, then the points of the kind.
fn read_dimension(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
    kind: DimensionKind,
) -> Result<Dimension, BitError> {
    if version >= Version::R2010 {
        r.raw_char()?; // the version of the dimension's own form
    }
    let extrusion = r.point_3bd()?;
    let text_midpoint = [r.raw_double()?, r.raw_double()?];
    let elevation = r.bit_double()?;
    let flags = r.raw_char()?;
    let user_text = text_value(r, strings)?;
    let text_rotation = r.bit_double()?;
    let horizontal_direction = r.bit_double()?;
    let insert_scale = r.point_3bd()?;
    let insert_rotation = r.bit_double()?;
    let (mut attachment_point, mut line_spacing_style, mut line_spacing_factor, mut measurement) =
        (0, 0, 0.0, 0.0);
    if version >= Version::R2000 {
        attachment_point = r.bit_short()?;
        line_spacing_style = r.bit_short()?;
        line_spacing_factor = r.bit_double()?;
        measurement = r.bit_double()?;
    }
    let mut flip_arrows = [false; 2];
    if version >= Version::R2007 {
        r.bit()?; // undocumented
        flip_arrows = [r.bit()?, r.bit()?];
    }
    let point12 = [r.raw_double()?, r.raw_double()?];
    let mut d = Dimension {
        kind,
        extrusion,
        text_midpoint,
        elevation,
        flags,
        user_text,
        text_rotation,
        horizontal_direction,
        insert_scale,
        insert_rotation,
        attachment_point,
        line_spacing_style,
        line_spacing_factor,
        measurement,
        flip_arrows,
        point12,
        definition_point: [0.0; 3],
        point13: None,
        point14: None,
        point15: None,
        point16: None,
        extension_rotation: None,
        rotation: None,
        leader_length: None,
        ordinate_flags: None,
        style: None,
        block: None,
    };
    match kind {
        DimensionKind::Ordinate => {
            d.definition_point = r.point_3bd()?;
            d.point13 = Some(r.point_3bd()?);
            d.point14 = Some(r.point_3bd()?);
            d.ordinate_flags = Some(r.raw_char()?);
        }
        DimensionKind::Linear => {
            d.point13 = Some(r.point_3bd()?);
            d.point14 = Some(r.point_3bd()?);
            d.definition_point = r.point_3bd()?;
            d.extension_rotation = Some(r.bit_double()?);
            d.rotation = Some(r.bit_double()?);
        }
        DimensionKind::Aligned => {
            d.point13 = Some(r.point_3bd()?);
            d.point14 = Some(r.point_3bd()?);
            d.definition_point = r.point_3bd()?;
            d.extension_rotation = Some(r.bit_double()?);
        }
        DimensionKind::Angular3Point => {
            d.definition_point = r.point_3bd()?;
            d.point13 = Some(r.point_3bd()?);
            d.point14 = Some(r.point_3bd()?);
            d.point15 = Some(r.point_3bd()?);
        }
        DimensionKind::Angular2Line => {
            d.point16 = Some([r.raw_double()?, r.raw_double()?]);
            d.point13 = Some(r.point_3bd()?);
            d.point14 = Some(r.point_3bd()?);
            d.point15 = Some(r.point_3bd()?);
            d.definition_point = r.point_3bd()?;
        }
        DimensionKind::Radius => {
            d.definition_point = r.point_3bd()?;
            d.point15 = Some(r.point_3bd()?);
            d.leader_length = Some(r.bit_double()?);
        }
        DimensionKind::Diameter => {
            d.point15 = Some(r.point_3bd()?);
            d.definition_point = r.point_3bd()?;
            d.leader_length = Some(r.bit_double()?);
        }
    }
    Ok(d)
}

/// Reads a multiline text.
///
/// The second value says whether the handle stream names a registered
/// application after the style (R2018, when the text is not annotative).
fn read_mtext(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(Mtext, bool), BitError> {
    let insertion = r.point_3bd()?;
    let extrusion = r.point_3bd()?;
    let x_axis = r.point_3bd()?;
    let rect_width = r.bit_double()?;
    let rect_height = if version >= Version::R2007 {
        r.bit_double()?
    } else {
        0.0
    };
    let text_height = r.bit_double()?;
    let attachment = r.bit_short()?;
    let drawing_direction = r.bit_short()?;
    let extents_height = r.bit_double()?;
    let extents_width = r.bit_double()?;
    let value = text_value(r, strings)?;
    let line_spacing_style = r.bit_short()?;
    let line_spacing_factor = r.bit_double()?;
    r.bit()?; // a bit the format leaves unexplained
    if version >= Version::R2004 {
        let background = r.bit_long()?;
        let framed = version >= Version::R2018 && background & 0x10 != 0;
        if background & 1 != 0 || framed {
            // The background scale factor. The specification types it as a bit
            // long yet gives it the default 1.5, so it is read as a bit double;
            // no file of the corpus has a background to say which is right.
            r.bit_double()?;
            skip_color(r, strings)?;
            r.bit_long()?; // the background transparency
        }
    }
    let mut application = false;
    if version >= Version::R2018 && r.bit()? {
        // Not annotative: a version, a flag, the fields stated a second time
        // and the column data follow.
        r.bit_short()?;
        r.bit()?;
        application = true;
        r.bit_long()?; // attachment point
        r.point_3bd()?; // x axis direction
        r.point_3bd()?; // insertion point
        for _ in 0..4 {
            r.bit_double()?; // rectangle width and height, extents width and height
        }
        let column_type = r.bit_short()?;
        if column_type != 0 {
            let heights = bounded_count(r, "column height count")?;
            r.bit_double()?; // column width
            r.bit_double()?; // gutter
            let auto_height = r.bit()?;
            r.bit()?; // flow reversed
            if !auto_height && column_type == 2 {
                for _ in 0..heights {
                    r.bit_double()?;
                }
            }
        }
    }
    Ok((
        Mtext {
            insertion,
            extrusion,
            x_axis,
            rect_width,
            rect_height,
            text_height,
            attachment,
            drawing_direction,
            extents_height,
            extents_width,
            value,
            line_spacing_style,
            line_spacing_factor,
            style: 0,
        },
        application,
    ))
}

/// A text value from where the version keeps it: the string stream from R2010,
/// the main stream before.
fn text_value(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
) -> Result<TextValue, BitError> {
    match strings {
        Some(s) => s.text(true),
        None => r.text(false),
    }
}

/// Reads an attribute or an attribute definition: a text, then the tag, the
/// flags and, for a definition, the prompt.
///
/// From R2010 a version byte follows the text (R2018 adds the attribute type
/// next), and a definition has a second version byte after the lock flag, as
/// the specification lists. Real files agree: the main stream ends exactly
/// where it should with them, and not without.
fn read_attribute(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
    definition: bool,
) -> Result<Attribute, BitError> {
    let text = read_text(r, strings)?;
    if version >= Version::R2010 {
        r.raw_char()?;
    }
    let mut attribute_type = 1;
    if version >= Version::R2018 {
        attribute_type = r.raw_char()?;
    }
    let tag = text_value(r, strings)?;
    r.bit_short()?; // the field length, unused
    let flags = r.raw_char()?;
    let position_locked = if version >= Version::R2010 {
        r.bit()?
    } else {
        false
    };
    if definition && version >= Version::R2010 {
        r.raw_char()?;
    }
    let prompt = if definition {
        Some(text_value(r, strings)?)
    } else {
        None
    };
    if attribute_type != 1 {
        // Anything but a single line of text carries an embedded multiline text.
        return Err(BitError::Unsupported {
            form: "a multiline attribute",
        });
    }
    Ok(Attribute {
        text,
        tag,
        flags,
        position_locked,
        prompt,
    })
}

/// Reads a line of text. Its text value is in the string stream from R2007
/// and in the main stream before; `strings` is that stream when there is one.
fn read_text(r: &mut BitReader<'_>, strings: &mut Option<BitReader<'_>>) -> Result<Text, BitError> {
    let flags = r.raw_char()?;
    let elevation = if flags & 1 == 0 { r.raw_double()? } else { 0.0 };
    let insertion = [r.raw_double()?, r.raw_double()?];
    let alignment = if flags & 2 == 0 {
        Some([
            r.bit_double_with_default(insertion[0])?,
            r.bit_double_with_default(insertion[1])?,
        ])
    } else {
        None
    };
    let extrusion = r.bit_extrusion()?;
    let thickness = r.bit_thickness()?;
    let oblique = if flags & 4 == 0 { r.raw_double()? } else { 0.0 };
    let rotation = if flags & 8 == 0 { r.raw_double()? } else { 0.0 };
    let height = r.raw_double()?;
    let width_factor = if flags & 0x10 == 0 {
        r.raw_double()?
    } else {
        1.0
    };
    let value = text_value(r, strings)?;
    let generation = if flags & 0x20 == 0 { r.bit_short()? } else { 0 };
    let horizontal = if flags & 0x40 == 0 { r.bit_short()? } else { 0 };
    let vertical = if flags & 0x80 == 0 { r.bit_short()? } else { 0 };
    Ok(Text {
        elevation,
        insertion,
        alignment,
        extrusion,
        thickness,
        oblique,
        rotation,
        height,
        width_factor,
        value,
        generation,
        horizontal,
        vertical,
        style: 0,
    })
}

/// Reads the main-stream fields of a block insertion; the second value is the
/// count of attributes it owns (R2004 and later), which the handle stream lists.
fn read_insert(r: &mut BitReader<'_>, version: Version) -> Result<(Insert, usize), BitError> {
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
    let rotation = r.bit_double()?;
    let extrusion = r.point_3bd()?;
    let has_attributes = r.bit()?;
    let owned = if has_attributes && version >= Version::R2004 {
        bounded_count(r, "attribute count")?
    } else {
        0
    };
    Ok((
        Insert {
            position,
            scale,
            rotation,
            extrusion,
            has_attributes,
            block: 0,
            attributes: Vec::new(),
            sequence_end: None,
        },
        owned,
    ))
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
    read_entity_with_classes(objects, entry, version, &[])
}

/// The entities before and after the entity at `entry` in its block.
///
/// R2000 keeps a block's entities as a chain: the block header names the first
/// and the last, and each entity names its neighbours. This reads the links
/// of an entity of any kind -- only the data every entity has and the handle
/// stream, never its geometry.
pub fn read_entity_links(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<EntityLinks, ObjectError> {
    let (_, mut r, layout) = open_object(objects, entry, version)?;
    let (common, flags) = read_common(&mut r, version)?;
    if common.handle != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "entity handle",
        });
    }
    let mut h = r.clone();
    h.seek(layout.handle_start);
    let references = read_references(&mut h, &common, flags)?;
    Ok(EntityLinks {
        stored: references.links_stored,
        previous: references.previous,
        next: references.next,
    })
}

/// The neighbours of an entity in its block, as [`read_entity_links`] reads them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLinks {
    /// Whether the file states the neighbours; when it does not, they are the
    /// handles next to the entity's own.
    pub stored: bool,
    /// The entity before this one.
    pub previous: Option<u64>,
    /// The entity after this one.
    pub next: Option<u64>,
}

/// Like [`read_entity`], and also reads the kinds that have no fixed type
/// code but are named by a class of the drawing (hatches): `classes` is the
/// class table of the file.
pub fn read_entity_with_classes(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
    classes: &[ClassDef],
) -> Result<Option<Entity>, ObjectError> {
    let (mut frame, mut r, layout) = open_object(objects, entry, version)?;
    // A hatch has the fixed type code 78 in R2000 and is an object of a class
    // (named by its DXF name) in later versions.
    if frame.type_code == 78
        || (frame.type_code >= 500
            && classes.iter().any(|c| {
                i32::from(c.number) == i32::from(frame.type_code) && c.dxf_name == "HATCH"
            }))
    {
        frame.type_code = TYPE_HATCH;
    }
    if frame.type_code >= 500
        && classes
            .iter()
            .any(|c| i32::from(c.number) == i32::from(frame.type_code) && c.dxf_name == "IMAGE")
    {
        frame.type_code = TYPE_IMAGE;
    }
    if !matches!(
        frame.type_code,
        TYPE_ARC
            | TYPE_CIRCLE
            | TYPE_LINE
            | TYPE_POINT
            | TYPE_LWPOLYLINE
            | TYPE_INSERT
            | TYPE_TEXT
            | TYPE_MTEXT
            | TYPE_HATCH
            | TYPE_TRACE
            | TYPE_RAY
            | TYPE_XLINE
            | TYPE_TOLERANCE
            | TYPE_SPLINE
            | TYPE_POLYLINE_2D
            | TYPE_POLYLINE_3D
            | TYPE_VERTEX_2D
            | TYPE_VERTEX_3D
            | TYPE_DIMENSION_ORDINATE
            ..=TYPE_DIMENSION_DIAMETER
                | TYPE_ATTRIB
                | TYPE_ATTDEF
                | TYPE_ELLIPSE
                | TYPE_SOLID
                | TYPE_3DFACE
                | TYPE_VIEWPORT
                | TYPE_IMAGE
                | TYPE_LEADER
                | TYPE_MLINE
    ) {
        return Ok(None);
    }
    let (common, flags) = read_common(&mut r, version)?;
    if common.handle != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "entity handle",
        });
    }
    let mut owned = 0;
    let mut boundary_counts: Vec<usize> = Vec::new();
    let mut registered_application = false;
    let mut frozen_layer_count = 0usize;
    // From R2007 the text values of an entity are in its string stream.
    let mut strings = layout.strings.map(|(from, _)| {
        let mut s = r.clone();
        s.seek(from);
        s
    });
    let mut geometry = match frame.type_code {
        TYPE_LINE => Geometry::Line(read_line(&mut r)?),
        TYPE_POINT => Geometry::Point(read_point(&mut r)?),
        TYPE_CIRCLE => Geometry::Circle(read_circle(&mut r)?),
        TYPE_LWPOLYLINE => Geometry::LwPolyline(read_lwpolyline(&mut r, version)?),
        TYPE_INSERT => {
            let (insert, count) = read_insert(&mut r, version)?;
            owned = count;
            Geometry::Insert(insert)
        }
        TYPE_TEXT => Geometry::Text(read_text(&mut r, &mut strings)?),
        TYPE_DIMENSION_ORDINATE..=TYPE_DIMENSION_DIAMETER => {
            let kind = match frame.type_code {
                TYPE_DIMENSION_ORDINATE => DimensionKind::Ordinate,
                TYPE_DIMENSION_LINEAR => DimensionKind::Linear,
                TYPE_DIMENSION_ALIGNED => DimensionKind::Aligned,
                TYPE_DIMENSION_ANGULAR_3POINT => DimensionKind::Angular3Point,
                TYPE_DIMENSION_ANGULAR_2LINE => DimensionKind::Angular2Line,
                TYPE_DIMENSION_RADIUS => DimensionKind::Radius,
                _ => DimensionKind::Diameter,
            };
            Geometry::Dimension(read_dimension(&mut r, &mut strings, version, kind)?)
        }
        TYPE_SPLINE => Geometry::Spline(read_spline(&mut r, version)?),
        TYPE_HATCH => {
            let (hatch, counts) = read_hatch(&mut r, &mut strings, version)?;
            boundary_counts = counts;
            Geometry::Hatch(hatch)
        }
        TYPE_TRACE => Geometry::Trace(read_solid(&mut r)?),
        TYPE_RAY | TYPE_XLINE => {
            let line = InfiniteLine {
                point: r.point_3bd()?,
                vector: r.point_3bd()?,
            };
            if frame.type_code == TYPE_RAY {
                Geometry::Ray(line)
            } else {
                Geometry::Xline(line)
            }
        }
        TYPE_TOLERANCE => Geometry::Tolerance(read_tolerance(&mut r, &mut strings, version)?),
        TYPE_POLYLINE_2D => {
            let (polyline, count) = read_polyline_2d(&mut r, version)?;
            owned = count;
            Geometry::Polyline2d(polyline)
        }
        TYPE_POLYLINE_3D => {
            let (polyline, count) = read_polyline_3d(&mut r, version)?;
            owned = count;
            Geometry::Polyline3d(polyline)
        }
        TYPE_VERTEX_2D => Geometry::Vertex2d(read_vertex_2d(&mut r, version)?),
        TYPE_VERTEX_3D => Geometry::Vertex3d(Vertex3d {
            flags: r.raw_char()?,
            point: r.point_3bd()?,
        }),
        TYPE_MTEXT => {
            let (mtext, application) = read_mtext(&mut r, &mut strings, version)?;
            registered_application = application;
            Geometry::Mtext(mtext)
        }
        TYPE_ATTRIB => Geometry::Attrib(read_attribute(&mut r, &mut strings, version, false)?),
        TYPE_ATTDEF => Geometry::Attdef(read_attribute(&mut r, &mut strings, version, true)?),
        TYPE_ELLIPSE => Geometry::Ellipse(read_ellipse(&mut r)?),
        TYPE_SOLID => Geometry::Solid(read_solid(&mut r)?),
        TYPE_3DFACE => Geometry::Face3d(read_face3d(&mut r)?),
        TYPE_IMAGE => Geometry::Image(read_image(&mut r, version)?),
        TYPE_LEADER => Geometry::Leader(read_leader(&mut r, version)?),
        TYPE_MLINE => Geometry::Mline(read_mline(&mut r)?),
        TYPE_VIEWPORT => {
            let (viewport, frozen) = read_viewport(&mut r, &mut strings, version)?;
            frozen_layer_count = frozen;
            Geometry::Viewport(viewport)
        }
        _ => Geometry::Arc(read_arc(&mut r)?),
    };
    // The fields must fill the main stream exactly: a kind that stops short or
    // runs over was read wrongly, whatever values it produced.
    if r.position() != layout.main_end {
        return Err(ObjectError::Unexpected {
            what: "entity data length",
        });
    }
    // The string stream, when there is one, must be read to its end too.
    if let (Some(s), Some((_, to))) = (&strings, layout.strings) {
        if s.position() != to {
            return Err(ObjectError::Unexpected {
                what: "entity string length",
            });
        }
    }
    let mut h = r.clone();
    h.seek(layout.handle_start);
    let references = read_references(&mut h, &common, flags)?;
    let style = match &mut geometry {
        Geometry::Text(t) => Some(&mut t.style),
        Geometry::Mtext(m) => Some(&mut m.style),
        Geometry::Attrib(a) | Geometry::Attdef(a) => Some(&mut a.text.style),
        _ => None,
    };
    if let Some(style) = style {
        *style = h
            .handle()?
            .resolve(common.handle)
            .ok_or(ObjectError::Unexpected {
                what: "text style handle",
            })?;
    }
    if registered_application {
        h.handle()?; // the registered application
    }
    // A polyline names its vertices -- R2000 the first and the last, later versions all
    // of them -- and then the sequence end.
    let (vertices, sequence_end) = match &mut geometry {
        Geometry::Polyline2d(p) => (Some(&mut p.vertices), Some(&mut p.sequence_end)),
        Geometry::Polyline3d(p) => (Some(&mut p.vertices), Some(&mut p.sequence_end)),
        _ => (None, None),
    };
    if let (Some(vertices), Some(sequence_end)) = (vertices, sequence_end) {
        let listed = if version >= Version::R2004 { owned } else { 2 };
        for _ in 0..listed {
            if let Some(v) = h.handle()?.resolve(common.handle).filter(|&v| v != 0) {
                vertices.push(v);
            }
        }
        *sequence_end = h.handle()?.resolve(common.handle).filter(|&v| v != 0);
    }
    if let Geometry::Hatch(hatch) = &mut geometry {
        for (path, count) in hatch.paths.iter_mut().zip(&boundary_counts) {
            for _ in 0..*count {
                if let Some(v) = h.handle()?.resolve(common.handle).filter(|&v| v != 0) {
                    path.boundary_objects.push(v);
                }
            }
        }
    }
    if let Geometry::Tolerance(t) = &mut geometry {
        t.style = h.handle()?.resolve(common.handle).filter(|&v| v != 0);
    }
    if let Geometry::Viewport(v) = &mut geometry {
        let mut next = || -> Result<Option<u64>, ObjectError> {
            Ok(h.handle()?.resolve(common.handle).filter(|&x| x != 0))
        };
        for _ in 0..frozen_layer_count {
            if let Some(layer) = next()? {
                v.frozen_layers.push(layer);
            }
        }
        v.clip_boundary = next()?;
        if version == Version::R2000 {
            next()?; // the viewport entity header
        }
        v.named_ucs = next()?;
        v.base_ucs = next()?;
        if version >= Version::R2007 {
            next()?; // the background
            next()?; // the visual style
            next()?; // the shade plot object
            next()?; // the sun
        }
    }
    if let Geometry::Mline(m) = &mut geometry {
        m.style = h.handle()?.resolve(common.handle).filter(|&x| x != 0);
    }
    if let Geometry::Leader(l) = &mut geometry {
        l.annotation = h.handle()?.resolve(common.handle).filter(|&x| x != 0);
        l.style = h.handle()?.resolve(common.handle).filter(|&x| x != 0);
    }
    if let Geometry::Image(i) = &mut geometry {
        i.definition = h.handle()?.resolve(common.handle).filter(|&x| x != 0);
        i.reactor = h.handle()?.resolve(common.handle).filter(|&x| x != 0);
    }
    if let Geometry::Dimension(d) = &mut geometry {
        // A null handle (zero) names nothing.
        let mut next = || -> Result<Option<u64>, ObjectError> {
            Ok(h.handle()?.resolve(common.handle).filter(|&v| v != 0))
        };
        d.style = next()?;
        d.block = next()?;
    }
    if let Geometry::Insert(insert) = &mut geometry {
        insert.block = h
            .handle()?
            .resolve(common.handle)
            .ok_or(ObjectError::Unexpected {
                what: "block handle",
            })?;
        if insert.has_attributes {
            let listed = if version >= Version::R2004 { owned } else { 2 };
            for _ in 0..listed {
                if let Some(v) = h.handle()?.resolve(common.handle) {
                    insert.attributes.push(v);
                }
            }
            insert.sequence_end = h.handle()?.resolve(common.handle);
        }
    }
    // The handle stream runs to the end of the object's data, padded to a byte.
    if layout.end.saturating_sub(h.position()) >= 8 {
        return Err(ObjectError::Unexpected {
            what: "handle stream length",
        });
    }
    Ok(Some(Entity {
        common,
        geometry,
        references,
    }))
}

/// A viewport: a window in a paper-space layout onto a view of model space.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    /// The centre of the window in paper space.
    pub center: [f64; 3],
    /// The width of the window.
    pub width: f64,
    /// The height of the window.
    pub height: f64,
    /// The point the view looks at.
    pub view_target: [f64; 3],
    /// The direction the view looks from.
    pub view_direction: [f64; 3],
    /// The twist of the view, in radians.
    pub view_twist: f64,
    /// The height of the view in model space units.
    pub view_height: f64,
    /// The lens length.
    pub lens_length: f64,
    /// The front clip plane distance.
    pub front_clip: f64,
    /// The back clip plane distance.
    pub back_clip: f64,
    /// The snap angle.
    pub snap_angle: f64,
    /// The centre of the view.
    pub view_center: [f64; 2],
    /// The snap base point.
    pub snap_base: [f64; 2],
    /// The snap spacing.
    pub snap_spacing: [f64; 2],
    /// The grid spacing.
    pub grid_spacing: [f64; 2],
    /// The circle zoom percent.
    pub circle_zoom: i16,
    /// How often the grid shows a major line (R2007 and later).
    pub grid_major: Option<i16>,
    /// The status flags: perspective `1`, front clip on `2`, back clip on `4`,
    /// UCS follow `8`, front clip not at the eye `16`, UCS icon on `32`, icon
    /// at the origin `64`, fast zoom `128`, snap `256`, grid `512`, turned off
    /// `0x20000`, as stored.
    pub status_flags: u32,
    /// The name of the plot style sheet.
    pub style_sheet: TextValue,
    /// The render mode.
    pub render_mode: u8,
    /// The UCS is at the origin.
    pub ucs_at_origin: bool,
    /// The viewport has a UCS of its own.
    pub ucs_per_viewport: bool,
    /// The UCS origin.
    pub ucs_origin: [f64; 3],
    /// The UCS x axis.
    pub ucs_x_axis: [f64; 3],
    /// The UCS y axis.
    pub ucs_y_axis: [f64; 3],
    /// The UCS elevation.
    pub ucs_elevation: f64,
    /// The orthographic type of the UCS.
    pub ucs_ortho_type: i16,
    /// The shade plot mode (R2004 and later).
    pub shade_plot_mode: Option<i16>,
    /// The layers frozen in this viewport.
    pub frozen_layers: Vec<u64>,
    /// The clip boundary entity, when the window is not rectangular.
    pub clip_boundary: Option<u64>,
    /// The named UCS.
    pub named_ucs: Option<u64>,
    /// The base UCS.
    pub base_ucs: Option<u64>,
}

/// Reads a viewport and the number of layers it freezes (their handles are in
/// the handle stream).
fn read_viewport(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(Viewport, usize), BitError> {
    let center = r.point_3bd()?;
    let width = r.bit_double()?;
    let height = r.bit_double()?;
    let view_target = r.point_3bd()?;
    let view_direction = r.point_3bd()?;
    let view_twist = r.bit_double()?;
    let view_height = r.bit_double()?;
    let lens_length = r.bit_double()?;
    let front_clip = r.bit_double()?;
    let back_clip = r.bit_double()?;
    let snap_angle = r.bit_double()?;
    let view_center = [r.raw_double()?, r.raw_double()?];
    let snap_base = [r.raw_double()?, r.raw_double()?];
    let snap_spacing = [r.raw_double()?, r.raw_double()?];
    let grid_spacing = [r.raw_double()?, r.raw_double()?];
    let circle_zoom = r.bit_short()?;
    let grid_major = if version >= Version::R2007 {
        Some(r.bit_short()?)
    } else {
        None
    };
    let frozen = bounded_count(r, "frozen layer count")?;
    let status_flags = r.bit_long()? as u32;
    let style_sheet = text_value(r, strings)?;
    let render_mode = r.raw_char()?;
    let ucs_at_origin = r.bit()?;
    let ucs_per_viewport = r.bit()?;
    let ucs_origin = r.point_3bd()?;
    let ucs_x_axis = r.point_3bd()?;
    let ucs_y_axis = r.point_3bd()?;
    let ucs_elevation = r.bit_double()?;
    let ucs_ortho_type = r.bit_short()?;
    let shade_plot_mode = if version >= Version::R2004 {
        Some(r.bit_short()?)
    } else {
        None
    };
    if version >= Version::R2007 {
        r.bit()?; // default lighting is used
        r.raw_char()?; // the default lighting type
        r.bit_double()?; // the brightness
        r.bit_double()?; // the contrast
        skip_color(r, strings)?; // the ambient light colour
    }
    Ok((
        Viewport {
            center,
            width,
            height,
            view_target,
            view_direction,
            view_twist,
            view_height,
            lens_length,
            front_clip,
            back_clip,
            snap_angle,
            view_center,
            snap_base,
            snap_spacing,
            grid_spacing,
            circle_zoom,
            grid_major,
            status_flags,
            style_sheet,
            render_mode,
            ucs_at_origin,
            ucs_per_viewport,
            ucs_origin,
            ucs_x_axis,
            ucs_y_axis,
            ucs_elevation,
            ucs_ortho_type,
            shade_plot_mode,
            frozen_layers: Vec::new(),
            clip_boundary: None,
            named_ucs: None,
            base_ucs: None,
        },
        frozen,
    ))
}

/// A raster image: a picture placed in the drawing. The picture itself is
/// named by an image definition (a separate object).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// The class version.
    pub class_version: i32,
    /// The insertion point (the lower left corner of the picture).
    pub insertion: [f64; 3],
    /// The vector along one pixel row, the length of one pixel.
    pub u_vector: [f64; 3],
    /// The vector along one pixel column, the length of one pixel.
    pub v_vector: [f64; 3],
    /// The size of the picture in pixels.
    pub size: [f64; 2],
    /// The display properties: show image `1`, show when not aligned with
    /// the screen `2`, use the clipping boundary `4`, transparency on `8`.
    pub display_flags: i16,
    /// The image is clipped.
    pub clipping: bool,
    /// The brightness, 0 to 100.
    pub brightness: u8,
    /// The contrast, 0 to 100.
    pub contrast: u8,
    /// The fade, 0 to 100.
    pub fade: u8,
    /// Whether the clip keeps what is inside the boundary (R2010 and later).
    pub clip_inside: Option<bool>,
    /// The boundary type: `1` a rectangle (two corners), `2` a polygon.
    pub clip_type: i16,
    /// The clip boundary points, in pixel coordinates.
    pub clip_boundary: Vec<[f64; 2]>,
    /// The image definition.
    pub definition: Option<u64>,
    /// The image definition reactor.
    pub reactor: Option<u64>,
}

fn read_image(r: &mut BitReader<'_>, version: Version) -> Result<Image, BitError> {
    // From R2010 about 150 bytes precede the class version in the files seen (the
    // clip polygon appears in them); what they are is not understood yet, and
    // reading on from there would give wrong numbers.
    let class_version = r.bit_long()?;
    let insertion = r.point_3bd()?;
    let u_vector = r.point_3bd()?;
    let v_vector = r.point_3bd()?;
    let size = [r.raw_double()?, r.raw_double()?];
    let display_flags = r.bit_short()?;
    let clipping = r.bit()?;
    let brightness = r.raw_char()?;
    let contrast = r.raw_char()?;
    let fade = r.raw_char()?;
    let clip_inside = if version >= Version::R2010 {
        Some(r.bit()?)
    } else {
        None
    };
    let clip_type = r.bit_short()?;
    let count = if clip_type == 1 {
        2
    } else {
        bounded_count(r, "image clip vertex count")?
    };
    let mut clip_boundary = Vec::with_capacity(count);
    for _ in 0..count {
        clip_boundary.push([r.raw_double()?, r.raw_double()?]);
    }
    Ok(Image {
        class_version,
        insertion,
        u_vector,
        v_vector,
        size,
        display_flags,
        clipping,
        brightness,
        contrast,
        fade,
        clip_inside,
        clip_type,
        clip_boundary,
        definition: None,
        reactor: None,
    })
}

/// A leader: a line (or a spline) from an annotation to what it points at.
#[derive(Debug, Clone, PartialEq)]
pub struct Leader {
    /// What the leader is attached to: `0` a multiline text, `1` a tolerance,
    /// `2` a block insertion, `3` nothing.
    pub annotation_type: i16,
    /// `0` straight segments, `1` a spline.
    pub path_type: i16,
    /// The vertices of the path.
    pub points: Vec<[f64; 3]>,
    /// The origin of the leader plane.
    pub origin: [f64; 3],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// The direction of the x axis of the leader plane.
    pub x_direction: [f64; 3],
    /// The offset from the end of the leader to the block insertion point.
    pub block_offset: [f64; 3],
    /// The end point projection (the offset of the annotation with the
    /// extrusion direction taken into account).
    pub end_projection: [f64; 3],
    /// The height of the annotation's text box.
    pub box_height: f64,
    /// The width of the annotation's text box.
    pub box_width: f64,
    /// The hook line lies along the x direction.
    pub hook_line_on_x: bool,
    /// The leader has an arrowhead.
    pub arrowhead: bool,
    /// The annotation the leader is attached to.
    pub annotation: Option<u64>,
    /// The dimension style.
    pub style: Option<u64>,
}

fn read_leader(r: &mut BitReader<'_>, version: Version) -> Result<Leader, BitError> {
    // The leaders of R2010 and later in the files seen are shorter than the fields
    // the specification lists for the part after the vertices, and what the
    // difference is has not been worked out; reading on would give wrong numbers.
    if version >= Version::R2010 {
        return Err(BitError::Unsupported {
            form: "a leader of R2010 and later",
        });
    }
    r.bit()?; // always zero
    let annotation_type = r.bit_short()?;
    let path_type = r.bit_short()?;
    let count = bounded_count(r, "leader vertex count")?;
    let mut points = Vec::with_capacity(count);
    for _ in 0..count {
        points.push(r.point_3bd()?);
    }
    let origin = r.point_3bd()?;
    let extrusion = r.point_3bd()?;
    let x_direction = r.point_3bd()?;
    let block_offset = r.point_3bd()?;
    let end_projection = r.point_3bd()?;
    let box_height = r.bit_double()?;
    let box_width = r.bit_double()?;
    let hook_line_on_x = r.bit()?;
    let arrowhead = r.bit()?;
    r.bit_short()?; // unknown
    r.bit()?; // unknown
    r.bit()?; // unknown
    Ok(Leader {
        annotation_type,
        path_type,
        points,
        origin,
        extrusion,
        x_direction,
        block_offset,
        end_projection,
        box_height,
        box_width,
        hook_line_on_x,
        arrowhead,
        annotation: None,
        style: None,
    })
}

/// A multiline: several parallel lines drawn along one path.
#[derive(Debug, Clone, PartialEq)]
pub struct Mline {
    /// The scale of the style's offsets.
    pub scale: f64,
    /// Which line follows the path: top `0`, centre `1`, bottom `2`.
    pub justification: u8,
    /// The base point.
    pub base_point: [f64; 3],
    /// The extrusion direction.
    pub extrusion: [f64; 3],
    /// `1` open, `3` closed.
    pub open_closed: i16,
    /// How many lines the style has.
    pub lines_in_style: u8,
    /// The vertices of the path.
    pub vertices: Vec<MlineVertex>,
    /// The multiline style.
    pub style: Option<u64>,
}

/// A vertex of a multiline.
#[derive(Debug, Clone, PartialEq)]
pub struct MlineVertex {
    /// The vertex.
    pub point: [f64; 3],
    /// The direction of the segment that starts here.
    pub direction: [f64; 3],
    /// The direction of the miter at the vertex.
    pub miter: [f64; 3],
    /// For each line of the style, the segment parameters (dash lengths).
    pub segment_parameters: Vec<Vec<f64>>,
    /// For each line of the style, the area fill parameters.
    pub area_fill_parameters: Vec<Vec<f64>>,
}

fn read_mline(r: &mut BitReader<'_>) -> Result<Mline, BitError> {
    let scale = r.bit_double()?;
    let justification = r.raw_char()?;
    let base_point = r.point_3bd()?;
    let extrusion = r.point_3bd()?;
    let open_closed = r.bit_short()?;
    let lines_in_style = r.raw_char()?;
    let count = usize::try_from(r.bit_short()?).map_err(|_| BitError::TooLong {
        form: "multiline vertex count",
    })?;
    if count > r.remaining() {
        return Err(BitError::TooLong {
            form: "multiline vertex count",
        });
    }
    let mut vertices = Vec::with_capacity(count);
    for _ in 0..count {
        let point = r.point_3bd()?;
        let direction = r.point_3bd()?;
        let miter = r.point_3bd()?;
        let mut segment_parameters = Vec::new();
        let mut area_fill_parameters = Vec::new();
        for _ in 0..lines_in_style {
            segment_parameters.push(read_bit_doubles(r, "multiline segment parameter count")?);
            area_fill_parameters.push(read_bit_doubles(r, "multiline area fill parameter count")?);
        }
        vertices.push(MlineVertex {
            point,
            direction,
            miter,
            segment_parameters,
            area_fill_parameters,
        });
    }
    Ok(Mline {
        scale,
        justification,
        base_point,
        extrusion,
        open_closed,
        lines_in_style,
        vertices,
        style: None,
    })
}

/// A count (a short) and that many bitdoubles.
fn read_bit_doubles(r: &mut BitReader<'_>, what: &'static str) -> Result<Vec<f64>, BitError> {
    let n = usize::try_from(r.bit_short()?).map_err(|_| BitError::TooLong { form: what })?;
    if n > r.remaining() {
        return Err(BitError::TooLong { form: what });
    }
    (0..n).map(|_| r.bit_double()).collect()
}
