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
use uncad_model::model::{
    ArcEntity, AttdefEntity, AttribEntity, AttributeFlags, CircleEntity, Confidence,
    DimensionEntity, DimensionKind as ModelDimensionKind, DimensionPoints, EllipseEntity,
    EntityCommon, EntityId, EntityLinetype, Face3DEntity, HatchBoundaryPath,
    HatchEdge as ModelHatchEdge, HatchEntity, HatchPatternLine, HatchStyle,
    HorizontalJustification, ImageEntity, InsertEntity, LeaderAnnotation, LeaderEntity, LeaderPath,
    LineEntity, LwPolylineEntity, MLineEntity, MLineVertex, MTextAttachment, MTextEntity,
    OrdinateAxis, Origin, OverrideValue, Point2D, Point3D, PointEntity, PolylineEntity,
    PolylineVertex, RayEntity, Ref, SolidEntity, SplineEntity, StyleOverride, TextEntity,
    TextOverride, ToleranceEntity, VerticalJustification, ViewportEntity, ViewportView,
};
use uncad_model::tables::{
    AngularUnitFormat, ArcSymbol, BlockRecord, DimStyleRecord, ExternalReference, FractionFormat,
    ImageDefinition, LayerRecord, LayoutRecord, LinearUnitFormat, PlotPaperUnits, PlotRotation,
    PlotSettings, ResolutionUnit, Tables,
};
use uncad_model::text::decode_escapes;
use uncad_model::{CadDatabase, Entity, HeaderVariables as ModelHeader, ReadDiagnostics};

use crate::bits::TextValue;
use crate::drawing::{Drawing, DrawingError};
use crate::entities::{self, read_entity_links, read_entity_with_classes, Geometry};
use crate::objects::ObjectEntry;
use crate::tables::{
    read_appid_name, read_block_control, read_block_header, read_dim_style, read_dimstyle_name,
    read_image_definition, read_layer, read_layout, read_linetype_name, read_mline_style,
    read_mlinestyle_name, read_style_name, BlockHeader, Layer,
};
use crate::version::Version;

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

/// A text as characters. A text in a code page other than ASCII cannot be
/// turned into characters without knowing the page, so it is not guessed.
fn characters(t: &TextValue) -> Option<String> {
    let stored = match t {
        TextValue::Unicode(s) => s.clone(),
        TextValue::Ansi(b) if b.is_ascii() => String::from_utf8_lossy(b).into_owned(),
        TextValue::Ansi(_) => return None,
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

/// Reads the drawing in `bytes` into the entity model.
pub fn read_database(bytes: &[u8]) -> Result<CadDatabase, DatabaseError> {
    let drawing = Drawing::open(bytes)?;
    let version = drawing.version();
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

    // The tables the entities refer to by handle.
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
    let mut headers: Vec<BlockHeader> = Vec::new();
    let mut table_order: Vec<Option<u64>> = Vec::new();
    for e in &entries {
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
                if matches!(characters(&name).as_deref(), Some(n) if n.eq_ignore_ascii_case("ACAD"))
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

    // Block names, made unique the way a table needs. An anonymous block
    // (`*U`, `*D`, ...) is numbered by its place in the block table. A name the
    // file gives to several blocks (the paper space of each layout) belongs
    // bare to the block the header names as the current paper space and, for the
    // others, gets its place in the table as a number.
    let mut block_names: BTreeMap<u64, String> = BTreeMap::new();
    let position: BTreeMap<u64, usize> = table_order
        .iter()
        .enumerate()
        .filter_map(|(i, h)| h.map(|h| (h, i)))
        .collect();
    let mut stored_names: BTreeMap<u64, String> = BTreeMap::new();
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for block in &headers {
        let Some(stored) = characters(&block.name) else {
            notes.add("block name in a code page that is not read");
            continue;
        };
        *uses.entry(stored.clone()).or_default() += 1;
        stored_names.insert(block.handle, stored);
    }
    let current_paper = drawing.variables().paper_space_block;
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
    let model_space = block_names.get(&drawing.variables().model_space_block);
    let paper_space = block_names.get(&drawing.variables().paper_space_block);

    // The layers, by name.
    let mut layer_names: BTreeMap<u64, String> = BTreeMap::new();
    let mut layer_records: BTreeMap<String, LayerRecord> = BTreeMap::new();
    for (handle, layer) in &layers {
        let Some(name) = characters(&layer.name) else {
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
            Some(h) => match linetypes.get(&h).and_then(characters) {
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

    // Every entity the file keeps, read once.
    let by_handle: BTreeMap<u64, ObjectEntry> = entries.iter().map(|e| (e.handle, *e)).collect();
    let mut read: Vec<entities::Entity> = Vec::new();
    for e in &entries {
        match read_entity_with_classes(objects, *e, version, drawing.classes()) {
            Ok(Some(entity)) => read.push(entity),
            Ok(None) => {}
            Err(e) => notes.add(format!("entity that could not be read ({e})")),
        }
    }
    // The vertices of polylines, which the polyline that owns them takes.
    let mut vertices_2d: BTreeMap<u64, entities::Vertex2d> = BTreeMap::new();
    let mut vertices_3d: BTreeMap<u64, entities::Vertex3d> = BTreeMap::new();
    for entity in &read {
        match &entity.geometry {
            Geometry::Vertex2d(v) => {
                vertices_2d.insert(entity.common.handle, v.clone());
            }
            Geometry::Vertex3d(v) => {
                vertices_3d.insert(entity.common.handle, v.clone());
            }
            _ => {}
        }
    }
    let ctx = Context {
        version,
        objects,
        by_handle: &by_handle,
        layers: &layer_names,
        linetypes: &linetypes,
        styles: &styles,
        dim_styles: &dim_styles,
        mline_styles: &mline_styles,
        acad,
        block_names: &block_names,
        vertices_2d: &vertices_2d,
        vertices_3d: &vertices_3d,
    };

    let viewport_handles: BTreeSet<u64> = read
        .iter()
        .filter(|e| matches!(e.geometry, Geometry::Viewport(_)))
        .map(|e| e.common.handle)
        .collect();

    // The attributes first: an insertion takes the ones it lists.
    let mut attribs: BTreeMap<u64, AttribEntity> = BTreeMap::new();
    for entity in &read {
        if let Geometry::Attrib(a) = &entity.geometry {
            if let Some(text) = convert_text(&a.text, &ctx, &mut notes) {
                attribs.insert(
                    entity.common.handle,
                    AttribEntity {
                        common: convert_common(entity, &ctx),
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
                if let Some(tag) = characters(&a.tag) {
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

    // The entities, converted, with the block each one says it belongs to.
    let mut converted: BTreeMap<u64, (String, Entity)> = BTreeMap::new();
    for entity in &read {
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
            // block; it is read with that entity.
            if !matches!(
                entity.geometry,
                Geometry::Vertex2d(_) | Geometry::Vertex3d(_) | Geometry::Attrib(_)
            ) {
                notes.add("entity whose block is not known");
            }
            continue;
        };
        let Some(converted_entity) = convert_entity(entity, &ctx, &attribs, &mut notes) else {
            let kind = format!("{:?}", entity.geometry);
            let kind = kind.split(['(', ' ', '{']).next().unwrap_or("").to_string();
            notes.add(format!("{kind} entity not converted"));
            continue;
        };
        converted.insert(entity.common.handle, (owner.clone(), converted_entity));
    }

    // The entities of each block, in the order the block draws them: the
    // list the block header keeps (R2004 and later), or the chain from its
    // first entity to its last (R2000).
    let mut by_block: BTreeMap<String, Vec<Entity>> = BTreeMap::new();
    for block in &headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        let order = if version >= Version::R2004 {
            block.owned_entities.clone()
        } else {
            match (block.first_entity, block.last_entity) {
                (Some(first), Some(last)) => chain(&ctx, first, last, &mut notes),
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

    // What model space and the paper spaces own is also listed at the top.
    let mut top_level: Vec<Entity> = Vec::new();
    for block in &headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        if Some(name) == model_space
            || Some(name) == paper_space
            || name.starts_with("*Paper_Space")
        {
            top_level.extend(by_block.get(name).into_iter().flatten().cloned());
        }
    }

    // The block records, by name.
    let mut block_records: BTreeMap<String, BlockRecord> = BTreeMap::new();
    for block in &headers {
        let Some(name) = block_names.get(&block.handle) else {
            continue;
        };
        let external_reference = if block.is_xref {
            match characters(&block.xref_path) {
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

    // The multiline styles: each one's line offsets, in the order the style keeps them.
    let mut mline_style_records: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for m in &mline_style_rows {
        match characters(&m.name) {
            Some(name) => {
                mline_style_records.insert(name, m.elements.iter().map(|e| e.offset).collect());
            }
            None => notes.add("multiline style name in a code page that is not read"),
        }
    }

    // The layouts, by name.
    let mut layout_records: BTreeMap<String, LayoutRecord> = BTreeMap::new();
    for l in &layout_rows {
        let (Some(name), Some(paper_name)) = (characters(&l.name), characters(&l.paper_size))
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

    // The dimension styles, by name.
    let mut dim_style_records: BTreeMap<String, DimStyleRecord> = BTreeMap::new();
    for d in &dim_style_rows {
        let (Some(name), Some(post)) = (characters(&d.name), characters(&d.post)) else {
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

    // The image definitions, by handle (the key an image names its picture by).
    let mut image_definitions: BTreeMap<String, ImageDefinition> = BTreeMap::new();
    for (handle, def) in &image_defs {
        let file_path = match characters(&def.file_path) {
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

    let vars = drawing.variables();
    let header = ModelHeader {
        insunits: Some(vars.insertion_units as u16),
        fingerprintguid: characters(&vars.fingerprint_guid),
        versionguid: characters(&vars.version_guid),
        pdmode: Some(vars.point_mode),
        pdsize: Some(vars.point_size),
    };

    Ok(CadDatabase {
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
    })
}

/// The entities from `first` to `last` of an R2000 block (or the attributes of
/// an insertion), each naming the next -- or, where the file keeps no links, the
/// handle after its own. A break in the chain is reported, and what follows it
/// is not read.
fn chain(ctx: &Context<'_>, first: u64, last: u64, notes: &mut Notes) -> Vec<u64> {
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = first;
    loop {
        if !seen.insert(current) {
            notes.add("entity chain that loops");
            break;
        }
        order.push(current);
        if current == last {
            break;
        }
        let next = ctx
            .by_handle
            .get(&current)
            .and_then(|e| read_entity_links(ctx.objects, *e, ctx.version).ok())
            .and_then(|links| {
                if links.stored {
                    links.next
                } else {
                    current.checked_add(1)
                }
            });
        match next {
            Some(n) => current = n,
            None => {
                notes.add("entity chain that breaks");
                break;
            }
        }
    }
    order
}

/// What converting an entity needs to know about the rest of the drawing.
struct Context<'a> {
    version: Version,
    objects: &'a [u8],
    by_handle: &'a BTreeMap<u64, ObjectEntry>,
    layers: &'a BTreeMap<u64, String>,
    linetypes: &'a BTreeMap<u64, TextValue>,
    styles: &'a BTreeMap<u64, TextValue>,
    dim_styles: &'a BTreeMap<u64, TextValue>,
    mline_styles: &'a BTreeMap<u64, TextValue>,
    /// The handle of the application named ACAD, which keeps dimension overrides.
    acad: Option<u64>,
    block_names: &'a BTreeMap<u64, String>,
    vertices_2d: &'a BTreeMap<u64, entities::Vertex2d>,
    vertices_3d: &'a BTreeMap<u64, entities::Vertex3d>,
}

fn point2(p: [f64; 2]) -> Point2D {
    Point2D { x: p[0], y: p[1] }
}

/// The fields a text, an attribute and an attribute definition share, in the
/// model's words.
struct TextFields {
    start_point: Point2D,
    height: f64,
    value: String,
    rotation: f64,
    horizontal: HorizontalJustification,
    vertical: VerticalJustification,
    alignment: Option<Point2D>,
    width_factor: f64,
    oblique: f64,
    style: Ref<String>,
    elevation: f64,
    extrusion: Point3D,
}

/// `None` (reported) when the text is in a code page this reader cannot turn
/// into characters.
fn convert_text(t: &entities::Text, ctx: &Context<'_>, notes: &mut Notes) -> Option<TextFields> {
    let Some(value) = characters(&t.value) else {
        notes.add("text in a code page that is not read");
        return None;
    };
    let style = match ctx.styles.get(&t.style) {
        None => Ref::Unresolved(hex(t.style)),
        Some(n) => match characters(n) {
            Some(n) => Ref::Resolved(n),
            None => Ref::Unresolved(hex(t.style)),
        },
    };
    Some(TextFields {
        start_point: point2(t.insertion),
        height: t.height,
        value,
        rotation: t.rotation,
        horizontal: match t.horizontal {
            1 => HorizontalJustification::Center,
            2 => HorizontalJustification::Right,
            3 => HorizontalJustification::Aligned,
            4 => HorizontalJustification::Middle,
            5 => HorizontalJustification::Fit,
            _ => HorizontalJustification::Left,
        },
        vertical: match t.vertical {
            1 => VerticalJustification::Bottom,
            2 => VerticalJustification::Middle,
            3 => VerticalJustification::Top,
            _ => VerticalJustification::Baseline,
        },
        // A text justified left on the baseline states no alignment point; any other
        // justification does, and a file that leaves it out means the origin.
        alignment: if t.horizontal == 0 && t.vertical == 0 {
            None
        } else {
            Some(point2(t.alignment.unwrap_or([0.0, 0.0])))
        },
        width_factor: t.width_factor,
        oblique: t.oblique,
        style,
        elevation: t.elevation,
        extrusion: point(t.extrusion),
    })
}

/// The name a handle points at in a table of names, as a model reference.
fn named(table: &BTreeMap<u64, TextValue>, handle: Option<u64>) -> Ref<String> {
    match handle {
        None => Ref::Absent,
        Some(h) => match table.get(&h).and_then(characters) {
            Some(n) => Ref::Resolved(n),
            None => Ref::Unresolved(hex(h)),
        },
    }
}

/// The handles of the objects an entity owns (its vertices): all of them from
/// R2004; R2000 names the first and the last and links the rest.
fn owned(ctx: &Context<'_>, handles: &[u64], notes: &mut Notes) -> Vec<u64> {
    match (ctx.version, handles.first(), handles.last()) {
        (Version::R2000, Some(first), Some(last)) => chain(ctx, *first, *last, notes),
        _ => handles.to_vec(),
    }
}

fn attribute_flags(bits: u8) -> AttributeFlags {
    AttributeFlags {
        invisible: bits & 1 != 0,
        constant: bits & 2 != 0,
        verify: bits & 4 != 0,
        preset: bits & 8 != 0,
    }
}

/// The dimension-style variables a dimension sets for itself: the `DSTYLE`
/// list in its `ACAD` extended data, as (variable, value) pairs. An entity
/// without one sets none; data that does not read as the format describes it
/// ends the list there.
fn style_overrides(entity: &entities::Entity, ctx: &Context<'_>) -> Vec<StyleOverride> {
    let unicode = ctx.version >= Version::R2007;
    let mut out = Vec::new();
    for data in &entity.common.extended_data {
        if Some(data.application) != ctx.acad {
            continue;
        }
        let items = extended_items(&data.bytes, unicode);
        // `1000 DSTYLE`, `1002 {`, pairs of a `1070` variable and a value, `1002 }`.
        let mut it = items.into_iter();
        if !matches!(it.next(), Some(Item::Text(t)) if t == "DSTYLE") {
            continue;
        }
        if !matches!(it.next(), Some(Item::Brace(false))) {
            continue;
        }
        while let Some(Item::Integer(variable)) = it.next() {
            let value = match it.next() {
                Some(Item::Real(v)) => OverrideValue::Real(v),
                Some(Item::Integer(v)) => OverrideValue::Integer(v),
                Some(Item::Text(t)) => OverrideValue::Text(t),
                Some(Item::Handle(h)) => OverrideValue::Handle(hex(h)),
                _ => break,
            };
            if let Ok(variable) = u16::try_from(variable) {
                out.push(StyleOverride { variable, value });
            }
        }
    }
    out
}

/// One item of an application's extended data.
enum Item {
    Text(String),
    /// `1002`: `false` opens a group, `true` closes it.
    Brace(bool),
    Integer(i32),
    Real(f64),
    Handle(u64),
    /// Anything else the format allows (binary chunks, points, layer handles):
    /// it is skipped over, not read.
    Other,
}

/// The items of an application's extended data. Reading stops at the first
/// item the format does not describe.
fn extended_items(bytes: &[u8], unicode: bool) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0usize;
    let take = |i: &mut usize, n: usize| -> Option<&[u8]> {
        let slice = bytes.get(*i..i.checked_add(n)?)?;
        *i += n;
        Some(slice)
    };
    while let Some(code) = bytes.get(i).copied() {
        i += 1;
        let item = match code {
            0 => {
                let text = if unicode {
                    let len =
                        take(&mut i, 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                    len.and_then(|n| take(&mut i, n * 2)).and_then(|b| {
                        let units: Vec<u16> = (0..b.len() / 2)
                            .map(|k| u16::from_le_bytes([b[2 * k], b[2 * k + 1]]))
                            .collect();
                        String::from_utf16(&units).ok()
                    })
                } else {
                    let len = take(&mut i, 1).map(|b| usize::from(b[0]));
                    len.and_then(|n| take(&mut i, 2).and(take(&mut i, n)))
                        .and_then(|b| {
                            b.is_ascii()
                                .then(|| String::from_utf8_lossy(b).into_owned())
                        })
                };
                match text {
                    Some(t) => Item::Text(t),
                    None => break,
                }
            }
            2 => match take(&mut i, 1) {
                Some(b) => Item::Brace(b[0] != 0),
                None => break,
            },
            3 | 5 => match take(&mut i, 8) {
                Some(b) => {
                    let h = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
                    if code == 5 {
                        Item::Handle(h)
                    } else {
                        Item::Other
                    }
                }
                None => break,
            },
            4 => match take(&mut i, 1)
                .map(|b| usize::from(b[0]))
                .and_then(|n| take(&mut i, n))
            {
                Some(_) => Item::Other,
                None => break,
            },
            10..=13 => match take(&mut i, 24) {
                Some(_) => Item::Other,
                None => break,
            },
            40..=42 => match take(&mut i, 8) {
                Some(b) => Item::Real(f64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ])),
                None => break,
            },
            70 => match take(&mut i, 2) {
                Some(b) => Item::Integer(i32::from(i16::from_le_bytes([b[0], b[1]]))),
                None => break,
            },
            71 => match take(&mut i, 4) {
                Some(b) => Item::Integer(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                None => break,
            },
            _ => break,
        };
        items.push(item);
    }
    items
}

/// A tangent the file states, or `None` for the zero vector (which states none).
fn tangent(t: Option<[f64; 3]>) -> Option<Point3D> {
    t.filter(|v| v.iter().any(|c| *c != 0.0)).map(point)
}

/// The model entity for `entity`, or `None` for a kind this reader does not
/// convert yet (or a text it cannot turn into characters, which is reported).
fn convert_entity(
    entity: &entities::Entity,
    ctx: &Context<'_>,
    attribs: &BTreeMap<u64, AttribEntity>,
    notes: &mut Notes,
) -> Option<Entity> {
    let common = convert_common(entity, ctx);
    Some(match &entity.geometry {
        Geometry::Line(l) => Entity::Line(LineEntity {
            common,
            start_point: point(l.start),
            end_point: point(l.end),
        }),
        Geometry::Circle(c) => Entity::Circle(CircleEntity {
            common,
            center: point(c.center),
            radius: c.radius,
            extrusion: point(c.extrusion),
        }),
        Geometry::Arc(a) => Entity::Arc(ArcEntity {
            common,
            center: point(a.center),
            radius: a.radius,
            start_angle: a.start_angle,
            end_angle: a.end_angle,
            extrusion: point(a.extrusion),
        }),
        Geometry::Point(p) => Entity::Point(PointEntity {
            common,
            position: point(p.position),
        }),
        Geometry::LwPolyline(p) => Entity::LwPolyline(LwPolylineEntity {
            common,
            vertices: p
                .vertices
                .iter()
                .map(|v| PolylineVertex {
                    point: point2(v.point),
                    bulge: v.bulge,
                    start_width: v.start_width,
                    end_width: v.end_width,
                })
                .collect(),
            closed: p.closed,
            const_width: p.const_width,
            elevation: p.elevation,
            extrusion: point(p.normal),
        }),
        Geometry::Solid(s) | Geometry::Trace(s) => {
            let solid = SolidEntity {
                common,
                corner1: point2(s.corners[0]),
                corner2: point2(s.corners[1]),
                corner3: point2(s.corners[2]),
                corner4: point2(s.corners[3]),
                elevation: s.elevation,
                extrusion: point(s.extrusion),
            };
            if matches!(entity.geometry, Geometry::Trace(_)) {
                Entity::Trace(solid)
            } else {
                Entity::Solid(solid)
            }
        }
        Geometry::Ray(r) => Entity::Ray(RayEntity {
            common,
            point: point(r.point),
            vector: point(r.vector),
        }),
        Geometry::Xline(r) => Entity::XLine(RayEntity {
            common,
            point: point(r.point),
            vector: point(r.vector),
        }),
        Geometry::Ellipse(e) => Entity::Ellipse(EllipseEntity {
            common,
            center: point(e.center),
            major_axis_endpoint: point(e.major_axis),
            axis_ratio: e.axis_ratio,
            start_angle: e.start_angle,
            end_angle: e.end_angle,
            extrusion: point(e.extrusion),
        }),
        Geometry::Face3d(f) => Entity::Face3D(Face3DEntity {
            common,
            corner1: point(f.corners[0]),
            corner2: point(f.corners[1]),
            corner3: point(f.corners[2]),
            corner4: point(f.corners[3]),
            invisible_edges: Face3DEntity::invisible_edges_from_bits(f.invisible_edges as u32),
        }),
        Geometry::Text(t) => {
            let t = convert_text(t, ctx, notes)?;
            Entity::Text(TextEntity {
                common,
                start_point: t.start_point,
                text_height: t.height,
                text: t.value,
                rotation: t.rotation,
                horizontal_justification: t.horizontal,
                vertical_justification: t.vertical,
                alignment_point: t.alignment,
                width_factor: t.width_factor,
                oblique_angle: t.oblique,
                style_name: t.style,
                elevation: t.elevation,
                extrusion: t.extrusion,
            })
        }
        Geometry::Attdef(a) => {
            let t = convert_text(&a.text, ctx, notes)?;
            let Some(tag) = characters(&a.tag) else {
                notes.add("attribute tag in a code page that is not read");
                return None;
            };
            Entity::Attdef(AttdefEntity {
                common,
                start_point: t.start_point,
                text_height: t.height,
                tag,
                flags: attribute_flags(a.flags),
                default_value: t.value,
                rotation: t.rotation,
                horizontal_justification: t.horizontal,
                vertical_justification: t.vertical,
                alignment_point: t.alignment,
                width_factor: t.width_factor,
                oblique_angle: t.oblique,
                style_name: t.style,
                elevation: t.elevation,
                extrusion: t.extrusion,
            })
        }
        Geometry::Insert(i) => {
            // R2000 names the first and the last attribute and links the rest;
            // later versions list them all.
            let handles = match (ctx.version, i.attributes.first(), i.attributes.last()) {
                (Version::R2000, Some(first), Some(last)) => chain(ctx, *first, *last, notes),
                _ => i.attributes.clone(),
            };
            Entity::Insert(InsertEntity {
                common,
                block_name: match ctx.block_names.get(&i.block) {
                    Some(n) => Ref::Resolved(n.clone()),
                    None => Ref::Unresolved(hex(i.block)),
                },
                insertion_point: point(i.position),
                scale: point(i.scale),
                rotation: i.rotation,
                attribs: handles
                    .iter()
                    .filter_map(|h| attribs.get(h).cloned())
                    .collect(),
                extrusion: point(i.extrusion),
            })
        }
        Geometry::Mtext(m) => {
            let Some(text) = characters(&m.value) else {
                notes.add("text in a code page that is not read");
                return None;
            };
            let style_name = match ctx.styles.get(&m.style) {
                None => Ref::Unresolved(hex(m.style)),
                Some(n) => match characters(n) {
                    Some(n) => Ref::Resolved(n),
                    None => Ref::Unresolved(hex(m.style)),
                },
            };
            Entity::MText(MTextEntity {
                common,
                insertion_point: point(m.insertion),
                text,
                text_height: m.text_height,
                rotation: m.x_axis[1].atan2(m.x_axis[0]),
                line_spacing_factor: m.line_spacing_factor,
                attachment: MTextAttachment::from_code(i64::from(m.attachment)),
                reference_width: m.rect_width,
                extents_width: Some(m.extents_width),
                extents_height: Some(m.extents_height),
                style_name,
            })
        }
        Geometry::Spline(s) => Entity::Spline(SplineEntity {
            common,
            degree: s.degree,
            // A control-point spline states whether it is closed and periodic; a
            // fit-point spline states closed from R2013 (its flag), and no periodic.
            closed: if s.fit_data {
                (ctx.version >= Version::R2013).then_some(s.flags & 4 != 0)
            } else {
                Some(s.closed)
            },
            periodic: (!s.fit_data).then_some(s.periodic),
            knots: s.knots.clone(),
            weights: s.weights.clone(),
            fit_points: s.fit_points.iter().copied().map(point).collect(),
            control_points: s.control_points.iter().copied().map(point).collect(),
            start_tangent: tangent(s.start_tangent),
            end_tangent: tangent(s.end_tangent),
        }),
        Geometry::Polyline2d(p) => {
            let vertices: Vec<PolylineVertex> = owned(ctx, &p.vertices, notes)
                .iter()
                .filter_map(|h| ctx.vertices_2d.get(h))
                .map(|v| PolylineVertex {
                    point: Point2D {
                        x: v.point[0],
                        y: v.point[1],
                    },
                    bulge: v.bulge,
                    start_width: v.start_width,
                    end_width: v.end_width,
                })
                .collect();
            Entity::Polyline2D(LwPolylineEntity {
                common,
                vertices,
                closed: p.flags & 1 != 0,
                const_width: 0.0,
                elevation: p.elevation,
                extrusion: point(p.extrusion),
            })
        }
        Geometry::Polyline3d(p) => Entity::Polyline3D(PolylineEntity {
            common,
            vertices: owned(ctx, &p.vertices, notes)
                .iter()
                .filter_map(|h| ctx.vertices_3d.get(h))
                .map(|v| point(v.point))
                .collect(),
            closed: p.closed_flags & 1 != 0,
        }),
        Geometry::Tolerance(t) => {
            let Some(text_value) = characters(&t.text) else {
                notes.add("text in a code page that is not read");
                return None;
            };
            Entity::Tolerance(ToleranceEntity {
                common,
                insertion_point: point(t.insertion),
                text_height: None,
                text_value,
                direction: Some(point(t.x_direction)),
                style_name: named(ctx.dim_styles, t.style),
            })
        }
        Geometry::Mline(m) => Entity::MLine(MLineEntity {
            common,
            vertices: m
                .vertices
                .iter()
                .map(|v| MLineVertex {
                    point: point(v.point),
                    miter_direction: point(v.miter),
                })
                .collect(),
            closed: m.open_closed == 3,
            mlinestyle_name: named(ctx.mline_styles, m.style),
            scale: Some(m.scale),
        }),
        Geometry::Viewport(v) => Entity::Viewport(ViewportEntity {
            common,
            center: point(v.center),
            width: v.width,
            height: v.height,
            view: Some(ViewportView {
                center: point2(v.view_center),
                height: v.view_height,
                target: point(v.view_target),
                direction: point(v.view_direction),
                twist: v.view_twist,
                lens_length: v.lens_length,
            }),
            on: Some(v.status_flags & 0x20000 == 0),
            viewport_id: None,
            frozen_layers: v
                .frozen_layers
                .iter()
                .map(|h| match ctx.layers.get(h) {
                    Some(n) => Ref::Resolved(n.clone()),
                    None => Ref::Unresolved(hex(*h)),
                })
                .collect(),
        }),
        Geometry::Dimension(d) => {
            let text_override = match characters(&d.user_text) {
                None => {
                    notes.add("text in a code page that is not read");
                    return None;
                }
                Some(t) if t.is_empty() || t == "<>" => TextOverride::Measured,
                Some(t) if t == " " => TextOverride::Suppressed,
                Some(t) => TextOverride::Literal(t),
            };
            let at_elevation = |p: [f64; 2]| Point3D {
                x: p[0],
                y: p[1],
                z: d.elevation,
            };
            Entity::Dimension(DimensionEntity {
                common,
                block_name: match d.block {
                    None => Ref::Absent,
                    Some(h) => match ctx.block_names.get(&h) {
                        Some(n) => Ref::Resolved(n.clone()),
                        None => Ref::Unresolved(hex(h)),
                    },
                },
                kind: Some(match d.kind {
                    entities::DimensionKind::Linear => ModelDimensionKind::Rotated,
                    entities::DimensionKind::Aligned => ModelDimensionKind::Aligned,
                    entities::DimensionKind::Angular2Line => ModelDimensionKind::Angular2Line,
                    entities::DimensionKind::Diameter => ModelDimensionKind::Diameter,
                    entities::DimensionKind::Radius => ModelDimensionKind::Radius,
                    entities::DimensionKind::Angular3Point => ModelDimensionKind::Angular3Point,
                    entities::DimensionKind::Ordinate => ModelDimensionKind::Ordinate,
                }),
                // A measurement is a length or an angle, never negative: a writer that
                // did not measure the dimension stores -1.
                measurement: (d.measurement >= 0.0).then_some(d.measurement),
                text_override,
                definition_point: Some(point(d.definition_point)),
                text_midpoint: point2(d.text_midpoint),
                points: DimensionPoints {
                    extension1: d.point13.map(point),
                    extension2: d.point14.map(point),
                    radial: d.point15.map(point),
                    arc: d.point16.map(at_elevation),
                },
                rotation: d.rotation.unwrap_or(0.0),
                text_rotation: d.text_rotation,
                style_name: named(ctx.dim_styles, d.style),
                ordinate_axis: d.ordinate_flags.map(|f| {
                    if f & 1 != 0 {
                        OrdinateAxis::X
                    } else {
                        OrdinateAxis::Y
                    }
                }),
                style_overrides: Some(style_overrides(entity, ctx)),
            })
        }
        Geometry::Hatch(h) => {
            // A gradient fill carries colours this reader does not read yet.
            if h.gradient {
                notes.add("hatch with a gradient fill entity not converted");
                return None;
            }
            Entity::Hatch(HatchEntity {
                common,
                boundary_paths: h.paths.iter().map(hatch_path).collect(),
                solid_fill: h.solid_fill,
                gradient: None,
                pattern_lines: h
                    .pattern_lines
                    .iter()
                    .map(|l| HatchPatternLine {
                        angle: l.angle,
                        base_point: point2(l.base_point),
                        offset: point2(l.offset),
                        dash_pattern: l.dashes.clone(),
                    })
                    .collect(),
                elevation: h.elevation,
                extrusion: point(h.extrusion),
                style: HatchStyle::from_code(i64::from(h.style)),
            })
        }
        Geometry::Image(i) => {
            // The file states the clip boundary in pixels (the centre of a pixel is
            // half a pixel in, and rows count down from the top); the model states
            // it in the drawing, and a rectangle as its four corners.
            let to_world = |p: [f64; 2]| Point2D {
                x: i.insertion[0]
                    + i.u_vector[0] * (p[0] + 0.5)
                    + i.v_vector[0] * (i.size[1] - (p[1] + 0.5)),
                y: i.insertion[1]
                    + i.u_vector[1] * (p[0] + 0.5)
                    + i.v_vector[1] * (i.size[1] - (p[1] + 0.5)),
            };
            let corners: Vec<[f64; 2]> = match (i.clip_type, i.clip_boundary.as_slice()) {
                (1, [a, b]) => vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]],
                (_, points) => points.to_vec(),
            };
            Entity::Image(ImageEntity {
                common,
                insertion_point: point(i.insertion),
                u_vector: point(i.u_vector),
                v_vector: point(i.v_vector),
                size_pixels: point2(i.size),
                definition: match i.definition {
                    Some(h) => Ref::Resolved(hex(h)),
                    None => Ref::Absent,
                },
                display_flags: Some(i.display_flags as u16),
                clipping: Some(i.clipping),
                brightness: Some(i.brightness),
                contrast: Some(i.contrast),
                fade: Some(i.fade),
                clip_outside: i.clip_mode,
                boundary: corners.into_iter().map(to_world).collect(),
            })
        }
        Geometry::Leader(l) => Entity::Leader(LeaderEntity {
            common,
            vertices: l.points.iter().copied().map(point).collect(),
            has_arrowhead: Some(l.arrowhead),
            path_type: Some(if l.path_type == 1 {
                LeaderPath::Spline
            } else {
                LeaderPath::Straight
            }),
            annotation: match l.annotation_type {
                0 => LeaderAnnotation::MText,
                1 => LeaderAnnotation::Tolerance,
                2 => LeaderAnnotation::Insert,
                _ => LeaderAnnotation::Nothing,
            },
            annotation_id: match l.annotation {
                Some(h) => Ref::Resolved(EntityId::new(h)),
                None => Ref::Absent,
            },
            style_name: named(ctx.dim_styles, l.style),
            style_overrides: Some(style_overrides(entity, ctx)),
        }),
        _ => return None,
    })
}

/// A hatch boundary in the model's words.
fn hatch_path(path: &entities::HatchPath) -> HatchBoundaryPath {
    if path.flags & 2 != 0 {
        HatchBoundaryPath::Polyline(
            path.polyline
                .iter()
                .map(|(p, bulge)| PolylineVertex {
                    point: point2(*p),
                    bulge: *bulge,
                    start_width: 0.0,
                    end_width: 0.0,
                })
                .collect(),
        )
    } else {
        HatchBoundaryPath::Edges(
            path.edges
                .iter()
                .map(|e| match e {
                    entities::HatchEdge::Line { start, end } => ModelHatchEdge::Line {
                        start: point2(*start),
                        end: point2(*end),
                    },
                    entities::HatchEdge::Arc {
                        center,
                        radius,
                        start_angle,
                        end_angle,
                        counter_clockwise,
                    } => ModelHatchEdge::Arc {
                        center: point2(*center),
                        radius: *radius,
                        start_angle: *start_angle,
                        end_angle: *end_angle,
                        is_ccw: *counter_clockwise,
                    },
                    entities::HatchEdge::Ellipse {
                        center,
                        major_axis,
                        ratio,
                        start_angle,
                        end_angle,
                        counter_clockwise,
                    } => ModelHatchEdge::Ellipse {
                        center: point2(*center),
                        end: point2(*major_axis),
                        minor_major_ratio: *ratio,
                        start_angle: *start_angle,
                        end_angle: *end_angle,
                        is_ccw: *counter_clockwise,
                    },
                    entities::HatchEdge::Spline {
                        degree,
                        rational,
                        periodic,
                        knots,
                        control_points,
                        fit_points,
                        tangents,
                    } => ModelHatchEdge::Spline {
                        degree: *degree,
                        rational: *rational,
                        periodic: *periodic,
                        knots: knots.clone(),
                        control_points: control_points.iter().map(|(p, _)| point2(*p)).collect(),
                        weights: control_points.iter().map(|(_, w)| *w).collect(),
                        fit_points: fit_points.iter().copied().map(point2).collect(),
                        start_tangent: tangents.map(|(s, _)| point2(s)),
                        end_tangent: tangents.map(|(_, e)| point2(e)),
                    },
                })
                .collect(),
        )
    }
}

/// What every entity states, in the model's words.
fn convert_common(entity: &entities::Entity, ctx: &Context<'_>) -> EntityCommon {
    let (version, layers, linetypes) = (ctx.version, ctx.layers, ctx.linetypes);
    let c = &entity.common;
    let refs = &entity.references;
    let layer = match layers.get(&refs.layer) {
        Some(name) => Ref::Resolved(name.clone()),
        None => Ref::Unresolved(hex(refs.layer)),
    };
    // The colour flags carry the index in the low nine bits; a true colour
    // stands in its place.
    let color_index = (c.color & 0x1FF) as i16;
    let linetype = match c.linetype_flags {
        0 => EntityLinetype::ByLayer,
        1 => EntityLinetype::ByBlock,
        2 => EntityLinetype::Named(Ref::Resolved("Continuous".to_string())),
        _ => EntityLinetype::Named(match refs.linetype {
            None => Ref::Absent,
            Some(h) => match linetypes.get(&h).and_then(characters) {
                Some(n) => Ref::Resolved(n),
                None => Ref::Unresolved(hex(h)),
            },
        }),
    };
    EntityCommon {
        id: EntityId::new(c.handle),
        origin: Origin::Vector,
        confidence: Confidence::High,
        source_handle: Ref::Resolved(hex(c.handle)),
        layer,
        color_index,
        true_color: c.rgb,
        invisible: c.invisible & 1 != 0,
        linetype,
        linetype_scale: c.linetype_scale,
        lineweight: lineweight(c.lineweight),
        transparency: (version >= Version::R2004).then(|| c.transparency.unwrap_or(0)),
    }
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
    fn a_text_in_a_code_page_is_not_turned_into_characters() {
        assert_eq!(
            characters(&TextValue::Unicode("Layer".into())).as_deref(),
            Some("Layer")
        );
        assert_eq!(
            characters(&TextValue::Ansi(b"Layer".to_vec())).as_deref(),
            Some("Layer")
        );
        // A byte above 0x7F means something only in the drawing's code page.
        assert_eq!(characters(&TextValue::Ansi(vec![b'A', 0xB0])), None);
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
