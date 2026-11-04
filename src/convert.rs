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
    ArcEntity, AttdefEntity, AttribEntity, AttributeFlags, CircleEntity, Confidence, EllipseEntity,
    EntityCommon, EntityId, EntityLinetype, Face3DEntity, HorizontalJustification, InsertEntity,
    LineEntity, LwPolylineEntity, MLineEntity, MLineVertex, MTextAttachment, MTextEntity, Origin,
    Point2D, Point3D, PointEntity, PolylineEntity, PolylineVertex, RayEntity, Ref, SolidEntity,
    SplineEntity, TextEntity, ToleranceEntity, VerticalJustification, ViewportEntity, ViewportView,
};
use uncad_model::tables::{BlockRecord, ExternalReference, LayerRecord, Tables};
use uncad_model::text::decode_escapes;
use uncad_model::{CadDatabase, Entity, HeaderVariables as ModelHeader, ReadDiagnostics};

use crate::bits::TextValue;
use crate::drawing::{Drawing, DrawingError};
use crate::entities::{self, read_entity_links, read_entity_with_classes, Geometry};
use crate::objects::ObjectEntry;
use crate::tables::{
    read_block_control, read_block_header, read_dimstyle_name, read_layer, read_linetype_name,
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
    let mut mline_styles: BTreeMap<u64, TextValue> = BTreeMap::new();
    let mut headers: Vec<BlockHeader> = Vec::new();
    let mut table_order: Vec<u64> = Vec::new();
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

    // Block names, made unique the way a table needs: an anonymous block
    // (`*U`, `*D`, ...) is numbered by its place in the block table, and a
    // name the file gives twice (the paper space of each layout) gets a
    // counter from the second on.
    let mut block_names: BTreeMap<u64, String> = BTreeMap::new();
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    let position: BTreeMap<u64, usize> = table_order
        .iter()
        .enumerate()
        .map(|(i, h)| (*h, i))
        .collect();
    for (index, block) in headers.iter().enumerate() {
        let index = position.get(&block.handle).copied().unwrap_or(index);
        let Some(stored) = characters(&block.name) else {
            notes.add("block name in a code page that is not read");
            continue;
        };
        let name = if block.anonymous {
            format!("{stored}{index}")
        } else {
            let seen = taken.entry(stored.clone()).or_default();
            *seen += 1;
            if *seen == 1 {
                stored
            } else {
                format!("{stored}{}", *seen - 2)
            }
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
        block_names: &block_names,
        vertices_2d: &vertices_2d,
        vertices_3d: &vertices_3d,
    };

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
            ..Tables::default()
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
        _ => return None,
    })
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
