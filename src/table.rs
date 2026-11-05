//! The table entity (`ACAD_TABLE`) as R2000 to R2007 write it: the placement a
//! table shares with a block insertion, its columns and rows, every cell, and
//! the overrides the table states over its style.
//!
//! The fields are in the order the public file specification lists them for
//! "until R21". R2010 and later keep a table's content in an embedded content
//! structure and are not read here.
//!
//! A table is read whole or not at all: its main stream must end exactly where
//! the last field does, and so must its handle stream. A table that does not
//! is reported by the caller as an entity of a kind this reader does not read.

use crate::bits::{BitError, BitReader, TextValue};
use crate::entities::{
    bounded_count, read_common, read_insert, read_references, text_value, CommonEntity, Entity,
    Geometry, Insert,
};
use crate::objects::{open_object, ObjectEntry, ObjectError};
use crate::tables::read_full_color;
use crate::version::Version;

/// One cell of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    /// What the cell holds: `1` a text, `2` a block.
    pub kind: i16,
    /// Whether the cell is merged with another cell.
    pub merged: bool,
    /// How many columns the cell spans (`1` for a cell that spans none).
    pub span_columns: i32,
    /// How many rows the cell spans (`1` for a cell that spans none).
    pub span_rows: i32,
    /// The text of a text cell as the file states it directly in the cell; an
    /// empty text is kept (it is a statement of an empty cell).
    pub text: Option<TextValue>,
    /// The kind of value the cell states (R2007 and later), as stored; `None`
    /// before that. `4` is a string.
    pub value_kind: Option<i32>,
    /// The string the value states when it is a string (R2007 and later).
    pub value_text: Option<TextValue>,
}

/// A table.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The placement the table shares with a block insertion.
    pub insert: Insert,
    /// The table flags as stored: bit `0x10` the rows run up, `0x20` the
    /// title is suppressed.
    pub flags: i16,
    /// The horizontal direction.
    pub direction: [f64; 3],
    /// The width of each column.
    pub column_widths: Vec<f64>,
    /// The height of each row.
    pub row_heights: Vec<f64>,
    /// The cells, row by row (`row_heights.len() * column_widths.len()`).
    pub cells: Vec<TableCell>,
    /// The flow direction the table states over its style (`0` down, `1`
    /// up); `None` when it states none.
    pub flow: Option<i16>,
    /// The table style.
    pub style: Option<u64>,
}

/// Reads the table the object at `entry` is, for a version this reader reads
/// tables of; the caller has found the class that names it.
pub(crate) fn read_table_entity(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Entity, ObjectError> {
    if version >= Version::R2010 {
        return Err(ObjectError::Unexpected {
            what: "table version",
        });
    }
    let (_, mut r, layout) = open_object(objects, entry, version)?;
    let (common, flags) = read_common(&mut r, version)?;
    if common.handle != entry.handle {
        return Err(ObjectError::Unexpected {
            what: "entity handle",
        });
    }
    let mut strings = layout.strings.map(|(from, _)| {
        let mut s = r.clone();
        s.seek(from);
        s
    });
    let (mut table, owned, cell_plan) = read_fields(&mut r, &mut strings, version)?;
    if r.position() != layout.main_end {
        return Err(ObjectError::Unexpected {
            what: "table data length",
        });
    }
    if let (Some(s), Some((_, to))) = (&strings, layout.strings) {
        if s.position() != to {
            return Err(ObjectError::Unexpected {
                what: "table string length",
            });
        }
    }
    let mut h = r.clone();
    h.seek(layout.handle_start);
    let references = read_references(&mut h, &common, flags)?;
    read_handles(&mut h, &common, version, owned, &cell_plan, &mut table)?;
    if layout.end.saturating_sub(h.position()) >= 8 {
        return Err(ObjectError::Unexpected {
            what: "table handle stream length",
        });
    }
    Ok(Entity {
        common,
        geometry: Geometry::Table(Box::new(table)),
        references,
    })
}

/// What the handle stream holds for a cell, as far as the main stream says.
#[derive(Debug, Clone, Copy)]
struct CellHandles {
    /// The cell is a block cell and lists this many attribute definitions.
    attribute_definitions: usize,
    /// The cell overrides the text style.
    text_style: bool,
}

/// What the handle stream holds besides the common handles.
#[derive(Debug, Clone)]
struct CellPlan {
    cells: Vec<CellHandles>,
    /// The text styles the table overrides.
    style_handles: usize,
}

/// The fields of a table; the table with its own handles still unread, the
/// number of owned attributes, and what each cell leaves for the handle stream.
fn read_fields(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(Table, usize, CellPlan), BitError> {
    let (mut insert, owned) = read_insert(r, version)?;
    insert.block = 0;
    let flags = r.bit_short()?;
    let direction = r.point_3bd()?;
    let columns = bounded_count(r, "table column count")?;
    let rows = bounded_count(r, "table row count")?;
    let cell_count = columns
        .checked_mul(rows)
        .filter(|&n| n <= r.remaining())
        .ok_or(BitError::TooLong {
            form: "table cell count",
        })?;
    let mut column_widths = Vec::with_capacity(columns);
    for _ in 0..columns {
        column_widths.push(r.bit_double()?);
    }
    let mut row_heights = Vec::with_capacity(rows);
    for _ in 0..rows {
        row_heights.push(r.bit_double()?);
    }
    let mut cells = Vec::with_capacity(cell_count);
    let mut plan = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        let (cell, handles) = read_cell(r, strings, version)?;
        cells.push(cell);
        plan.push(handles);
    }
    let (flow, style_handles) = read_table_overrides(r, strings)?;
    Ok((
        Table {
            insert,
            flags,
            direction,
            column_widths,
            row_heights,
            cells,
            flow,
            style: None,
        },
        owned,
        CellPlan {
            cells: plan,
            style_handles,
        },
    ))
}

fn read_cell(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<(TableCell, CellHandles), BitError> {
    let kind = r.bit_short()?;
    r.raw_char()?; // the edges that have property overrides
    let merged = r.bit()?;
    r.bit()?; // autofit
    let span_columns = r.bit_long()?;
    let span_rows = r.bit_long()?;
    r.bit_double()?; // the rotation of the content
    let mut text = None;
    let mut attribute_definitions = 0usize;
    match kind {
        // From R2007 a text cell keeps its text in its value; measured on real
        // files, the cell states no text of its own there.
        1 if version < Version::R2007 => text = Some(text_value(r, strings)?),
        2 => {
            r.bit_double()?; // the block scale
            if r.bit()? {
                let count = usize::try_from(r.bit_short()?).map_err(|_| BitError::TooLong {
                    form: "table attribute definition count",
                })?;
                if count > r.remaining() {
                    return Err(BitError::TooLong {
                        form: "table attribute definition count",
                    });
                }
                for _ in 0..count {
                    r.bit_short()?; // the index
                    text_value(r, strings)?;
                }
                attribute_definitions = count;
            }
        }
        _ => {}
    }
    let mut text_style = false;
    if r.bit()? {
        let overrides = r.bit_long()? as u32;
        r.raw_char()?; // the edges that are virtual
        text_style = overrides & 0x10 != 0;
        skip_cell_overrides(r, strings, overrides)?;
    }
    let (mut value_kind, mut value_text) = (None, None);
    if version >= Version::R2007 {
        r.bit_long()?; // unknown
        let (k, t) = read_value(r, strings)?;
        value_kind = k;
        value_text = t;
    }
    Ok((
        TableCell {
            kind,
            merged,
            span_columns,
            span_rows,
            text,
            value_kind,
            value_text,
        },
        CellHandles {
            attribute_definitions,
            text_style,
        },
    ))
}

/// A value (R2007 and later): its flags, the kind of data, the data, the unit
/// type, the format string and the string of the value.
fn read_value(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
) -> Result<(Option<i32>, Option<TextValue>), BitError> {
    let flags = r.bit_long()?;
    let kind = r.bit_long()?;
    let mut text = None;
    if flags & 1 == 0 {
        match kind {
            0 | 1 => {
                r.bit_long()?;
            }
            2 => {
                r.bit_double()?;
            }
            4 => {
                // The string is a byte count and UTF-16 bytes in the main
                // stream, closed by a zero (measured on real files: the
                // specification names a text value here).
                let size = bounded_count(r, "table value size")?;
                let mut units = Vec::with_capacity(size / 2);
                for _ in 0..size / 2 {
                    units.push(r.raw_short()?);
                }
                if size % 2 != 0 {
                    r.raw_char()?;
                }
                while units.last() == Some(&0) {
                    units.pop();
                }
                text = Some(TextValue::Unicode(
                    String::from_utf16(&units).map_err(|_| BitError::BadText)?,
                ));
            }
            8 | 16 | 32 | 512 => {
                let size = bounded_count(r, "table value size")?;
                for _ in 0..size {
                    r.raw_char()?;
                }
            }
            _ => {
                return Err(BitError::TooLong {
                    form: "table value kind",
                })
            }
        }
    }
    r.bit_long()?; // the unit type
    text_value(r, strings)?; // the format string
    text_value(r, strings)?; // the string of the value
    Ok((Some(kind), text))
}

fn skip_cell_overrides(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    flags: u32,
) -> Result<(), BitError> {
    if flags & 0x01 != 0 {
        r.raw_short()?; // the alignment
    }
    if flags & 0x02 != 0 {
        r.bit()?; // no background fill
    }
    if flags & 0x04 != 0 {
        read_full_color(r, strings)?; // the background colour
    }
    if flags & 0x08 != 0 {
        read_full_color(r, strings)?; // the content colour
    }
    if flags & 0x20 != 0 {
        r.bit_double()?; // the text height
    }
    // The four edges -- top, right, bottom, left -- each with a colour, a line
    // weight and a visibility.
    for (colour, weight, visible) in [
        (0x40, 0x400, 0x4000),
        (0x80, 0x800, 0x8000),
        (0x100, 0x1000, 0x10000),
        (0x200, 0x2000, 0x20000),
    ] {
        if flags & colour != 0 {
            read_full_color(r, strings)?;
        }
        if flags & weight != 0 {
            r.bit_short()?;
        }
        if flags & visible != 0 {
            r.bit_short()?;
        }
    }
    Ok(())
}

/// The overrides the table states over its style, and the border overrides;
/// returns the flow direction when the table states one and how many text
/// styles the table overrides (handles in the handle stream).
fn read_table_overrides(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
) -> Result<(Option<i16>, usize), BitError> {
    let mut flow = None;
    let mut style_handles = 0usize;
    if r.bit()? {
        let overrides = r.bit_long()? as u32;
        if overrides & 0x0001 != 0 {
            r.bit()?; // the title is suppressed
        }
        if overrides & 0x0004 != 0 {
            flow = Some(r.bit_short()?);
        }
        if overrides & 0x0008 != 0 {
            r.bit_double()?; // the horizontal cell margin
        }
        if overrides & 0x0010 != 0 {
            r.bit_double()?; // the vertical cell margin
        }
        for bit in [0x20, 0x40, 0x80] {
            if overrides & bit != 0 {
                read_full_color(r, strings)?; // the title, header, data row colour
            }
        }
        for bit in [0x100, 0x200, 0x400] {
            if overrides & bit != 0 {
                r.bit()?; // no fill
            }
        }
        for bit in [0x800, 0x1000, 0x2000] {
            if overrides & bit != 0 {
                read_full_color(r, strings)?; // the fill colours
            }
        }
        for bit in [0x4000, 0x8000, 0x10000] {
            if overrides & bit != 0 {
                r.bit_short()?; // the alignments
            }
        }
        // 0x20000 to 0x80000: text styles, in the handle stream.
        for bit in [0x100000, 0x200000, 0x400000] {
            if overrides & bit != 0 {
                r.bit_double()?; // the row heights
            }
        }
        style_handles = (overrides & 0x20000 != 0) as usize
            + (overrides & 0x40000 != 0) as usize
            + (overrides & 0x80000 != 0) as usize;
    }
    // Border colours, line weights and visibilities: eighteen entries each,
    // flagged bit by bit.
    for kind in 0..3 {
        if r.bit()? {
            let overrides = r.bit_long()? as u32;
            for bit in 0..18 {
                if overrides & (1 << bit) != 0 {
                    if kind == 0 {
                        read_full_color(r, strings)?;
                    } else {
                        r.bit_short()?;
                    }
                }
            }
        }
    }
    Ok((flow, style_handles))
}

fn read_handles(
    h: &mut BitReader<'_>,
    common: &CommonEntity,
    version: Version,
    owned: usize,
    plan: &CellPlan,
    table: &mut Table,
) -> Result<(), ObjectError> {
    let next = |h: &mut BitReader<'_>| -> Result<Option<u64>, ObjectError> {
        Ok(h.handle()?.resolve(common.handle).filter(|&v| v != 0))
    };
    table.insert.block = next(h)?.ok_or(ObjectError::Unexpected {
        what: "block handle",
    })?;
    if table.insert.has_attributes {
        let listed = if version >= Version::R2004 { owned } else { 2 };
        for _ in 0..listed {
            if let Some(v) = next(h)? {
                table.insert.attributes.push(v);
            }
        }
        table.insert.sequence_end = next(h)?;
    }
    table.style = next(h)?;
    for cell in &plan.cells {
        next(h)?; // the cell's field or block
        for _ in 0..cell.attribute_definitions {
            next(h)?;
        }
        if cell.text_style {
            next(h)?;
        }
    }
    for _ in 0..plan.style_handles {
        next(h)?;
    }
    Ok(())
}
