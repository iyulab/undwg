//! Table entries: the named objects that entities refer to.
//!
//! The name of any entry can be read; layers and block headers are read in
//! full. The fields of an entry end exactly where its main stream ends -- that
//! is checked.

use crate::bits::{BitError, BitReader, TextValue};
use crate::classes::ClassDef;
use crate::objects::{open_object, skip_extended_data, Layout, ObjectEntry, ObjectError};
use crate::version::Version;

/// The type code of a block header.
pub const TYPE_BLOCK_HEADER: u16 = 49;

/// The type code of a layer.
pub const TYPE_LAYER: u16 = 51;
/// The type code of a registered application.
pub const TYPE_APPID: u16 = 67;
/// The type code of a text style.
pub const TYPE_STYLE: u16 = 53;

/// The type code of a linetype.
pub const TYPE_LINETYPE: u16 = 57;

/// The type code of a dimension style.
pub const TYPE_DIMSTYLE: u16 = 69;

/// The type code of a multiline style.
pub const TYPE_MLINESTYLE: u16 = 73;

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

/// The type code of the block table's control object.
pub const TYPE_BLOCK_CONTROL: u16 = 48;

/// Reads the block table's control object at `entry`: the handles of the block
/// headers in table order -- the blocks the file counts, then model space and
/// paper space. A place the table has emptied (a deleted block) is `None`, and
/// still counts. `Ok(None)` when the object is of another kind.
pub fn read_block_control(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<Vec<Option<u64>>>, ObjectError> {
    let Some(Prefix {
        mut r,
        layout,
        reactors,
        xdictionary_stored,
    }) = open_prefix(objects, entry, version, |code| code == TYPE_BLOCK_CONTROL)?
    else {
        return Ok(None);
    };
    let count = r.bit_long()?;
    if r.position() != layout.main_end {
        return Err(ObjectError::Unexpected {
            what: "block control main data length",
        });
    }
    let count = usize::try_from(count)
        .ok()
        .filter(|&n| n <= r.remaining() / 8)
        .ok_or(ObjectError::Unexpected {
            what: "block control entry count",
        })?;
    let mut h = r.clone();
    h.seek(layout.handle_start);
    let mut handles = Handles {
        h,
        holder: entry.handle,
    };
    handles.next()?; // the owner
    let reactor_count = usize::try_from(reactors)
        .ok()
        .filter(|&n| n <= handles.h.remaining() / 8)
        .ok_or(ObjectError::Unexpected {
            what: "reactor count",
        })?;
    for _ in 0..reactor_count {
        handles.next()?;
    }
    if xdictionary_stored {
        handles.next()?;
    }
    let mut order = Vec::with_capacity(count + 2);
    // The listed blocks, then model space and paper space.
    for _ in 0..count + 2 {
        order.push(handles.next()?);
    }
    Ok(Some(order))
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

/// Reads the name of the registered application at `entry`; `Ok(None)` when
/// the object is of another kind.
pub fn read_appid_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_APPID)
}

/// Reads the name of the text style at `entry`; `Ok(None)` when the object is
/// of another kind.
pub fn read_style_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_STYLE)
}

/// Reads the name of the linetype at `entry`; `Ok(None)` when the object is of
/// another kind.
pub fn read_linetype_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_LINETYPE)
}

/// Reads the name of the multiline style at `entry`; `Ok(None)` when the
/// object is of another kind.
pub fn read_mlinestyle_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<TextValue>, ObjectError> {
    read_table_name(objects, entry, version, TYPE_MLINESTYLE)
}

fn read_table_name(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
    type_code: u16,
) -> Result<Option<TextValue>, ObjectError> {
    Ok(open_entry(objects, entry, version, type_code)?.map(|opened| opened.name))
}

/// A table entry read up to and including its name.
struct Opened<'a> {
    /// Positioned just after the name (in the main stream).
    r: BitReader<'a>,
    layout: Layout,
    /// The string stream, positioned after the name (R2007 and later).
    strings: Option<BitReader<'a>>,
    reactors: i32,
    xdictionary_stored: bool,
    name: TextValue,
}

/// The data every object has, read: the reader positioned after it.
struct Prefix<'a> {
    r: BitReader<'a>,
    layout: Layout,
    reactors: i32,
    xdictionary_stored: bool,
}

/// Opens the object at `entry` and reads the data every object has, when its
/// type code is `type_code`; `Ok(None)` for another kind.
fn open_prefix<'a>(
    objects: &'a [u8],
    entry: ObjectEntry,
    version: Version,
    accepts: impl Fn(u16) -> bool,
) -> Result<Option<Prefix<'a>>, ObjectError> {
    let (frame, mut r, layout) = open_object(objects, entry, version)?;
    if !accepts(frame.type_code) {
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
    let reactors = r.bit_long()?;
    let xdictionary_stored = version < Version::R2004 || !r.bit()?;
    if version >= Version::R2013 {
        r.bit()?; // binary data is kept in the data store
    }
    Ok(Some(Prefix {
        r,
        layout,
        reactors,
        xdictionary_stored,
    }))
}

/// Opens the table entry at `entry` and reads the data every object has, then
/// the name. `Ok(None)` when the object is of another kind.
fn open_entry<'a>(
    objects: &'a [u8],
    entry: ObjectEntry,
    version: Version,
    type_code: u16,
) -> Result<Option<Opened<'a>>, ObjectError> {
    let Some(Prefix {
        mut r,
        layout,
        reactors,
        xdictionary_stored,
    }) = open_prefix(objects, entry, version, |code| code == type_code)?
    else {
        return Ok(None);
    };
    let mut strings = layout.strings.map(|(from, _)| {
        let mut s = r.clone();
        s.seek(from);
        s
    });
    let name = text_value(&mut r, &mut strings)?;
    if let (Some(s), Some((_, to))) = (&strings, layout.strings) {
        if s.position() > to {
            return Err(ObjectError::Unexpected {
                what: "table entry string length",
            });
        }
    }
    Ok(Some(Opened {
        r,
        layout,
        strings,
        reactors,
        xdictionary_stored,
        name,
    }))
}

impl Opened<'_> {
    /// The main stream must end exactly where the fields do.
    fn check_main_end(&self, what: &'static str) -> Result<(), ObjectError> {
        if self.r.position() == self.layout.main_end {
            Ok(())
        } else {
            Err(ObjectError::Unexpected { what })
        }
    }

    /// The handle stream, positioned after the owner, the reactors and the
    /// extension dictionary; also returns the owner.
    fn handles(&self, holder: u64) -> Result<(Handles<'_>, Option<u64>), ObjectError> {
        let mut h = self.r.clone();
        h.seek(self.layout.handle_start);
        let mut handles = Handles { h, holder };
        let owner = handles.next()?;
        let reactor_count = usize::try_from(self.reactors)
            .ok()
            .filter(|&n| n <= handles.h.remaining() / 8)
            .ok_or(ObjectError::Unexpected {
                what: "reactor count",
            })?;
        for _ in 0..reactor_count {
            handles.next()?;
        }
        if self.xdictionary_stored {
            handles.next()?;
        }
        Ok((handles, owner))
    }
}

/// The handle stream of an object.
struct Handles<'a> {
    h: BitReader<'a>,
    holder: u64,
}

impl Handles<'_> {
    fn next(&mut self) -> Result<Option<u64>, ObjectError> {
        Ok(self.h.handle()?.resolve(self.holder).filter(|&v| v != 0))
    }
}

/// Skips what a table entry states about being an external reference: from
/// R2007 one short (the index of the reference plus one), before that a bit, a
/// short and a bit (the 64-bit of the DXF flags, the index, and whether the
/// entry depends on a reference).
fn skip_xref_fields(r: &mut BitReader<'_>, version: Version) -> Result<(), BitError> {
    if version >= Version::R2007 {
        r.bit_short()?;
    } else {
        r.bit()?;
        r.bit_short()?;
        r.bit()?;
    }
    Ok(())
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

/// Reads a colour as a table entry states it.
fn read_table_color(
    r: &mut BitReader<'_>,
    strings: &mut Option<BitReader<'_>>,
    version: Version,
) -> Result<TableColor, BitError> {
    if version >= Version::R2004 {
        let index = r.bit_short()? as u16;
        let rgb = r.bit_long()? as u32;
        let names = r.raw_char()?;
        if names & 1 != 0 {
            text_value(r, strings)?; // the colour name
        }
        if names & 2 != 0 {
            text_value(r, strings)?; // the colour book name
        }
        Ok(TableColor {
            method: (rgb >> 24) as u8,
            value: rgb & 0x00FF_FFFF,
            index,
        })
    } else {
        let value = r.bit_short()? as u16;
        Ok(TableColor {
            method: 0xC3,
            value: u32::from(value),
            index: value,
        })
    }
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
    /// The colour number the entry also states (R2004 and later keep it next
    /// to the packed colour; it is the number a reader without true colour
    /// would show). For R2000 it is the colour number itself.
    pub index: u16,
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
    let Some(mut o) = open_entry(objects, entry, version, TYPE_LAYER)? else {
        return Ok(None);
    };
    skip_xref_fields(&mut o.r, version)?;
    let flags = o.r.bit_short()? as u16;
    let color = read_table_color(&mut o.r, &mut o.strings, version)?;
    o.check_main_end("layer main data length")?;
    let (mut h, owner) = o.handles(entry.handle)?;
    let xref_block = h.next()?;
    let plotstyle = h.next()?;
    let material = if version >= Version::R2007 {
        h.next()?
    } else {
        None
    };
    let linetype = h.next()?;
    Ok(Some(Layer {
        handle: entry.handle,
        name: o.name,
        flags,
        color,
        owner,
        xref_block,
        plotstyle,
        material,
        linetype,
    }))
}

/// A block header: the table entry that names a block and states where it is
/// based and whether it comes from another drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockHeader {
    /// The block header's handle.
    pub handle: u64,
    /// The block name.
    pub name: TextValue,
    /// An anonymous block (the file stores such a block under a name that
    /// starts with `*` and a letter).
    pub anonymous: bool,
    /// The block holds attribute definitions.
    pub has_attribute_definitions: bool,
    /// The block is an external reference.
    pub is_xref: bool,
    /// The external reference is an overlay.
    pub xref_overlaid: bool,
    /// The base point of the block.
    pub base_point: [f64; 3],
    /// The path of the external reference (empty for an ordinary block).
    pub xref_path: TextValue,
    /// The block description.
    pub description: TextValue,
    /// The block control object that owns the entry.
    pub owner: Option<u64>,
    /// The entities the block owns, in drawing order (R2004 and later; an
    /// external reference owns none).
    pub owned_entities: Vec<u64>,
    /// The first entity of the block (R2000, which states a chain: the
    /// entities link to the next one), and
    pub first_entity: Option<u64>,
    /// the last.
    pub last_entity: Option<u64>,
}

/// Reads the block header at `entry`; `Ok(None)` when the object is of another
/// kind.
///
/// Fields: name, the external reference fields, four flag bits, a loaded bit,
/// (R2004 and later) the number of owned objects, the base point, the
/// external reference path, the list of insert counts, the description, the
/// preview data and (R2007 and later) the insertion units, the explodable bit
/// and the scaling byte. The main stream must end exactly there. The handles
/// after the owner (the block entity, the owned objects, the layout) are not
/// read.
pub fn read_block_header(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<BlockHeader>, ObjectError> {
    let Some(mut o) = open_entry(objects, entry, version, TYPE_BLOCK_HEADER)? else {
        return Ok(None);
    };
    skip_xref_fields(&mut o.r, version)?;
    let anonymous = o.r.bit()?;
    let has_attribute_definitions = o.r.bit()?;
    let is_xref = o.r.bit()?;
    let xref_overlaid = o.r.bit()?;
    o.r.bit()?; // loaded
                // An external reference owns no objects, and states no count (real files
                // end exactly where the fields do only without it).
    let owned_count = if version >= Version::R2004 && !is_xref && !xref_overlaid {
        o.r.bit_long()? // the number of objects the block owns
    } else {
        0
    };
    let base_point = [o.r.bit_double()?, o.r.bit_double()?, o.r.bit_double()?];
    let xref_path = text_value(&mut o.r, &mut o.strings)?;
    // Insert counts: bytes up to a zero.
    while o.r.raw_char()? != 0 {}
    let description = text_value(&mut o.r, &mut o.strings)?;
    let preview = o.r.bit_long()?;
    let preview = usize::try_from(preview)
        .ok()
        .filter(|&n| n <= o.r.remaining() / 8)
        .ok_or(ObjectError::Unexpected {
            what: "block preview size",
        })?;
    for _ in 0..preview {
        o.r.raw_char()?;
    }
    if version >= Version::R2007 {
        o.r.bit_short()?; // insertion units
        o.r.bit()?; // explodable
        o.r.raw_char()?; // scaling
    }
    o.check_main_end("block header main data length")?;
    let (mut handles, owner) = o.handles(entry.handle)?;
    handles.next()?; // a null handle
    handles.next()?; // the block entity
    let (mut owned_entities, mut first_entity, mut last_entity) = (Vec::new(), None, None);
    if version < Version::R2004 {
        if !is_xref && !xref_overlaid {
            first_entity = handles.next()?;
            last_entity = handles.next()?;
        }
    } else {
        let count = usize::try_from(owned_count)
            .ok()
            .filter(|&n| n <= handles.h.remaining() / 8)
            .ok_or(ObjectError::Unexpected {
                what: "owned object count",
            })?;
        for _ in 0..count {
            owned_entities.push(handles.next()?.ok_or(ObjectError::Unexpected {
                what: "owned object handle",
            })?);
        }
    }
    Ok(Some(BlockHeader {
        handle: entry.handle,
        name: o.name,
        anonymous,
        has_attribute_definitions,
        is_xref,
        xref_overlaid,
        base_point,
        xref_path,
        description,
        owner,
        owned_entities,
        first_entity,
        last_entity,
    }))
}

/// An image definition: the file a raster image shows and how big its pixels
/// are. Raster images name it by handle.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDefinition {
    /// The definition's handle.
    pub handle: u64,
    /// The class version.
    pub class_version: i32,
    /// The size of the picture in pixels.
    pub size: [f64; 2],
    /// The path of the picture file.
    pub file_path: TextValue,
    /// Whether the picture is loaded.
    pub loaded: bool,
    /// The resolution units: `0` none, `2` centimetres, `5` inches.
    pub resolution_unit: u8,
    /// The size of one pixel in drawing units.
    pub pixel_size: [f64; 2],
    /// The object that owns the definition.
    pub owner: Option<u64>,
}

/// Reads the image definition at `entry`, an object of the class named
/// `IMAGEDEF` in `classes`; `Ok(None)` when the object is of another kind.
pub fn read_image_definition(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
    classes: &[ClassDef],
) -> Result<Option<ImageDefinition>, ObjectError> {
    let is_definition = |code: u16| {
        code >= 500
            && classes
                .iter()
                .any(|c| i32::from(c.number) == i32::from(code) && c.dxf_name == "IMAGEDEF")
    };
    let Some(Prefix {
        mut r,
        layout,
        reactors,
        xdictionary_stored,
    }) = open_prefix(objects, entry, version, is_definition)?
    else {
        return Ok(None);
    };
    let mut strings = layout.strings.map(|(from, _)| {
        let mut s = r.clone();
        s.seek(from);
        s
    });
    let class_version = r.bit_long()?;
    let size = [r.raw_double()?, r.raw_double()?];
    let file_path = text_value(&mut r, &mut strings)?;
    let loaded = r.bit()?;
    let resolution_unit = r.raw_char()?;
    let pixel_size = [r.raw_double()?, r.raw_double()?];
    if r.position() != layout.main_end {
        return Err(ObjectError::Unexpected {
            what: "image definition main data length",
        });
    }
    let opened = Opened {
        r,
        layout,
        strings,
        reactors,
        xdictionary_stored,
        name: TextValue::Ansi(Vec::new()),
    };
    let (_, owner) = opened.handles(entry.handle)?;
    Ok(Some(ImageDefinition {
        handle: entry.handle,
        class_version,
        size,
        file_path,
        loaded,
        resolution_unit,
        pixel_size,
        owner,
    }))
}

/// A dimension style: the settings the dimensions that use it follow. Only
/// the settings that dimension values and their texts depend on are kept; the
/// rest are read and dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct DimStyle {
    /// The style's handle.
    pub handle: u64,
    /// The style name.
    pub name: TextValue,
    /// The prefix and suffix of the measurement text (`DIMPOST`).
    pub post: TextValue,
    /// The overall scale (`DIMSCALE`).
    pub scale: f64,
    /// The arrow size (`DIMASZ`).
    pub arrow_size: f64,
    /// The rounding of the measurement (`DIMRND`).
    pub rounding: f64,
    /// The upper tolerance (`DIMTP`).
    pub tolerance_upper: f64,
    /// The lower tolerance (`DIMTM`).
    pub tolerance_lower: f64,
    /// Tolerances are shown (`DIMTOL`).
    pub tolerances: bool,
    /// Limits are shown (`DIMLIM`).
    pub limits: bool,
    /// The zero suppression (`DIMZIN`).
    pub zero_suppression: i16,
    /// The text height (`DIMTXT`).
    pub text_height: f64,
    /// The factor measurements are multiplied by (`DIMLFAC`).
    pub length_factor: f64,
    /// The gap between the text and the dimension line (`DIMGAP`).
    pub text_gap: f64,
    /// The number of decimal places (`DIMDEC`).
    pub decimal_places: i16,
    /// The number of decimal places of the tolerances (`DIMTDEC`).
    pub tolerance_decimal_places: i16,
    /// The angular unit format (`DIMAUNIT`).
    pub angular_unit_format: i16,
    /// The angular decimal places (`DIMADEC`).
    pub angular_decimal_places: i16,
    /// The fraction format (`DIMFRAC`).
    pub fraction_format: i16,
    /// The linear unit format (`DIMLUNIT`).
    pub linear_unit_format: i16,
    /// The arc length symbol (`DIMARCSYM`, R2007 and later).
    pub arc_symbol: Option<i16>,
    /// The text style (`DIMTXSTY`).
    pub text_style: Option<u64>,
    /// The object that owns the style.
    pub owner: Option<u64>,
}

/// Reads the dimension style at `entry`; `Ok(None)` when the object is of
/// another kind.
///
/// The fields are those of the dimension variables in the order of the
/// specification; the main stream must end exactly where they do.
pub fn read_dim_style(
    objects: &[u8],
    entry: ObjectEntry,
    version: Version,
) -> Result<Option<DimStyle>, ObjectError> {
    let Some(mut o) = open_entry(objects, entry, version, TYPE_DIMSTYLE)? else {
        return Ok(None);
    };
    skip_xref_fields(&mut o.r, version)?;
    let post = text_value(&mut o.r, &mut o.strings)?;
    text_value(&mut o.r, &mut o.strings)?; // DIMAPOST
    let scale = o.r.bit_double()?;
    let arrow_size = o.r.bit_double()?;
    o.r.bit_double()?; // DIMEXO
    o.r.bit_double()?; // DIMDLI
    o.r.bit_double()?; // DIMEXE
    let rounding = o.r.bit_double()?;
    o.r.bit_double()?; // DIMDLE
    let tolerance_upper = o.r.bit_double()?;
    let tolerance_lower = o.r.bit_double()?;
    if version >= Version::R2007 {
        o.r.bit_double()?; // DIMFXL
        o.r.bit_double()?; // DIMJOGANG
        o.r.bit_short()?; // DIMTFILL
        read_table_color(&mut o.r, &mut o.strings, version)?; // DIMTFILLCLR
    }
    let tolerances = o.r.bit()?;
    let limits = o.r.bit()?;
    for _ in 0..4 {
        o.r.bit()?; // DIMTIH, DIMTOH, DIMSE1, DIMSE2
    }
    o.r.bit_short()?; // DIMTAD
    let zero_suppression = o.r.bit_short()?;
    o.r.bit_short()?; // DIMAZIN
    let arc_symbol = if version >= Version::R2007 {
        Some(o.r.bit_short()?)
    } else {
        None
    };
    let text_height = o.r.bit_double()?;
    o.r.bit_double()?; // DIMCEN
    o.r.bit_double()?; // DIMTSZ
    o.r.bit_double()?; // DIMALTF
    let length_factor = o.r.bit_double()?;
    o.r.bit_double()?; // DIMTVP
    o.r.bit_double()?; // DIMTFAC
    let text_gap = o.r.bit_double()?;
    o.r.bit_double()?; // DIMALTRND
    o.r.bit()?; // DIMALT
    o.r.bit_short()?; // DIMALTD
    for _ in 0..4 {
        o.r.bit()?; // DIMTOFL, DIMSAH, DIMTIX, DIMSOXD
    }
    for _ in 0..3 {
        read_table_color(&mut o.r, &mut o.strings, version)?; // DIMCLRD, DIMCLRE, DIMCLRT
    }
    let angular_decimal_places = o.r.bit_short()?;
    let decimal_places = o.r.bit_short()?;
    let tolerance_decimal_places = o.r.bit_short()?;
    o.r.bit_short()?; // DIMALTU
    o.r.bit_short()?; // DIMALTTD
    let angular_unit_format = o.r.bit_short()?;
    let fraction_format = o.r.bit_short()?;
    let linear_unit_format = o.r.bit_short()?;
    o.r.bit_short()?; // DIMDSEP
    o.r.bit_short()?; // DIMTMOVE
    o.r.bit_short()?; // DIMJUST
    o.r.bit()?; // DIMSD1
    o.r.bit()?; // DIMSD2
    o.r.bit_short()?; // DIMTOLJ
    o.r.bit_short()?; // DIMTZIN
    o.r.bit_short()?; // DIMALTZ
    o.r.bit_short()?; // DIMALTTZ
    o.r.bit()?; // DIMUPT
    o.r.bit_short()?; // DIMATFIT
    if version >= Version::R2007 {
        o.r.bit()?; // DIMFXLON
    }
    if version >= Version::R2010 {
        o.r.bit()?; // DIMTXTDIRECTION
        o.r.bit_double()?; // DIMALTMZF
        text_value(&mut o.r, &mut o.strings)?; // DIMALTMZS
        o.r.bit_double()?; // DIMMZF
        text_value(&mut o.r, &mut o.strings)?; // DIMMZS
    }
    o.r.bit_short()?; // DIMLWD
    o.r.bit_short()?; // DIMLWE
    o.r.bit()?; // the 0-bit of the DXF flags
    o.check_main_end("dimension style main data length")?;
    let (mut h, owner) = o.handles(entry.handle)?;
    h.next()?; // the external reference block
    let text_style = h.next()?;
    Ok(Some(DimStyle {
        handle: entry.handle,
        name: o.name,
        post,
        scale,
        arrow_size,
        rounding,
        tolerance_upper,
        tolerance_lower,
        tolerances,
        limits,
        zero_suppression,
        text_height,
        length_factor,
        text_gap,
        decimal_places,
        tolerance_decimal_places,
        angular_unit_format,
        angular_decimal_places,
        fraction_format,
        linear_unit_format,
        arc_symbol,
        text_style,
        owner,
    }))
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
                index: 0,
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
