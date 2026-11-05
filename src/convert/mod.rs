//! Reading a whole drawing into the [`uncad_model`] entity model.
//!
//! [`read_database`] opens the file, reads its tables, header variables and
//! entities, and returns a [`CadDatabase`]. The model's own rule holds:
//! nothing read is silently dropped. What this reader does not convert yet, an
//! object it cannot decode and a text it cannot turn into characters are
//! each named in the drawing's [`ReadDiagnostics`], with how many there were.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use uncad_model::color::ACI_PALETTE;
use uncad_model::model::{AttribEntity, EntityId, Point3D, Ref};
use uncad_model::tables::{
    AngularUnitFormat, ArcSymbol, BlockRecord, DimStyleRecord, ExternalReference, FractionFormat,
    ImageDefinition, LayerRecord, LayoutRecord, LinearUnitFormat, PlotPaperUnits, PlotRotation,
    PlotSettings, ResolutionUnit, Tables,
};
use uncad_model::text::decode_escapes;
use uncad_model::{CadDatabase, Entity, HeaderVariables as ModelHeader, ReadDiagnostics};

use crate::bits::TextValue;
use crate::codepage::CodePage;
use crate::drawing::{Drawing, DrawingError};
use crate::entities::{self, read_entity_with_classes, Geometry};
use crate::objects::ObjectEntry;
use crate::tables::{
    read_appid_name, read_block_control, read_block_header, read_dim_style, read_dimstyle_name,
    read_image_definition, read_layer, read_layout, read_linetype_name, read_mleader_style,
    read_mline_style, read_mlinestyle_name, read_style_name, read_table_style, BlockHeader, Layer,
};
use crate::variables::HeaderVariables;
use crate::version::Version;

mod entity;
mod extended;

use entity::{
    attribute_flags, chain, convert_common, convert_entity, convert_text, point2, Context,
};

/// Why a drawing could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DatabaseError {
    /// The file could not be opened.
    Drawing(DrawingError),
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatabaseError::Drawing(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DatabaseError {}

impl From<DrawingError> for DatabaseError {
    fn from(e: DrawingError) -> Self {
        DatabaseError::Drawing(e)
    }
}

/// The line weights a file's codes stand for, in hundredths of a millimetre.
/// Codes 29 to 31 are by layer, by block and the default; a code the table
/// does not name states no weight.
const LINEWEIGHTS: [i16; 24] = [
    0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158, 200,
    211,
];

fn lineweight(code: u8) -> Option<i16> {
    match code {
        29 => Some(-1),
        30 => Some(-2),
        31 => Some(-3),
        c => LINEWEIGHTS.get(usize::from(c)).copied(),
    }
}

/// Counts of the things this reader reported instead of converting, keyed by
/// the words the report uses, so the same read always reports the same way.
#[derive(Default)]
struct Notes(BTreeMap<String, usize>);

impl Notes {
    fn add(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_default() += 1;
    }

    fn into_diagnostics(self) -> ReadDiagnostics {
        ReadDiagnostics {
            warnings: self
                .0
                .into_iter()
                .map(|(what, n)| format!("{what}: {n}"))
                .collect(),
        }
    }
}

/// A text as characters. Text past ASCII in a code page is turned into characters only through a page that is
/// known (see [`CodePage`]); with an unknown page it is not guessed.
fn characters(t: &TextValue, page: CodePage) -> Option<String> {
    let stored = match t {
        TextValue::Unicode(s) => s.clone(),
        TextValue::Ansi(b) if b.is_ascii() => String::from_utf8_lossy(b).into_owned(),
        // Past ASCII the bytes mean what the drawing's code page says -- and nothing when the page is not known.
        TextValue::Ansi(b) => page.decode(b)?,
    };
    // A file stores the characters its code page lacks as `\U+XXXX`; the model's
    // strings do not depend on how a file stored them. (`\M+` names a character
    // of an Asian code page, which this reader cannot decode: it stays as written.)
    Some(decode_escapes(&stored, |_, _| None).into_owned())
}

fn hex(handle: u64) -> String {
    format!("{handle:X}")
}

fn point(p: [f64; 3]) -> Point3D {
    Point3D {
        x: p[0],
        y: p[1],
        z: p[2],
    }
}

/// What a drawing states about itself that the entity model leaves out.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingFacts {
    /// The version the file is.
    pub version: Version,
    /// The code page number the file header declares.
    pub code_page: u16,
    /// The header variables.
    pub variables: HeaderVariables,
    /// `MEASUREMENT` (0 English, 1 metric), kept in the file's template; `None`
    /// when the file has none.
    pub measurement: Option<u16>,
    /// The name of the layer the header calls current; `None` when the header
    /// names no layer the layer table has, or the name is a text this reader
    /// cannot turn into characters.
    pub current_layer: Option<String>,
}

/// Reads the drawing in `bytes` into the entity model.
pub fn read_database(bytes: &[u8]) -> Result<CadDatabase, DatabaseError> {
    read_database_with_facts(bytes).map(|(database, _)| database)
}

/// Reads the drawing in `bytes` into the entity model, and returns beside it
/// what the file states about itself ([`DrawingFacts`]) from the same read.
pub fn read_database_with_facts(
    bytes: &[u8],
) -> Result<(CadDatabase, DrawingFacts), DatabaseError> {
    let drawing = Drawing::open(bytes)?;
    let version = drawing.version();
    let page = CodePage::of(drawing.code_page());
    let objects = drawing.objects();
    let mut notes = Notes::default();

    // The entries whose position lies inside the object data; the rest are
    // reported, not skipped quietly.
    let mut entries: Vec<ObjectEntry> = Vec::new();
    for e in drawing.entries() {
        if e.offset < objects.len() as u64 {
            entries.push(*e);
        } else {
            notes.add("object outside the object data");
        }
    }

    let ObjectTables {
        layers,
        linetypes,
        styles,
        dim_styles,
        acad,
        mline_style_rows,
        layout_rows,
        dim_style_rows,
        image_defs,
        mline_styles,
        mleader_styles,
        table_styles,
        headers,
        table_order,
    } = read_object_tables(&drawing, &entries, &mut notes);

    // Block names, unique the way a table needs.
    let block_names = block_names_of(
        &headers,
        &table_order,
        drawing.variables().paper_space_block,
        page,
        &mut notes,
    );
    let model_space = block_names.get(&drawing.variables().model_space_block);
    let paper_space = block_names.get(&drawing.variables().paper_space_block);

    let (layer_names, layer_records) = layer_records_of(&layers, &linetypes, page, &mut notes);

    // Every entity the file keeps, read once.
    let by_handle: BTreeMap<u64, ObjectEntry> = entries.iter().map(|e| (e.handle, *e)).collect();
    let read = read_entities(&drawing, &entries, &mut notes);
    // The vertices of polylines, which the polyline that owns them takes.
    let (vertices_2d, vertices_3d, faces) = polyline_parts(&read);
    let ctx = Context {
        version,
        page,
        objects,
        by_handle: &by_handle,
        layers: &layer_names,
        linetypes: &linetypes,
        styles: &styles,
        dim_styles: &dim_styles,
        mline_styles: &mline_styles,
        mleader_styles: &mleader_styles,
        table_styles: &table_styles,
        acad,
        block_names: &block_names,
        vertices_2d: &vertices_2d,
        vertices_3d: &vertices_3d,
        faces: &faces,
    };

    let viewport_handles: BTreeSet<u64> = read
        .iter()
        .filter(|e| matches!(e.geometry, Geometry::Viewport(_)))
        .map(|e| e.common.handle)
        .collect();

    // The attributes first: an insertion takes the ones it lists.
    let attribs = attribute_entities(&read, &ctx, &mut notes);

    // The entities, converted, with the block each one says it belongs to.
    let converted = convert_all(
        &read,
        &ctx,
        &attribs,
        &block_names,
        model_space,
        paper_space,
        &mut notes,
    );

    // The entities of each block, in the order the block draws them: the
    // list the block header keeps (R2004 and later), or the chain from its
    // first entity to its last (R2000).
    let mut by_block = entities_by_block(&headers, &block_names, converted, &ctx, &mut notes);

    // What model space and the paper spaces own is also listed at the top.
    let top_level = top_level_entities(&headers, &block_names, &by_block, model_space, paper_space);

    // The block records, by name.
    let block_records = block_records_of(&headers, &block_names, &mut by_block, page, &mut notes);

    // The multiline styles: each one's line offsets, in the order the style keeps them.
    let mline_style_records = mline_style_records_of(&mline_style_rows, page, &mut notes);

    // The layouts, by name.
    let layout_records = layout_records_of(
        &layout_rows,
        page,
        &block_names,
        &viewport_handles,
        &by_handle,
        &mut notes,
    );

    // The dimension styles, by name.
    let dim_style_records = dim_style_records_of(&dim_style_rows, page, &mut notes);

    // The image definitions, by handle (the key an image names its picture by).
    let image_definitions = image_definitions_of(&image_defs, page, &mut notes);

    let vars = drawing.variables();
    let header = ModelHeader {
        insunits: Some(vars.insertion_units as u16),
        fingerprintguid: characters(&vars.fingerprint_guid, page),
        versionguid: characters(&vars.version_guid, page),
        pdmode: Some(vars.point_mode),
        pdsize: Some(vars.point_size),
    };

    let current_layer = layers
        .get(&vars.current_layer)
        .and_then(|layer| characters(&layer.name, page));
    let facts = DrawingFacts {
        version,
        code_page: drawing.code_page(),
        variables: vars.clone(),
        measurement: drawing.measurement(),
        current_layer,
    };
    let database = CadDatabase {
        entities: top_level,
        tables: Tables {
            layers: layer_records,
            block_records,
            image_definitions,
            dim_styles: dim_style_records,
            mlinestyles: mline_style_records,
            layouts: layout_records,
        },
        header,
        read_diagnostics: notes.into_diagnostics(),
    };
    Ok((database, facts))
}

/// What the table objects of a file say, by handle, and the block headers in
/// the order the file lists them.
struct ObjectTables {
    layers: BTreeMap<u64, Layer>,
    linetypes: BTreeMap<u64, TextValue>,
    styles: BTreeMap<u64, TextValue>,
    dim_styles: BTreeMap<u64, TextValue>,
    acad: Option<u64>,
    mline_style_rows: Vec<crate::tables::MlineStyle>,
    layout_rows: Vec<crate::tables::PlotLayout>,
    dim_style_rows: Vec<crate::tables::DimStyle>,
    image_defs: BTreeMap<u64, crate::tables::ImageDefinition>,
    mline_styles: BTreeMap<u64, TextValue>,
    mleader_styles: BTreeMap<u64, i16>,
    table_styles: BTreeMap<u64, i16>,
    headers: Vec<BlockHeader>,
    table_order: Vec<Option<u64>>,
}

/// Reads every table object the entities refer to by handle; what cannot be
/// read is reported, not skipped quietly.
fn read_object_tables(
    drawing: &Drawing<'_>,
    entries: &[ObjectEntry],
    notes: &mut Notes,
) -> ObjectTables {
    let version = drawing.version();
    let page = CodePage::of(drawing.code_page());
    let objects = drawing.objects();
    let mut layers: BTreeMap<u64, Layer> = BTreeMap::new();
    let mut linetypes: BTreeMap<u64, TextValue> = BTreeMap::new();
    let mut styles: BTreeMap<u64, TextValue> = BTreeMap::new();
    let mut dim_styles: BTreeMap<u64, TextValue> = BTreeMap::new();
    let mut acad: Option<u64> = None;
    let mut mline_style_rows: Vec<crate::tables::MlineStyle> = Vec::new();
    let mut layout_rows: Vec<crate::tables::PlotLayout> = Vec::new();
    let mut dim_style_rows: Vec<crate::tables::DimStyle> = Vec::new();
    let mut image_defs: BTreeMap<u64, crate::tables::ImageDefinition> = BTreeMap::new();
    let mut mline_styles: BTreeMap<u64, TextValue> = BTreeMap::new();
    let mut mleader_styles: BTreeMap<u64, i16> = BTreeMap::new();
    let mut table_styles: BTreeMap<u64, i16> = BTreeMap::new();
    let mut headers: Vec<BlockHeader> = Vec::new();
    let mut table_order: Vec<Option<u64>> = Vec::new();
    for e in entries {
        match read_layer(objects, *e, version) {
            Ok(Some(layer)) => {
                layers.insert(e.handle, layer);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("layer that could not be read");
                continue;
            }
        }
        match read_linetype_name(objects, *e, version) {
            Ok(Some(name)) => {
                linetypes.insert(e.handle, name);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("linetype that could not be read");
                continue;
            }
        }
        match read_style_name(objects, *e, version) {
            Ok(Some(name)) => {
                styles.insert(e.handle, name);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("text style that could not be read");
                continue;
            }
        }
        match read_dim_style(objects, *e, version) {
            Ok(Some(style)) => {
                dim_styles.insert(e.handle, style.name.clone());
                dim_style_rows.push(style);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("dimension style that could not be read");
                continue;
            }
        }
        match read_dimstyle_name(objects, *e, version) {
            Ok(Some(name)) => {
                dim_styles.insert(e.handle, name);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("dimension style that could not be read");
                continue;
            }
        }
        match read_mline_style(objects, *e, version) {
            Ok(Some(style)) => {
                mline_styles.insert(e.handle, style.name.clone());
                mline_style_rows.push(style);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("multiline style that could not be read");
                continue;
            }
        }
        match read_layout(objects, *e, version, drawing.classes()) {
            Ok(Some(layout)) => {
                layout_rows.push(layout);
                continue;
            }
            Ok(None) => {}
            Err(e) => {
                notes.add(format!("layout that could not be read ({e})"));
                continue;
            }
        }
        match read_mlinestyle_name(objects, *e, version) {
            Ok(Some(name)) => {
                mline_styles.insert(e.handle, name);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("multiline style that could not be read");
                continue;
            }
        }
        match read_mleader_style(objects, *e, version, drawing.classes()) {
            Ok(Some(style)) => {
                mleader_styles.insert(e.handle, style.leader_type);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("multileader style that could not be read");
                continue;
            }
        }
        match read_table_style(objects, *e, version, drawing.classes()) {
            Ok(Some(style)) => {
                if let Some(flow) = style.flow {
                    table_styles.insert(e.handle, flow);
                }
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("table style that could not be read");
                continue;
            }
        }
        match read_image_definition(objects, *e, version, drawing.classes()) {
            Ok(Some(def)) => {
                image_defs.insert(e.handle, def);
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("image definition that could not be read");
                continue;
            }
        }
        match read_appid_name(objects, *e, version) {
            Ok(Some(name)) => {
                if matches!(characters(&name, page).as_deref(), Some(n) if n.eq_ignore_ascii_case("ACAD"))
                {
                    acad = Some(e.handle);
                }
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("application that could not be read");
                continue;
            }
        }
        match read_block_control(objects, *e, version) {
            Ok(Some(order)) => {
                table_order = order;
                continue;
            }
            Ok(None) => {}
            Err(_) => {
                notes.add("block table that could not be read");
                continue;
            }
        }
        match read_block_header(objects, *e, version) {
            Ok(Some(block)) => headers.push(block),
            Ok(None) => {}
            Err(_) => notes.add("block header that could not be read"),
        }
    }
    ObjectTables {
        layers,
        linetypes,
        styles,
        dim_styles,
        acad,
        mline_style_rows,
        layout_rows,
        dim_style_rows,
        image_defs,
        mline_styles,
        mleader_styles,
        table_styles,
        headers,
        table_order,
    }
}

/// Block names, made unique the way a table needs. An anonymous block
/// (`*U`, `*D`, ...) is numbered by its place in the block table. A name the
/// file gives to several blocks (the paper space of each layout) belongs bare
/// to the block the header names as the current paper space and, for the
/// others, gets its place in the table as a number.
fn block_names_of(
    headers: &[BlockHeader],
    table_order: &[Option<u64>],
    current_paper: u64,
    page: CodePage,
    notes: &mut Notes,
) -> BTreeMap<u64, String> {
    let mut block_names: BTreeMap<u64, String> = BTreeMap::new();
    let position: BTreeMap<u64, usize> = table_order
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.map(|h| (h, i)))
        .collect();
    let mut stored_names: BTreeMap<u64, String> = BTreeMap::new();
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for block in headers {
        let Some(stored) = characters(&block.name, page) else {
            notes.add("block name in a code page that is not read");
            continue;
        };
        *uses.entry(stored.clone()).or_default() += 1;
        stored_names.insert(block.handle, stored);
    }
    let mut bare_taken: BTreeSet<String> = BTreeSet::new();
    for (index, block) in headers.iter().enumerate() {
        let Some(stored) = stored_names.get(&block.handle) else {
            continue;
        };
        let index = position.get(&block.handle).copied().unwrap_or(index);
        let shared = uses.get(stored).copied().unwrap_or(0) > 1;
        let name = if block.anonymous || (shared && block.handle != current_paper) {
            format!("{stored}{index}")
        } else if bare_taken.insert(stored.clone()) {
            stored.clone()
        } else {
            format!("{stored}{index}")
        };
        block_names.insert(block.handle, name);
    }
    block_names
}

/// The layers by handle and, by name, the records the model keeps.
fn layer_records_of(
    layers: &BTreeMap<u64, Layer>,
    linetypes: &BTreeMap<u64, TextValue>,
    page: CodePage,
    notes: &mut Notes,
) -> (BTreeMap<u64, String>, BTreeMap<String, LayerRecord>) {
    let mut layer_names: BTreeMap<u64, String> = BTreeMap::new();
    let mut layer_records: BTreeMap<String, LayerRecord> = BTreeMap::new();
    for (handle, layer) in layers {
        let Some(name) = characters(&layer.name, page) else {
            notes.add("layer name in a code page that is not read");
            continue;
        };
        layer_names.insert(*handle, name.clone());
        // A layer states its colour number in the packed colour. A true colour
        // is shown as the number of the palette colour that has exactly its
        // value, and as 256 (no number) when there is none.
        let color_index = match layer.color.method {
            0xC3 => layer.color.value as i16,
            0xC2 => ACI_PALETTE[1..256]
                .iter()
                .position(|&rgb| rgb == layer.color.value)
                .map_or(256, |i| i as i16 + 1),
            _ => layer.color.index as i16,
        };
        let linetype = match layer.linetype {
            None => Ref::Absent,
            Some(h) => match linetypes.get(&h).and_then(|n| characters(n, page)) {
                Some(n) => Ref::Resolved(n),
                None => Ref::Unresolved(hex(h)),
            },
        };
        layer_records.insert(
            name.clone(),
            LayerRecord {
                name,
                color_index,
                off: layer.flags & 2 != 0,
                frozen: layer.flags & 1 != 0,
                locked: layer.flags & 8 != 0,
                plot: Some(layer.flags & 16 != 0),
                lineweight: lineweight(layer.lineweight()),
                linetype,
            },
        );
    }
    (layer_names, layer_records)
}

/// Every entity the file keeps, read once; what cannot be read is reported.
fn read_entities(
    drawing: &Drawing<'_>,
    entries: &[ObjectEntry],
    notes: &mut Notes,
) -> Vec<entities::Entity> {
    let version = drawing.version();
    let objects = drawing.objects();
    let mut read: Vec<entities::Entity> = Vec::new();
    for e in entries {
        match read_entity_with_classes(objects, *e, version, drawing.classes()) {
            Ok(Some(entity)) => read.push(entity),
            Ok(None) => {}
            Err(e) => notes.add(format!("entity that could not be read ({e})")),
        }
    }
    read
}

type PolylineParts = (
    BTreeMap<u64, entities::Vertex2d>,
    BTreeMap<u64, entities::Vertex3d>,
    BTreeMap<u64, entities::VertexFace>,
);

/// The vertices and faces of polylines, by handle, which the polyline that
/// owns them takes.
fn polyline_parts(read: &[entities::Entity]) -> PolylineParts {
    let mut vertices_2d: BTreeMap<u64, entities::Vertex2d> = BTreeMap::new();
    let mut vertices_3d: BTreeMap<u64, entities::Vertex3d> = BTreeMap::new();
    let mut faces: BTreeMap<u64, entities::VertexFace> = BTreeMap::new();
    for entity in read {
        match &entity.geometry {
            Geometry::Vertex2d(v) => {
                vertices_2d.insert(entity.common.handle, v.clone());
            }
            Geometry::Vertex3d(v) => {
                vertices_3d.insert(entity.common.handle, v.clone());
            }
            Geometry::VertexFace(f) => {
                faces.insert(entity.common.handle, *f);
            }
            _ => {}
        }
    }
    (vertices_2d, vertices_3d, faces)
}

/// The attributes of the file by handle, converted first: an insertion takes
/// the ones it lists.
fn attribute_entities(
    read: &[entities::Entity],
    ctx: &Context<'_>,
    notes: &mut Notes,
) -> BTreeMap<u64, AttribEntity> {
    let page = ctx.page;
    let mut attribs: BTreeMap<u64, AttribEntity> = BTreeMap::new();
    for entity in read {
        if let Geometry::Attrib(a) = &entity.geometry {
            if let Some(text) = convert_text(&a.text, ctx, notes) {
                attribs.insert(
                    entity.common.handle,
                    AttribEntity {
                        common: convert_common(entity, ctx),
                        start_point: text.start_point,
                        text_height: text.height,
                        tag: String::new(),
                        flags: attribute_flags(a.flags),
                        text: text.value,
                        rotation: text.rotation,
                        horizontal_justification: text.horizontal,
                        vertical_justification: text.vertical,
                        alignment_point: text.alignment,
                        width_factor: text.width_factor,
                        oblique_angle: text.oblique,
                        style_name: text.style,
                        elevation: text.elevation,
                        extrusion: text.extrusion,
                    },
                );
                if let Some(tag) = characters(&a.tag, page) {
                    if let Some(x) = attribs.get_mut(&entity.common.handle) {
                        x.tag = tag;
                    }
                } else {
                    attribs.remove(&entity.common.handle);
                    notes.add("attribute tag in a code page that is not read");
                }
            }
        }
    }
    attribs
}

/// Every entity converted, by handle, with the block each one says it
/// belongs to.
fn convert_all(
    read: &[entities::Entity],
    ctx: &Context<'_>,
    attribs: &BTreeMap<u64, AttribEntity>,
    block_names: &BTreeMap<u64, String>,
    model_space: Option<&String>,
    paper_space: Option<&String>,
    notes: &mut Notes,
) -> BTreeMap<u64, (String, Entity)> {
    let mut converted: BTreeMap<u64, (String, Entity)> = BTreeMap::new();
    for entity in read {
        let owner = match entity.references.owner.and_then(|h| block_names.get(&h)) {
            Some(name) => Some(name),
            None => match entity.common.entity_mode {
                2 => model_space,
                1 => paper_space,
                _ => None,
            },
        };
        let Some(owner) = owner else {
            // A vertex or an attribute belongs to the entity that owns it, not to a
            // block; it is read with that entity. A kind this reader does not read
            // and no block holds is an object the class table calls an entity (or one
            // an entity owns), not an entity of the drawing.
            if !matches!(
                entity.geometry,
                Geometry::Vertex2d(_)
                    | Geometry::Vertex3d(_)
                    | Geometry::VertexFace(_)
                    | Geometry::Attrib(_)
                    | Geometry::Unread(_)
            ) {
                let kind = format!("{:?}", entity.geometry);
                let kind = kind.split(['(', ' ', '{']).next().unwrap_or("").to_string();
                notes.add(format!("{kind} entity whose block is not known"));
            }
            continue;
        };
        let Some(converted_entity) = convert_entity(entity, ctx, attribs, notes) else {
            let kind = format!("{:?}", entity.geometry);
            let kind = kind.split(['(', ' ', '{']).next().unwrap_or("").to_string();
            notes.add(format!("{kind} entity not converted"));
            continue;
        };
        converted.insert(entity.common.handle, (owner.clone(), converted_entity));
    }
    converted
}

/// The entities of each block, in the order the block draws them: the list
/// the block header keeps (R2004 and later), or the chain from its first
/// entity to its last (R2000). An entity no block lists is kept in the block
/// it names, after the listed ones, and reported.
fn entities_by_block(
    headers: &[BlockHeader],
    block_names: &BTreeMap<u64, String>,
    mut converted: BTreeMap<u64, (String, Entity)>,
    ctx: &Context<'_>,
    notes: &mut Notes,
) -> BTreeMap<String, Vec<Entity>> {
    let version = ctx.version;
    let mut by_block: BTreeMap<String, Vec<Entity>> = BTreeMap::new();
    for block in headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        let order = if version >= Version::R2004 {
            block.owned_entities.clone()
        } else {
            match (block.first_entity, block.last_entity) {
                (Some(first), Some(last)) => chain(ctx, first, last, notes),
                _ => Vec::new(),
            }
        };
        for handle in order {
            if let Some((_, entity)) = converted.remove(&handle) {
                by_block.entry(name.clone()).or_default().push(entity);
            }
        }
    }
    // An entity no block lists is kept in the block it names, after the
    // listed ones, and reported.
    for (_, (owner, entity)) in std::mem::take(&mut converted) {
        notes.add("entity that its block does not list");
        by_block.entry(owner).or_default().push(entity);
    }
    by_block
}

/// What model space and the paper spaces own, which is also listed at the
/// top.
fn top_level_entities(
    headers: &[BlockHeader],
    block_names: &BTreeMap<u64, String>,
    by_block: &BTreeMap<String, Vec<Entity>>,
    model_space: Option<&String>,
    paper_space: Option<&String>,
) -> Vec<Entity> {
    let mut top_level: Vec<Entity> = Vec::new();
    for block in headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        if Some(name) == model_space
            || Some(name) == paper_space
            || name.starts_with("*Paper_Space")
        {
            for entity in by_block.get(name).into_iter().flatten() {
                top_level.push(entity.clone());
                // An insertion's attributes are listed at the top as well, right after it.
                if let Entity::Insert(insert) = entity {
                    top_level.extend(insert.attribs.iter().cloned().map(Entity::Attrib));
                }
            }
        }
    }
    top_level
}

/// The block records by name, each with the entities it draws.
fn block_records_of(
    headers: &[BlockHeader],
    block_names: &BTreeMap<u64, String>,
    by_block: &mut BTreeMap<String, Vec<Entity>>,
    page: CodePage,
    notes: &mut Notes,
) -> BTreeMap<String, BlockRecord> {
    let mut block_records: BTreeMap<String, BlockRecord> = BTreeMap::new();
    for block in headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        let external_reference = if block.is_xref {
            match characters(&block.xref_path, page) {
                Some(path) => Some(ExternalReference {
                    path,
                    overlay: block.xref_overlaid,
                }),
                None => {
                    notes.add("external reference path in a code page that is not read");
                    None
                }
            }
        } else {
            None
        };
        block_records.insert(
            name.clone(),
            BlockRecord {
                name: name.clone(),
                entities: by_block.remove(name).unwrap_or_default(),
                base_point: point(block.base_point),
                external_reference,
            },
        );
    }
    block_records
}

/// The multiline styles: each one's line offsets, in the order the style
/// keeps them.
fn mline_style_records_of(
    rows: &[crate::tables::MlineStyle],
    page: CodePage,
    notes: &mut Notes,
) -> BTreeMap<String, Vec<f64>> {
    let mut mline_style_records: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for m in rows {
        match characters(&m.name, page) {
            Some(name) => {
                mline_style_records.insert(name, m.elements.iter().map(|e| e.offset).collect());
            }
            None => notes.add("multiline style name in a code page that is not read"),
        }
    }
    mline_style_records
}

/// The layouts by name.
fn layout_records_of(
    rows: &[crate::tables::PlotLayout],
    page: CodePage,
    block_names: &BTreeMap<u64, String>,
    viewport_handles: &BTreeSet<u64>,
    by_handle: &BTreeMap<u64, ObjectEntry>,
    notes: &mut Notes,
) -> BTreeMap<String, LayoutRecord> {
    let mut layout_records: BTreeMap<String, LayoutRecord> = BTreeMap::new();
    for l in rows {
        let (Some(name), Some(paper_name)) =
            (characters(&l.name, page), characters(&l.paper_size, page))
        else {
            notes.add("layout in a code page that is not read");
            continue;
        };
        layout_records.insert(
            name.clone(),
            LayoutRecord {
                name,
                tab_order: l.tab_order,
                block_name: match l.block {
                    None => Ref::Absent,
                    Some(h) => match block_names.get(&h) {
                        Some(n) => Ref::Resolved(n.clone()),
                        None => Ref::Unresolved(hex(h)),
                    },
                },
                limits_min: point2(l.limits_min),
                limits_max: point2(l.limits_max),
                plot_settings: PlotSettings {
                    paper_name,
                    paper_width: l.paper_width,
                    paper_height: l.paper_height,
                    margin_left: l.margins[0],
                    margin_bottom: l.margins[1],
                    margin_right: l.margins[2],
                    margin_top: l.margins[3],
                    plot_origin: point2(l.plot_origin),
                    paper_units: match l.paper_units {
                        0 => Some(PlotPaperUnits::Inches),
                        1 => Some(PlotPaperUnits::Millimeters),
                        2 => Some(PlotPaperUnits::Pixels),
                        _ => None,
                    },
                    rotation: match l.plot_rotation {
                        0 => Some(PlotRotation::Unrotated),
                        1 => Some(PlotRotation::Counterclockwise90),
                        2 => Some(PlotRotation::UpsideDown),
                        3 => Some(PlotRotation::Clockwise90),
                        _ => None,
                    },
                    scale_numerator: l.scale_numerator,
                    scale_denominator: l.scale_denominator,
                },
                paper_space_linetype_scaling: l.flags & 1 != 0,
                limits_check: l.flags & 2 != 0,
                extents_min: Some(point(l.extents_min)),
                extents_max: Some(point(l.extents_max)),
                // A sheet names the VIEWPORT entity that was active last. A model
                // layout names a VPORT table record there, which is not an entity.
                active_viewport: match l.last_active_viewport {
                    None => Ref::Absent,
                    Some(h) if viewport_handles.contains(&h) => Ref::Resolved(EntityId::new(h)),
                    Some(h) if by_handle.contains_key(&h) => Ref::Absent,
                    Some(h) => Ref::Unresolved(hex(h)),
                },
            },
        );
    }
    layout_records
}

/// The dimension styles by name.
fn dim_style_records_of(
    rows: &[crate::tables::DimStyle],
    page: CodePage,
    notes: &mut Notes,
) -> BTreeMap<String, DimStyleRecord> {
    let mut dim_style_records: BTreeMap<String, DimStyleRecord> = BTreeMap::new();
    for d in rows {
        let (Some(name), Some(post)) = (characters(&d.name, page), characters(&d.post, page))
        else {
            notes.add("dimension style in a code page that is not read");
            continue;
        };
        dim_style_records.insert(
            name.clone(),
            DimStyleRecord {
                name,
                post: Some(post),
                scale: Some(d.scale),
                length_factor: Some(d.length_factor),
                tolerances: Some(d.tolerances),
                limits: Some(d.limits),
                tolerance_upper: Some(d.tolerance_upper),
                tolerance_lower: Some(d.tolerance_lower),
                decimal_places: Some(i32::from(d.decimal_places)),
                tolerance_decimal_places: Some(i32::from(d.tolerance_decimal_places)),
                text_height: Some(d.text_height),
                arrow_size: Some(d.arrow_size),
                linear_unit_format: LinearUnitFormat::from_code(i32::from(d.linear_unit_format)),
                zero_suppression: Some(i32::from(d.zero_suppression)),
                rounding: Some(d.rounding),
                angular_unit_format: AngularUnitFormat::from_code(i32::from(d.angular_unit_format)),
                angular_decimal_places: Some(i32::from(d.angular_decimal_places)),
                fraction_format: FractionFormat::from_code(i32::from(d.fraction_format)),
                arc_symbol: d
                    .arc_symbol
                    .and_then(|c| ArcSymbol::from_code(i32::from(c))),
                text_gap: Some(d.text_gap),
            },
        );
    }
    dim_style_records
}

/// The image definitions by handle (the key an image names its picture by).
fn image_definitions_of(
    defs: &BTreeMap<u64, crate::tables::ImageDefinition>,
    page: CodePage,
    notes: &mut Notes,
) -> BTreeMap<String, ImageDefinition> {
    let mut image_definitions: BTreeMap<String, ImageDefinition> = BTreeMap::new();
    for (handle, def) in defs {
        let file_path = match characters(&def.file_path, page) {
            Some(p) => Some(p),
            None => {
                notes.add("image path in a code page that is not read");
                continue;
            }
        };
        image_definitions.insert(
            hex(*handle),
            ImageDefinition {
                file_path,
                size_pixels: point2(def.size),
                pixel_size: point2(def.pixel_size),
                loaded: Some(def.loaded),
                resolution_unit: ResolutionUnit::from_code(i32::from(def.resolution_unit)),
            },
        );
    }
    image_definitions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_weight_codes_map_to_hundredths_of_a_millimetre() {
        assert_eq!(lineweight(0), Some(0));
        assert_eq!(lineweight(1), Some(5));
        assert_eq!(lineweight(23), Some(211));
        // By layer, by block, the default.
        assert_eq!(lineweight(29), Some(-1));
        assert_eq!(lineweight(30), Some(-2));
        assert_eq!(lineweight(31), Some(-3));
        // Codes the table does not name state no weight.
        assert_eq!(lineweight(24), None);
        assert_eq!(lineweight(28), None);
        assert_eq!(lineweight(200), None);
    }

    #[test]
    fn a_text_in_a_code_page_is_turned_into_characters_only_through_a_known_page() {
        let unknown = CodePage::UNKNOWN;
        assert_eq!(
            characters(&TextValue::Unicode("Layer".into()), unknown).as_deref(),
            Some("Layer")
        );
        assert_eq!(
            characters(&TextValue::Ansi(b"Layer".to_vec()), unknown).as_deref(),
            Some("Layer")
        );
        // A byte above 0x7F means something only in the drawing's code page: with no known page it is not guessed ...
        assert_eq!(
            characters(&TextValue::Ansi(vec![b'A', 0xB0]), unknown),
            None
        );
        assert_eq!(
            characters(&TextValue::Ansi(vec![b'A', 0xB0]), CodePage::of(39)),
            None
        );
        // ... and with one it is read through that page.
        assert_eq!(
            characters(&TextValue::Ansi(vec![b'A', 0xB0]), CodePage::of(30)).as_deref(),
            Some("A\u{B0}")
        );
    }

    #[test]
    fn the_report_names_each_thing_once_with_how_many() {
        let mut notes = Notes::default();
        notes.add("b");
        notes.add("a");
        notes.add("b");
        assert_eq!(
            notes.into_diagnostics().warnings,
            vec!["a: 1".to_string(), "b: 2".to_string()]
        );
    }

    #[test]
    fn a_file_that_is_not_a_drawing_is_an_error() {
        assert!(matches!(
            read_database(b"not a drawing at all"),
            Err(DatabaseError::Drawing(DrawingError::Header(_)))
        ));
    }
}
