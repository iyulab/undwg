//! Turning one entity the file states into the model's entity.

use std::collections::{BTreeMap, BTreeSet};

use uncad_model::model::{
    ArcEntity, AttdefEntity, AttribEntity, AttributeFlags, CircleEntity, Confidence,
    DimensionEntity, DimensionKind as ModelDimensionKind, DimensionPoints, EllipseEntity,
    EntityCommon, EntityId, EntityLinetype, Face3DEntity, HatchBoundaryPath,
    HatchEdge as ModelHatchEdge, HatchEntity, HatchGradient as ModelHatchGradient,
    HatchPatternLine, HatchStyle, HorizontalJustification, ImageEntity, InsertEntity,
    LeaderAnnotation, LeaderEntity, LeaderPath, LineEntity, LwPolylineEntity, MLineEntity,
    MLineVertex, MTextAttachment, MTextEntity, OrdinateAxis, Origin, Point2D, Point3D, PointEntity,
    PolylineEntity, PolylineVertex, RayEntity, Ref, Solid3DEntity, SolidEntity, SplineEntity,
    TextEntity, TextOverride, ToleranceEntity, VerticalJustification, ViewportEntity, ViewportView,
    WipeoutEntity,
};
use uncad_model::Entity;

use crate::bits::TextValue;
use crate::codepage::CodePage;
use crate::entities::{self, read_entity_links, Geometry};
use crate::objects::ObjectEntry;
use crate::version::Version;

use super::extended::style_overrides;
use super::{characters, hex, lineweight, point, Notes};

/// The entities from `first` to `last` of an R2000 block (or the attributes of
/// an insertion), each naming the next -- or, where the file keeps no links, the
/// handle after its own. A break in the chain is reported, and what follows it
/// is not read.
pub(super) fn chain(ctx: &Context<'_>, first: u64, last: u64, notes: &mut Notes) -> Vec<u64> {
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
pub(super) struct Context<'a> {
    pub(super) version: Version,
    pub(super) objects: &'a [u8],
    pub(super) by_handle: &'a BTreeMap<u64, ObjectEntry>,
    pub(super) layers: &'a BTreeMap<u64, String>,
    pub(super) linetypes: &'a BTreeMap<u64, TextValue>,
    pub(super) styles: &'a BTreeMap<u64, TextValue>,
    pub(super) dim_styles: &'a BTreeMap<u64, TextValue>,
    pub(super) mline_styles: &'a BTreeMap<u64, TextValue>,
    /// The leader type each multileader style states.
    pub(super) mleader_styles: &'a BTreeMap<u64, i16>,
    /// The handle of the application named ACAD, which keeps dimension overrides.
    pub(super) acad: Option<u64>,
    pub(super) block_names: &'a BTreeMap<u64, String>,
    pub(super) vertices_2d: &'a BTreeMap<u64, entities::Vertex2d>,
    pub(super) vertices_3d: &'a BTreeMap<u64, entities::Vertex3d>,
    pub(super) faces: &'a BTreeMap<u64, entities::VertexFace>,
    /// How the drawing's code page text becomes characters.
    pub(super) page: CodePage,
}

pub(super) fn point2(p: [f64; 2]) -> Point2D {
    Point2D { x: p[0], y: p[1] }
}

/// The fields a text, an attribute and an attribute definition share, in the
/// model's words.
pub(super) struct TextFields {
    pub(super) start_point: Point2D,
    pub(super) height: f64,
    pub(super) value: String,
    pub(super) rotation: f64,
    pub(super) horizontal: HorizontalJustification,
    pub(super) vertical: VerticalJustification,
    pub(super) alignment: Option<Point2D>,
    pub(super) width_factor: f64,
    pub(super) oblique: f64,
    pub(super) style: Ref<String>,
    pub(super) elevation: f64,
    pub(super) extrusion: Point3D,
}

/// `None` (reported) when the text is in a code page this reader cannot turn
/// into characters.
pub(super) fn convert_text(
    t: &entities::Text,
    ctx: &Context<'_>,
    notes: &mut Notes,
) -> Option<TextFields> {
    let Some(value) = characters(&t.value, ctx.page) else {
        notes.add("text in a code page that is not read");
        return None;
    };
    let style = match ctx.styles.get(&t.style) {
        None => Ref::Unresolved(hex(t.style)),
        Some(n) => match characters(n, ctx.page) {
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
pub(super) fn named(
    table: &BTreeMap<u64, TextValue>,
    handle: Option<u64>,
    page: CodePage,
) -> Ref<String> {
    match handle {
        None => Ref::Absent,
        Some(h) => match table.get(&h).and_then(|n| characters(n, page)) {
            Some(n) => Ref::Resolved(n),
            None => Ref::Unresolved(hex(h)),
        },
    }
}

/// A polyface mesh's faces as wireframe edges, face by face in the order of the chain: each
/// face's corners in order and back to the first. An index is 1-based, negative for an edge
/// drawn invisible (the sign says nothing else and is dropped) and 0 for an unused corner; a
/// face with fewer than two corners has no edge. An edge to a position the chain does not hold
/// cannot be drawn: it is counted, not drawn.
fn pface_wireframe(positions: &[Point3D], faces: &[[i16; 4]]) -> (Vec<[Point3D; 2]>, usize) {
    let mut edges = Vec::new();
    let mut skipped = 0;
    for face in faces {
        let corners: Vec<usize> = face
            .iter()
            .filter_map(|&i| usize::from(i.unsigned_abs()).checked_sub(1))
            .collect();
        if corners.len() < 2 {
            continue;
        }
        for w in 0..corners.len() {
            let (a, b) = (corners[w], corners[(w + 1) % corners.len()]);
            match (positions.get(a), positions.get(b)) {
                (Some(&pa), Some(&pb)) => edges.push([pa, pb]),
                _ => skipped += 1,
            }
        }
    }
    (edges, skipped)
}

/// A polygon mesh's grid lines in the order the model states: `m` rows of `n` vertices,
/// stored row by row (vertex `i * n + j` is row `i`, column `j`); first the edges
/// `(i, j)-(i + 1, j)` row by row, then `(i, j)-(i, j + 1)` row by row, the closing edges
/// included where the grid wraps. Unless exactly `m * n` vertices are held, no grid shape is
/// guessed: every edge the definition implies is reported as skipped.
fn mesh_wireframe(
    positions: &[Point3D],
    m: usize,
    n: usize,
    closed_m: bool,
    closed_n: bool,
) -> (Vec<[Point3D; 2]>, usize) {
    let rows_joined = if closed_m { m } else { m.saturating_sub(1) };
    let columns_joined = if closed_n { n } else { n.saturating_sub(1) };
    let edge_count = rows_joined
        .saturating_mul(n)
        .saturating_add(m.saturating_mul(columns_joined));
    if positions.is_empty() || positions.len() != m.saturating_mul(n) {
        return (Vec::new(), edge_count);
    }
    let at = |i: usize, j: usize| positions[i * n + j];
    let mut edges = Vec::with_capacity(edge_count);
    for i in 0..rows_joined {
        for j in 0..n {
            edges.push([at(i, j), at((i + 1) % m, j)]);
        }
    }
    for i in 0..m {
        for j in 0..columns_joined {
            edges.push([at(i, j), at(i, (j + 1) % n)]);
        }
    }
    (edges, 0)
}

/// The handles of the objects an entity owns (its vertices): all of them from
/// R2004; R2000 names the first and the last and links the rest.
pub(super) fn owned(ctx: &Context<'_>, handles: &[u64], notes: &mut Notes) -> Vec<u64> {
    match (ctx.version, handles.first(), handles.last()) {
        (Version::R2000, Some(first), Some(last)) => chain(ctx, *first, *last, notes),
        _ => handles.to_vec(),
    }
}

pub(super) fn attribute_flags(bits: u8) -> AttributeFlags {
    AttributeFlags {
        invisible: bits & 1 != 0,
        constant: bits & 2 != 0,
        verify: bits & 4 != 0,
        preset: bits & 8 != 0,
    }
}

/// A tangent the file states, or `None` for the zero vector (which states none).
pub(super) fn tangent(t: Option<[f64; 3]>) -> Option<Point3D> {
    t.filter(|v| v.iter().any(|c| *c != 0.0)).map(point)
}

/// The model entity for `entity`, or `None` for a kind this reader does not
/// convert yet (or a text it cannot turn into characters, which is reported).
pub(super) fn convert_entity(
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
            let Some(tag) = characters(&a.tag, ctx.page) else {
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
            let Some(text) = characters(&m.value, ctx.page) else {
                notes.add("text in a code page that is not read");
                return None;
            };
            let style_name = match ctx.styles.get(&m.style) {
                None => Ref::Unresolved(hex(m.style)),
                Some(n) => match characters(n, ctx.page) {
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
        Geometry::PolylinePface(p) => {
            let chain = owned(ctx, &p.vertices, notes);
            let positions: Vec<Point3D> = chain
                .iter()
                .filter_map(|h| ctx.vertices_3d.get(h))
                .map(|v| point(v.point))
                .collect();
            let faces: Vec<[i16; 4]> = chain
                .iter()
                .filter_map(|h| ctx.faces.get(h))
                .map(|f| f.indices)
                .collect();
            let (wireframe_edges, skipped_edges) = pface_wireframe(&positions, &faces);
            Entity::PolylinePFace(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        Geometry::PolylineMesh(p) => {
            let positions: Vec<Point3D> = owned(ctx, &p.vertices, notes)
                .iter()
                .filter_map(|h| ctx.vertices_3d.get(h))
                .map(|v| point(v.point))
                .collect();
            let count = |n: i16| usize::try_from(n).unwrap_or(0);
            let (wireframe_edges, skipped_edges) = mesh_wireframe(
                &positions,
                count(p.m_count),
                count(p.n_count),
                p.flags & 1 != 0,
                p.flags & 32 != 0,
            );
            Entity::PolylineMesh(Solid3DEntity {
                common,
                wireframe_edges,
                skipped_edges,
            })
        }
        Geometry::Tolerance(t) => {
            let Some(text_value) = characters(&t.text, ctx.page) else {
                notes.add("text in a code page that is not read");
                return None;
            };
            Entity::Tolerance(ToleranceEntity {
                common,
                insertion_point: point(t.insertion),
                text_height: None,
                text_value,
                direction: Some(point(t.x_direction)),
                style_name: named(ctx.dim_styles, t.style, ctx.page),
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
            mlinestyle_name: named(ctx.mline_styles, m.style, ctx.page),
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
            let text_override = match characters(&d.user_text, ctx.page) {
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
                    entities::DimensionKind::ArcLength => ModelDimensionKind::ArcLength,
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
                    // The record stores the first leader point whether or not the
                    // dimension has a leader, and the DXF twin writes the same point.
                    arc: match &d.arc {
                        Some(a) => Some(point(a.leader_point1)),
                        None => d.point16.map(at_elevation),
                    },
                },
                rotation: d.rotation.unwrap_or(0.0),
                text_rotation: d.text_rotation,
                style_name: named(ctx.dim_styles, d.style, ctx.page),
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
            let gradient = match &h.gradient {
                None => None,
                Some(g) => match hatch_gradient(g, ctx.page) {
                    Some(g) => Some(g),
                    None => {
                        notes.add(
                            "hatch with a gradient that states no colour entity not converted",
                        );
                        return None;
                    }
                },
            };
            Entity::Hatch(HatchEntity {
                common,
                boundary_paths: h.paths.iter().map(hatch_path).collect(),
                solid_fill: h.solid_fill,
                gradient,
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
            let boundary = image_boundary(i);
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
                boundary,
            })
        }
        Geometry::Wipeout(i) => Entity::Wipeout(WipeoutEntity {
            common,
            boundary: image_boundary(i),
        }),
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
            style_name: named(ctx.dim_styles, l.style, ctx.page),
            style_overrides: Some(style_overrides(entity, ctx)),
        }),
        // An entity of a kind this reader does not read keeps its common data and
        // the name the file gives it, as the model asks.
        Geometry::Unread(name) => Entity::Unknown {
            common,
            type_name: name.clone(),
        },
        Geometry::MultiLeader(m) => {
            // A root with no content has no last leader line point, and one the file
            // marks without a dog-leg has none; a line of no point is no line.
            let kept: Vec<&crate::multileader::MultiLeaderLine> = m
                .roots
                .iter()
                .flat_map(|root| root.lines.iter())
                .filter(|line| !line.points.is_empty())
                .collect();
            let leaders = m
                .roots
                .iter()
                .map(|root| uncad_model::model::LeaderRoot {
                    lines: root
                        .lines
                        .iter()
                        .filter(|line| !line.points.is_empty())
                        .map(|line| line.points.iter().copied().map(point).collect())
                        .collect(),
                    last_point: root.content_valid.then(|| point(root.connection_point)),
                    dogleg: root.dog_leg_valid.then(|| uncad_model::model::Dogleg {
                        direction: point(root.direction),
                        length: root.landing_distance,
                    }),
                })
                .collect();
            let style_type = m
                .style
                .and_then(|h| ctx.mleader_styles.get(&h))
                .map(|t| i64::from(*t));
            let line_type = uncad_model::model::LeaderLineType::resolve(
                Some(m.override_flags as u32),
                Some(i64::from(m.line_type)),
                style_type,
                kept.iter()
                    .map(|l| (Some(l.override_flags as u32), Some(i64::from(l.line_type)))),
            );
            let arrow_size = uncad_model::model::MultiLeaderEntity::resolve_arrow_size(
                Some(m.arrow_head_size),
                kept.iter()
                    .map(|l| (Some(l.override_flags as u32), Some(l.arrow_size))),
            );
            let content = match &m.content {
                Some(crate::multileader::MultiLeaderContent::Text(t)) => {
                    Some(uncad_model::model::MultiLeaderContent::MText(
                        uncad_model::model::MultiLeaderText {
                            text: t.text.clone(),
                            style_name: named(ctx.styles, t.style, ctx.page),
                            location: point(t.location),
                            direction: point(t.direction),
                            extrusion: point(t.normal),
                            height: m.text_height,
                            line_spacing_factor: t.line_spacing_factor,
                            rotation: t.rotation,
                            width: t.width,
                            scale: m.scale,
                            attachment: uncad_model::model::MTextAttachment::from_code(i64::from(
                                t.alignment,
                            )),
                        },
                    ))
                }
                Some(crate::multileader::MultiLeaderContent::Block(b)) => {
                    Some(uncad_model::model::MultiLeaderContent::Block(
                        uncad_model::model::MultiLeaderBlock {
                            block_name: match b.record {
                                None => Ref::Absent,
                                Some(h) => match ctx.block_names.get(&h) {
                                    Some(n) => Ref::Resolved(n.clone()),
                                    None => Ref::Unresolved(hex(h)),
                                },
                            },
                            location: point(b.location),
                            scale: point(b.scale),
                            rotation: b.rotation,
                            extrusion: point(b.normal),
                        },
                    ))
                }
                None => None,
            };
            Entity::MultiLeader(uncad_model::model::MultiLeaderEntity {
                common,
                leaders,
                line_type,
                arrow_size,
                content,
            })
        }
        _ => return None,
    })
}

/// The clip boundary of an image (or the outline of a wipeout) in the drawing.
///
/// The file states it in pixels (the centre of a pixel is half a pixel in, and
/// rows count down from the top); the model states it in the drawing, and a
/// rectangle as its four corners.
pub(super) fn image_boundary(i: &entities::Image) -> Vec<Point2D> {
    let to_world = |p: [f64; 2]| Point2D {
        x: i.insertion[0] + i.u_vector[0] * (p[0] + 0.5) + i.v_vector[0] * (i.size[1] - 0.5 - p[1]),
        y: i.insertion[1] + i.u_vector[1] * (p[0] + 0.5) + i.v_vector[1] * (i.size[1] - 0.5 - p[1]),
    };
    let corners: Vec<[f64; 2]> = match (i.clip_type, i.clip_boundary.as_slice()) {
        (1, [a, b]) => vec![[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]]],
        (_, points) => points.to_vec(),
    };
    corners.into_iter().map(to_world).collect()
}

/// A gradient fill in the model's words, or `None` when the file states no colour.
///
/// A colour long names a palette colour (`0xC3`) or is a 24-bit RGB value; a
/// single-colour gradient fades its colour by the tint, and a two-colour one runs
/// from the first stop to the last.
pub(super) fn hatch_gradient(
    g: &entities::HatchGradient,
    page: CodePage,
) -> Option<ModelHatchGradient> {
    let rgb = |packed: u32| -> Option<u32> {
        if packed >> 24 == 0xC3 {
            uncad_model::color::aci_to_rgb((packed & 0xFFFF) as u16)
        } else {
            Some(packed & 0xFF_FFFF)
        }
    };
    let mut stops: Vec<(f64, u32)> = g
        .colors
        .iter()
        .filter_map(|(p, c)| rgb(*c).map(|c| (*p, c)))
        .collect();
    let (color1, color2, tint) = if g.single_color {
        let (_, c) = *stops.first()?;
        (c, None, g.tint)
    } else if stops.len() >= 2 {
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        (stops[0].1, Some(stops[stops.len() - 1].1), 0.0)
    } else {
        let (_, c) = *stops.first()?;
        (c, Some(c), 0.0)
    };
    let name = characters(&g.name, page).map(|n| n.to_ascii_uppercase());
    Some(ModelHatchGradient {
        is_radial: matches!(name.as_deref(), Some("SPHERICAL" | "HEMISPHERICAL")),
        angle: g.angle,
        color1,
        color2,
        tint,
    })
}

/// A hatch boundary in the model's words.
pub(super) fn hatch_path(path: &entities::HatchPath) -> HatchBoundaryPath {
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
pub(super) fn convert_common(entity: &entities::Entity, ctx: &Context<'_>) -> EntityCommon {
    let (version, layers, linetypes, page) = (ctx.version, ctx.layers, ctx.linetypes, ctx.page);
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
            Some(h) => match linetypes.get(&h).and_then(|n| characters(n, page)) {
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
