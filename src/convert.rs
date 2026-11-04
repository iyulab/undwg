//! Reading a whole drawing into the [`uncad_model`] entity model.
//!
//! [`read_database`] opens the file, reads its tables, header variables and
//! entities, and returns a [`CadDatabase`]. The model's own rule holds:
//! nothing read is silently dropped. What this reader does not convert yet, an
//! object it cannot decode and a text it cannot turn into characters are
//! each named in the drawing's [`ReadDiagnostics`], with how many there were.

use std::collections::BTreeMap;
use std::fmt;

use uncad_model::color::ACI_PALETTE;
use uncad_model::model::{
    ArcEntity, CircleEntity, Confidence, EllipseEntity, EntityCommon, EntityId, EntityLinetype,
    Face3DEntity, HorizontalJustification, LineEntity, LwPolylineEntity, Origin, Point2D, Point3D,
    PointEntity, PolylineVertex, RayEntity, Ref, SolidEntity, TextEntity, VerticalJustification,
};
use uncad_model::tables::{BlockRecord, ExternalReference, LayerRecord, Tables};
use uncad_model::text::decode_escapes;
use uncad_model::{CadDatabase, Entity, HeaderVariables as ModelHeader, ReadDiagnostics};

use crate::bits::TextValue;
use crate::drawing::{Drawing, DrawingError};
use crate::entities::{self, read_entity_links, read_entity_with_classes, Geometry};
use crate::objects::ObjectEntry;
use crate::tables::{
    read_block_header, read_layer, read_linetype_name, read_style_name, BlockHeader, Layer,
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
    let mut headers: Vec<BlockHeader> = Vec::new();
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
    for (index, block) in headers.iter().enumerate() {
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

    // The entities, converted, with the block each one says it belongs to.
    let mut converted: BTreeMap<u64, (String, Entity)> = BTreeMap::new();
    for e in &entries {
        let entity = match read_entity_with_classes(objects, *e, version, drawing.classes()) {
            Ok(Some(entity)) => entity,
            Ok(None) => continue,
            Err(e) => {
                notes.add(format!("entity that could not be read ({e})"));
                continue;
            }
        };
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
        let ctx = Context {
            version,
            layers: &layer_names,
            linetypes: &linetypes,
            styles: &styles,
        };
        let Some(entity) = convert_entity(&entity, &ctx, &mut notes) else {
            let kind = format!("{:?}", entity.geometry);
            let kind = kind.split(['(', ' ', '{']).next().unwrap_or("").to_string();
            notes.add(format!("{kind} entity not converted"));
            continue;
        };
        converted.insert(e.handle, (owner.clone(), entity));
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
            chain(objects, &entries, version, block, &mut notes)
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

/// The entities of an R2000 block in drawing order: from the first to the last,
/// each naming the next -- or, where the file keeps no links, the handle after
/// its own. A break in the chain is reported, and what follows it is not read.
fn chain(
    objects: &[u8],
    entries: &[ObjectEntry],
    version: Version,
    block: &BlockHeader,
    notes: &mut Notes,
) -> Vec<u64> {
    let (Some(first), Some(last)) = (block.first_entity, block.last_entity) else {
        return Vec::new();
    };
    let by_handle: BTreeMap<u64, ObjectEntry> = entries.iter().map(|e| (e.handle, *e)).collect();
    let mut order = Vec::new();
    let mut current = first;
    loop {
        if order.contains(&current) {
            notes.add("entity chain that loops");
            break;
        }
        order.push(current);
        if current == last {
            break;
        }
        let next = by_handle
            .get(&current)
            .and_then(|e| read_entity_links(objects, *e, version).ok())
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
    layers: &'a BTreeMap<u64, String>,
    linetypes: &'a BTreeMap<u64, TextValue>,
    styles: &'a BTreeMap<u64, TextValue>,
}

fn point2(p: [f64; 2]) -> Point2D {
    Point2D { x: p[0], y: p[1] }
}

/// The model entity for `entity`, or `None` for a kind this reader does not
/// convert yet (or a text it cannot turn into characters, which is reported).
fn convert_entity(
    entity: &entities::Entity,
    ctx: &Context<'_>,
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
            let Some(text) = characters(&t.value) else {
                notes.add("text in a code page that is not read");
                return None;
            };
            let style_name = match ctx.styles.get(&t.style) {
                None => Ref::Unresolved(hex(t.style)),
                Some(n) => match characters(n) {
                    Some(n) => Ref::Resolved(n),
                    None => Ref::Unresolved(hex(t.style)),
                },
            };
            Entity::Text(TextEntity {
                common,
                start_point: point2(t.insertion),
                text_height: t.height,
                text,
                rotation: t.rotation,
                horizontal_justification: match t.horizontal {
                    1 => HorizontalJustification::Center,
                    2 => HorizontalJustification::Right,
                    3 => HorizontalJustification::Aligned,
                    4 => HorizontalJustification::Middle,
                    5 => HorizontalJustification::Fit,
                    _ => HorizontalJustification::Left,
                },
                vertical_justification: match t.vertical {
                    1 => VerticalJustification::Bottom,
                    2 => VerticalJustification::Middle,
                    3 => VerticalJustification::Top,
                    _ => VerticalJustification::Baseline,
                },
                alignment_point: t.alignment.map(point2),
                width_factor: t.width_factor,
                oblique_angle: t.oblique,
                style_name,
                elevation: t.elevation,
                extrusion: point(t.extrusion),
            })
        }
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
