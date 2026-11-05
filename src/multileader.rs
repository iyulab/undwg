//! The multileader entity (R2010 and later): leader roots with the lines that
//! reach them, the content they point out (a text or a block) and the
//! settings of the entity itself.
//!
//! The fields are in the order the public file specification lists them. The
//! entity embeds its annotation context (the leaders and the content) in its own
//! data, before the settings the entity states about itself; the handles the
//! fields name are in the handle stream, in the order the fields appear.

use crate::bits::{BitError, BitReader};
use crate::entities::{bounded_count, text_value};
use crate::version::Version;

/// A colour as R2004 and later write it: an index (always zero in these
/// files), the packed RGB value, and the names the flag byte says follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiLeaderColor {
    /// The colour index.
    pub index: i16,
    /// The packed value: the method in the top byte, the RGB below it.
    pub packed: u32,
}

/// An arrowhead of the entity's own arrowhead list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiLeaderArrowHead {
    /// Whether it is the default arrowhead.
    pub is_default: bool,
    /// The arrow symbol block.
    pub handle: Option<u64>,
}

/// A block attribute label of the entity (R2007 and earlier).
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderBlockLabel {
    /// The attribute definition the label belongs to.
    pub attribute_definition: Option<u64>,
    /// The label text.
    pub text: String,
    /// The sequential index of the label in the entity's collection.
    pub ui_index: i16,
    /// The width of the label.
    pub width: f64,
}

/// A leader line: the points it runs through and how it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderLine {
    /// The points of the line, from the one away from the content.
    pub points: Vec<[f64; 3]>,
    /// The breaks of the line: the segment each is on, and its start and end
    /// points.
    pub breaks: Vec<MultiLeaderBreak>,
    /// The index of the line among the root's lines.
    pub index: i32,
    /// `0` invisible, `1` straight, `2` spline.
    pub line_type: i16,
    /// The line colour.
    pub color: MultiLeaderColor,
    /// The line type object.
    pub line_type_handle: Option<u64>,
    /// The line weight.
    pub line_weight: i32,
    /// The arrowhead size.
    pub arrow_size: f64,
    /// The arrow symbol block.
    pub arrow_handle: Option<u64>,
    /// Which of the above the line states for itself rather than takes from
    /// the entity: 1 type, 2 colour, 4 line type, 8 weight, 16 arrow size,
    /// 32 arrow symbol.
    pub override_flags: i32,
}

/// A break of a leader line.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderBreak {
    /// The index of the segment the break is on.
    pub segment: i32,
    /// The start and end points of each gap.
    pub pairs: Vec<([f64; 3], [f64; 3])>,
}

/// A leader root: where the leaders meet the content, and the lines that
/// run to it.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderRoot {
    /// Whether the root has content (and so a last leader line point).
    pub content_valid: bool,
    /// Whether the root has a dog-leg.
    pub dog_leg_valid: bool,
    /// The point the lines connect at.
    pub connection_point: [f64; 3],
    /// The direction the root leaves the content in.
    pub direction: [f64; 3],
    /// The start and end points of the breaks of the root.
    pub breaks: Vec<([f64; 3], [f64; 3])>,
    /// The index of the root.
    pub index: i32,
    /// The length of the landing.
    pub landing_distance: f64,
    /// The lines that run to the root.
    pub lines: Vec<MultiLeaderLine>,
    /// `0` horizontal, `1` vertical.
    pub attachment_direction: i16,
}

/// The text a multileader points out.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderText {
    /// The text, in the format codes of a multiline text.
    pub text: String,
    /// The normal of the plane the text lies in.
    pub normal: [f64; 3],
    /// The text style.
    pub style: Option<u64>,
    /// Where the text is.
    pub location: [f64; 3],
    /// The direction of the text.
    pub direction: [f64; 3],
    /// The rotation about the normal, in radians.
    pub rotation: f64,
    /// The width of the box the text is laid out in.
    pub width: f64,
    /// The height of the box.
    pub height: f64,
    /// The line spacing factor.
    pub line_spacing_factor: f64,
    /// `1` at least, `2` exactly.
    pub line_spacing_style: i16,
    /// The text colour.
    pub color: MultiLeaderColor,
    /// `1` left, `2` centre, `3` right.
    pub alignment: i16,
    /// `1` horizontal, `3` vertical, `6` by style.
    pub flow_direction: i16,
    /// The background fill colour.
    pub background_color: MultiLeaderColor,
    /// The background scale factor.
    pub background_scale: f64,
    /// The background transparency.
    pub background_transparency: i32,
    /// Whether the background is filled.
    pub background_fill: bool,
    /// Whether the background mask is on.
    pub background_mask: bool,
    /// The column type.
    pub column_type: i16,
    /// Whether the text height is automatic.
    pub height_automatic: bool,
    /// The column width.
    pub column_width: f64,
    /// The gutter between columns.
    pub column_gutter: f64,
    /// Whether the columns flow in reverse.
    pub column_flow_reversed: bool,
    /// The size of each column.
    pub column_sizes: Vec<f64>,
    /// Whether words break at the box.
    pub word_break: bool,
}

/// The block a multileader points out.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeaderBlock {
    /// The block table record.
    pub record: Option<u64>,
    /// The normal of the plane the block lies in.
    pub normal: [f64; 3],
    /// Where the block's base point goes.
    pub location: [f64; 3],
    /// The scale along the block's axes.
    pub scale: [f64; 3],
    /// The rotation about the normal, in radians.
    pub rotation: f64,
    /// The block colour.
    pub color: MultiLeaderColor,
    /// The 16 numbers of the complete transformation matrix.
    pub matrix: [f64; 16],
}

/// What a multileader points out.
#[derive(Debug, Clone, PartialEq)]
pub enum MultiLeaderContent {
    /// A text.
    Text(MultiLeaderText),
    /// A block.
    Block(MultiLeaderBlock),
}

/// A multileader.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiLeader {
    /// The version the entity states (2).
    pub version: i16,
    /// The leader roots.
    pub roots: Vec<MultiLeaderRoot>,
    /// The overall scale.
    pub scale: f64,
    /// The base point of the content.
    pub content_base_point: [f64; 3],
    /// The text height.
    pub text_height: f64,
    /// The arrowhead size.
    pub arrow_head_size: f64,
    /// The landing gap.
    pub landing_gap: f64,
    /// The left text attachment type.
    pub left_attachment: i16,
    /// The right text attachment type.
    pub right_attachment: i16,
    /// The text align type (`0` left, `1` centre, `2` right).
    pub text_align: i16,
    /// `0` content extents, `1` insertion point.
    pub attachment_type: i16,
    /// What the leaders point out, when they point at anything.
    pub content: Option<MultiLeaderContent>,
    /// The base point.
    pub base_point: [f64; 3],
    /// The base direction.
    pub base_direction: [f64; 3],
    /// The base vertical.
    pub base_vertical: [f64; 3],
    /// Whether the normal is reversed.
    pub normal_reversed: bool,
    /// The top text attachment type.
    pub top_attachment: i16,
    /// The bottom text attachment type.
    pub bottom_attachment: i16,
    /// The multileader style.
    pub style: Option<u64>,
    /// Which settings the entity states for itself rather than takes from its
    /// style (bit 0 leader type, bit 1 colour, ...).
    pub override_flags: i32,
    /// `0` invisible, `1` straight, `2` spline.
    pub line_type: i16,
    /// The leader line colour.
    pub line_color: MultiLeaderColor,
    /// The leader line type object.
    pub line_type_handle: Option<u64>,
    /// The leader line weight.
    pub line_weight: i32,
    /// Whether a landing is enabled.
    pub landing_enabled: bool,
    /// Whether a dog-leg is enabled.
    pub dog_leg_enabled: bool,
    /// The landing distance.
    pub landing_distance: f64,
    /// The arrowhead block.
    pub arrow_handle: Option<u64>,
    /// The arrowhead size.
    pub arrow_size: f64,
    /// `0` none, `1` block, `2` text, `3` tolerance.
    pub content_type: i16,
    /// The style's text style.
    pub text_style: Option<u64>,
    /// The style's left text attachment type.
    pub style_left_attachment: i16,
    /// The style's right text attachment type.
    pub style_right_attachment: i16,
    /// The style's text angle type.
    pub text_angle_type: i16,
    /// The style's text alignment type.
    pub text_alignment_type: i16,
    /// The style's text colour.
    pub text_color: MultiLeaderColor,
    /// Whether the text frame is enabled.
    pub text_frame: bool,
    /// The style's block.
    pub block_handle: Option<u64>,
    /// The style's block colour.
    pub block_color: MultiLeaderColor,
    /// The style's block scale.
    pub block_scale: [f64; 3],
    /// The style's block rotation.
    pub block_rotation: f64,
    /// `0` content extents, `1` insertion point.
    pub style_attachment_type: i16,
    /// Whether the entity is annotative.
    pub annotative: bool,
    /// The entity's own arrowheads.
    pub arrow_heads: Vec<MultiLeaderArrowHead>,
    /// The entity's block attribute labels (R2007 and earlier; none after).
    pub block_labels: Vec<MultiLeaderBlockLabel>,
    /// Whether the text direction is negative.
    pub text_direction_negative: bool,
    /// The IPE alignment (meaning unknown).
    pub ipe_align: i16,
    /// The justification (`1` left, `2` centre, `3` right).
    pub justification: i16,
    /// The scale factor.
    pub scale_factor: f64,
    /// `0` horizontal, `1` vertical.
    pub attachment_direction: i16,
    /// The style's top text attachment type.
    pub style_top_attachment: i16,
    /// The style's bottom text attachment type.
    pub style_bottom_attachment: i16,
    /// Whether the leader is extended to the text (R2013 and later).
    pub extended_to_text: bool,
}

/// A handle the main stream's fields leave for the handle stream.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Slot {
    LineType(usize, usize),
    LineArrow(usize, usize),
    TextStyle,
    BlockRecord,
    Style,
    EntityLineType,
    EntityArrow,
    EntityTextStyle,
    EntityBlock,
    ArrowHead(usize),
    BlockLabel(usize),
}

fn point(r: &mut BitReader<'_>) -> Result<[f64; 3], BitError> {
    r.point_3bd()
}

fn color(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<MultiLeaderColor, BitError> {
    let index = r.bit_short()?;
    if version < Version::R2004 {
        // R2000 states the colour as its index alone.
        return Ok(MultiLeaderColor { index, packed: 0 });
    }
    let packed = r.bit_long()? as u32;
    let flags = r.raw_char()?;
    if flags & 1 != 0 {
        text_value(r, strings)?;
    }
    if flags & 2 != 0 {
        text_value(r, strings)?;
    }
    Ok(MultiLeaderColor { index, packed })
}

/// Reads a multileader: the fields, and the handles they leave for the handle
/// stream, in the order they are read. The fields the specification marks R2010 are
/// left out of the older layouts, which have the block labels instead.
pub(crate) fn read_multileader(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(MultiLeader, Vec<Slot>), BitError> {
    let mut slots = Vec::new();
    let r2010 = version >= Version::R2010;
    let entity_version = if r2010 { r.bit_short()? } else { 0 };

    // The annotation context: the leader roots.
    let root_count = bounded_count(r, "multileader root count")?;
    let mut roots = Vec::with_capacity(root_count);
    for root_at in 0..root_count {
        let content_valid = r.bit()?;
        let dog_leg_valid = r.bit()?;
        let connection_point = point(r)?;
        let direction = point(r)?;
        let break_count = bounded_count(r, "multileader break count")?;
        let mut breaks = Vec::with_capacity(break_count);
        for _ in 0..break_count {
            breaks.push((point(r)?, point(r)?));
        }
        let index = r.bit_long()?;
        let landing_distance = r.bit_double()?;
        let line_count = bounded_count(r, "multileader line count")?;
        let mut lines = Vec::with_capacity(line_count);
        for line_at in 0..line_count {
            let point_count = bounded_count(r, "multileader point count")?;
            let mut points = Vec::with_capacity(point_count);
            for _ in 0..point_count {
                points.push(point(r)?);
            }
            let break_info_count = bounded_count(r, "multileader break info count")?;
            let mut line_breaks = Vec::with_capacity(break_info_count);
            for _ in 0..break_info_count {
                let segment = r.bit_long()?;
                let pair_count = bounded_count(r, "multileader break pair count")?;
                let mut pairs = Vec::with_capacity(pair_count);
                for _ in 0..pair_count {
                    pairs.push((point(r)?, point(r)?));
                }
                line_breaks.push(MultiLeaderBreak { segment, pairs });
            }
            let line_index = r.bit_long()?;
            let (line_type, line_color, line_weight, arrow_size, override_flags) = if r2010 {
                let line_type = r.bit_short()?;
                let line_color = color(r, strings, version)?;
                slots.push(Slot::LineType(root_at, line_at));
                let line_weight = r.bit_long()?;
                let arrow_size = r.bit_double()?;
                slots.push(Slot::LineArrow(root_at, line_at));
                let override_flags = r.bit_long()?;
                (
                    line_type,
                    line_color,
                    line_weight,
                    arrow_size,
                    override_flags,
                )
            } else {
                // Older layouts keep these on the entity only; no override of its own.
                (
                    0,
                    MultiLeaderColor {
                        index: 0,
                        packed: 0,
                    },
                    0,
                    0.0,
                    0,
                )
            };
            lines.push(MultiLeaderLine {
                points,
                breaks: line_breaks,
                index: line_index,
                line_type,
                color: line_color,
                line_type_handle: None,
                line_weight,
                arrow_size,
                arrow_handle: None,
                override_flags,
            });
        }
        let attachment_direction = if r2010 { r.bit_short()? } else { 0 };
        roots.push(MultiLeaderRoot {
            content_valid,
            dog_leg_valid,
            connection_point,
            direction,
            breaks,
            index,
            landing_distance,
            lines,
            attachment_direction,
        });
    }

    // The context's own settings and its content.
    let scale = r.bit_double()?;
    let content_base_point = point(r)?;
    let text_height = r.bit_double()?;
    let arrow_head_size = r.bit_double()?;
    let landing_gap = r.bit_double()?;
    let left_attachment = r.bit_short()?;
    let right_attachment = r.bit_short()?;
    let text_align = r.bit_short()?;
    let attachment_type = r.bit_short()?;
    let content = if r.bit()? {
        let text = text_value(r, strings)?.plain().unwrap_or_default();
        let normal = point(r)?;
        slots.push(Slot::TextStyle);
        let location = point(r)?;
        let direction = point(r)?;
        let rotation = r.bit_double()?;
        let width = r.bit_double()?;
        let height = r.bit_double()?;
        let line_spacing_factor = r.bit_double()?;
        let line_spacing_style = r.bit_short()?;
        let text_color = color(r, strings, version)?;
        let alignment = r.bit_short()?;
        let flow_direction = r.bit_short()?;
        let background_color = color(r, strings, version)?;
        let background_scale = r.bit_double()?;
        let background_transparency = r.bit_long()?;
        let background_fill = r.bit()?;
        let background_mask = r.bit()?;
        let column_type = r.bit_short()?;
        let height_automatic = r.bit()?;
        let column_width = r.bit_double()?;
        let column_gutter = r.bit_double()?;
        let column_flow_reversed = r.bit()?;
        let size_count = bounded_count(r, "multileader column count")?;
        let mut column_sizes = Vec::with_capacity(size_count);
        for _ in 0..size_count {
            column_sizes.push(r.bit_double()?);
        }
        let word_break = r.bit()?;
        r.bit()?; // unknown
        Some(MultiLeaderContent::Text(MultiLeaderText {
            text,
            normal,
            style: None,
            location,
            direction,
            rotation,
            width,
            height,
            line_spacing_factor,
            line_spacing_style,
            color: text_color,
            alignment,
            flow_direction,
            background_color,
            background_scale,
            background_transparency,
            background_fill,
            background_mask,
            column_type,
            height_automatic,
            column_width,
            column_gutter,
            column_flow_reversed,
            column_sizes,
            word_break,
        }))
    } else if r.bit()? {
        slots.push(Slot::BlockRecord);
        let normal = point(r)?;
        let location = point(r)?;
        let block_scale = point(r)?;
        let rotation = r.bit_double()?;
        let block_color = color(r, strings, version)?;
        let mut matrix = [0.0; 16];
        for m in &mut matrix {
            *m = r.bit_double()?;
        }
        Some(MultiLeaderContent::Block(MultiLeaderBlock {
            record: None,
            normal,
            location,
            scale: block_scale,
            rotation,
            color: block_color,
            matrix,
        }))
    } else {
        None
    };
    let base_point = point(r)?;
    let base_direction = point(r)?;
    let base_vertical = point(r)?;
    let normal_reversed = r.bit()?;
    let (top_attachment, bottom_attachment) = if r2010 {
        (r.bit_short()?, r.bit_short()?)
    } else {
        (0, 0)
    };

    // The settings of the entity itself.
    slots.push(Slot::Style);
    let override_flags = r.bit_long()?;
    let line_type = r.bit_short()?;
    let line_color = color(r, strings, version)?;
    slots.push(Slot::EntityLineType);
    let line_weight = r.bit_long()?;
    let landing_enabled = r.bit()?;
    let dog_leg_enabled = r.bit()?;
    let landing_distance = r.bit_double()?;
    slots.push(Slot::EntityArrow);
    let arrow_size = r.bit_double()?;
    let content_type = r.bit_short()?;
    slots.push(Slot::EntityTextStyle);
    let style_left_attachment = r.bit_short()?;
    let style_right_attachment = r.bit_short()?;
    let text_angle_type = r.bit_short()?;
    let text_alignment_type = r.bit_short()?;
    let text_color = color(r, strings, version)?;
    let text_frame = r.bit()?;
    slots.push(Slot::EntityBlock);
    let block_color = color(r, strings, version)?;
    let block_scale = point(r)?;
    let block_rotation = r.bit_double()?;
    let style_attachment_type = r.bit_short()?;
    let annotative = r.bit()?;
    let arrow_head_count = bounded_count(r, "multileader arrowhead count")?;
    let mut arrow_heads = Vec::with_capacity(arrow_head_count);
    for at in 0..arrow_head_count {
        arrow_heads.push(MultiLeaderArrowHead {
            is_default: r.bit()?,
            handle: None,
        });
        slots.push(Slot::ArrowHead(at));
    }
    let mut block_labels = Vec::new();
    if !r2010 {
        let label_count = bounded_count(r, "multileader block label count")?;
        for at in 0..label_count {
            slots.push(Slot::BlockLabel(at));
            block_labels.push(MultiLeaderBlockLabel {
                attribute_definition: None,
                text: text_value(r, strings)?.plain().unwrap_or_default(),
                ui_index: r.bit_short()?,
                width: r.bit_double()?,
            });
        }
    }
    let text_direction_negative = r.bit()?;
    let ipe_align = r.bit_short()?;
    let justification = r.bit_short()?;
    let scale_factor = r.bit_double()?;
    let (attachment_direction, style_top_attachment, style_bottom_attachment) = if r2010 {
        (r.bit_short()?, r.bit_short()?, r.bit_short()?)
    } else {
        (0, 0, 0)
    };
    let extended_to_text = if version >= Version::R2013 {
        r.bit()?
    } else {
        false
    };

    Ok((
        MultiLeader {
            version: entity_version,
            roots,
            scale,
            content_base_point,
            text_height,
            arrow_head_size,
            landing_gap,
            left_attachment,
            right_attachment,
            text_align,
            attachment_type,
            content,
            base_point,
            base_direction,
            base_vertical,
            normal_reversed,
            top_attachment,
            bottom_attachment,
            style: None,
            override_flags,
            line_type,
            line_color,
            line_type_handle: None,
            line_weight,
            landing_enabled,
            dog_leg_enabled,
            landing_distance,
            arrow_handle: None,
            arrow_size,
            content_type,
            text_style: None,
            style_left_attachment,
            style_right_attachment,
            text_angle_type,
            text_alignment_type,
            text_color,
            text_frame,
            block_handle: None,
            block_color,
            block_scale,
            block_rotation,
            style_attachment_type,
            annotative,
            arrow_heads,
            block_labels,
            text_direction_negative,
            ipe_align,
            justification,
            scale_factor,
            attachment_direction,
            style_top_attachment,
            style_bottom_attachment,
            extended_to_text,
        },
        slots,
    ))
}

impl MultiLeader {
    /// Stores the handle a slot names.
    pub(crate) fn set_slot(&mut self, slot: Slot, handle: Option<u64>) {
        match slot {
            Slot::LineType(root, line) => self.roots[root].lines[line].line_type_handle = handle,
            Slot::LineArrow(root, line) => self.roots[root].lines[line].arrow_handle = handle,
            Slot::TextStyle => {
                if let Some(MultiLeaderContent::Text(t)) = &mut self.content {
                    t.style = handle;
                }
            }
            Slot::BlockRecord => {
                if let Some(MultiLeaderContent::Block(b)) = &mut self.content {
                    b.record = handle;
                }
            }
            Slot::Style => self.style = handle,
            Slot::EntityLineType => self.line_type_handle = handle,
            Slot::EntityArrow => self.arrow_handle = handle,
            Slot::EntityTextStyle => self.text_style = handle,
            Slot::EntityBlock => self.block_handle = handle,
            Slot::ArrowHead(at) => self.arrow_heads[at].handle = handle,
            Slot::BlockLabel(at) => self.block_labels[at].attribute_definition = handle,
        }
    }
}
