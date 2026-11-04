//! Lines, circles, arcs and points of real files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every line, circle, arc, point, lightweight polyline, ellipse, solid, 3D face and block insertion whose frame is intact must
//! read, state the handle the object map gives, and hold finite numbers (and
//! a non-negative radius). With `UNDWG_DUMP` set to a file name, every entity
//! read is also written there, one per line, so that it can be compared with
//! another reader's output. Without `UNDWG_CORPUS` the test says it skipped
//! and passes -- the files are not part of this repository.

use std::io::Write;
use std::path::{Path, PathBuf};

use undwg::{
    read_block_name, read_classes, read_dimstyle_name, read_directory, read_entity,
    read_entity_with_classes, read_header, read_layer, read_linetype_name, read_object_frame,
    read_object_map, read_section, FileHeader, Geometry, HatchEdge, TextValue, Version,
};

/// A text as the dump writes it: `U:` and the hex of its UTF-8 for Unicode
/// text, `A:` and the hex of its bytes for code-page text.
fn text_hex(t: &TextValue) -> String {
    match t {
        TextValue::Ansi(b) => format!(
            "A:{}",
            b.iter().map(|c| format!("{c:02x}")).collect::<String>()
        ),
        TextValue::Unicode(s) => format!(
            "U:{}",
            s.as_bytes()
                .iter()
                .map(|c| format!("{c:02x}"))
                .collect::<String>()
        ),
    }
}

fn dwg_files(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            dwg_files(&path, into);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
        {
            into.push(path);
        }
    }
}

/// The fields of a text as the dump writes them: elevation, insertion,
/// alignment flag and point, extrusion, oblique, rotation, height, width
/// factor, (horizontal, vertical), generation, text.
fn text_fields(t: &undwg::Text) -> String {
    let a = t.alignment.unwrap_or([0.0, 0.0]);
    format!(
        "{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
        t.elevation,
        t.insertion,
        i32::from(t.alignment.is_some()),
        a,
        t.extrusion,
        t.oblique,
        t.rotation,
        t.height,
        t.width_factor,
        (t.horizontal, t.vertical),
        t.generation,
        text_hex(&t.value)
    )
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

#[test]
fn every_real_line_circle_and_arc_reads() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");
    let mut dump = std::env::var_os("UNDWG_DUMP").map(|p| {
        std::io::BufWriter::new(std::fs::File::create(p).expect("UNDWG_DUMP is writable"))
    });

    let (
        mut files_read,
        mut lines,
        mut circles,
        mut arcs,
        mut points,
        mut polylines,
        mut inserts,
        mut ellipses,
        mut solids,
        mut faces,
        mut texts,
        mut attributes,
        mut mtexts,
        mut dimensions,
        mut splines,
        mut polylines_2d,
        mut polylines_3d,
        mut vertices,
        mut others,
        mut hatches,
        mut failed,
    ) = (
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0usize,
    );
    let mut named_blocks = 0usize;
    let mut references_checked = 0usize;
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (version, handles, objects): (Version, Vec<u8>, Vec<u8>) = match read_header(&bytes) {
            Ok(FileHeader::R2004(version, header)) => {
                let dir = read_directory(&bytes, &header)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let get = |name: &str| {
                    read_section(&bytes, &dir, name)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                        .unwrap_or_else(|| panic!("{}: no {name}", path.display()))
                };
                (version, get("AcDb:Handles"), get("AcDb:AcDbObjects"))
            }
            Ok(FileHeader::R2000(header)) => {
                let handles = header
                    .section(&bytes, 2)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .unwrap_or_else(|| panic!("{}: no object map", path.display()))
                    .to_vec();
                (Version::R2000, handles, bytes.clone())
            }
            _ => continue,
        };
        let classes_section: Option<Vec<u8>> = match read_header(&bytes) {
            Ok(FileHeader::R2004(_, header)) => {
                let dir = read_directory(&bytes, &header).expect("directory");
                read_section(&bytes, &dir, "AcDb:Classes").ok().flatten()
            }
            Ok(FileHeader::R2000(header)) => {
                header.section(&bytes, 1).ok().flatten().map(<[u8]>::to_vec)
            }
            _ => None,
        };
        let classes = classes_section
            .map(|section| read_classes(&section, version).expect("classes"))
            .unwrap_or_default();
        let is_hatch = |code: u16| {
            code == 78
                || (code >= 500
                    && classes
                        .iter()
                        .any(|c| i32::from(c.number) == i32::from(code) && c.dxf_name == "HATCH"))
        };
        let entries = read_object_map(&handles)
            .unwrap_or_else(|e| panic!("{}: object map: {e}", path.display()));
        files_read += 1;
        // The type of every object whose frame holds, to check what the
        // references of an entity point at.
        let types: std::collections::BTreeMap<u64, u16> = entries
            .iter()
            .filter(|e| e.offset < objects.len() as u64)
            .filter_map(|e| {
                read_object_frame(&objects, *e, version)
                    .ok()
                    .map(|f| (e.handle, f.type_code))
            })
            .collect();
        // The name of every block header, to name what an insertion inserts.
        let block_names: std::collections::BTreeMap<u64, TextValue> = entries
            .iter()
            .filter(|e| e.offset < objects.len() as u64)
            .filter_map(|e| match read_block_name(&objects, *e, version) {
                Ok(Some(name)) => Some((e.handle, name)),
                Ok(None) => None,
                Err(err) => panic!("{}: block header {:#x}: {err}", path.display(), e.handle),
            })
            .collect();
        // The name of every dimension style.
        let style_names: std::collections::BTreeMap<u64, TextValue> = entries
            .iter()
            .filter(|e| e.offset < objects.len() as u64)
            .filter_map(|e| match read_dimstyle_name(&objects, *e, version) {
                Ok(Some(name)) => Some((e.handle, name)),
                Ok(None) => None,
                Err(err) => panic!("{}: dimension style {:#x}: {err}", path.display(), e.handle),
            })
            .collect();
        // Every layer and every linetype name; the layers go to the dump once per file.
        let layers: std::collections::BTreeMap<u64, undwg::Layer> = entries
            .iter()
            .filter(|e| e.offset < objects.len() as u64)
            .filter_map(|e| match read_layer(&objects, *e, version) {
                Ok(Some(layer)) => Some((e.handle, layer)),
                Ok(None) => None,
                Err(err) => panic!("{}: layer {:#x}: {err}", path.display(), e.handle),
            })
            .collect();
        let linetype_names: std::collections::BTreeMap<u64, TextValue> = entries
            .iter()
            .filter(|e| e.offset < objects.len() as u64)
            .filter_map(|e| match read_linetype_name(&objects, *e, version) {
                Ok(Some(name)) => Some((e.handle, name)),
                Ok(None) => None,
                Err(err) => panic!("{}: linetype {:#x}: {err}", path.display(), e.handle),
            })
            .collect();
        for layer in layers.values() {
            // A layer's references name objects of the right kind.
            if let Some(owner) = layer.owner {
                assert_eq!(
                    types.get(&owner),
                    Some(&50),
                    "{}: layer owner",
                    path.display()
                );
            }
            if let Some(lt) = layer.linetype {
                assert_eq!(
                    types.get(&lt),
                    Some(&57),
                    "{}: layer linetype",
                    path.display()
                );
            }
            if let Some(out) = dump.as_mut() {
                let linetype = layer
                    .linetype
                    .and_then(|h| linetype_names.get(&h))
                    .map(text_hex)
                    .unwrap_or_default();
                writeln!(
                    out,
                    "{}\t{:X}\tLAYER\t{:?}\t{}\t{}\t{}\t{:?}",
                    path.display(),
                    layer.handle,
                    text_hex(&layer.name),
                    layer.flags,
                    layer.color.method,
                    layer.color.value,
                    linetype
                )
                .unwrap();
            }
        }
        // The vertices of 2D and 3D polylines, by handle, with the owner each states.
        let mut vertices_2d = std::collections::BTreeMap::new();
        let mut vertices_3d = std::collections::BTreeMap::new();
        for e in entries.iter().filter(|e| e.offset < objects.len() as u64) {
            if let Ok(Some(vertex)) = read_entity(&objects, *e, version) {
                match vertex.geometry {
                    Geometry::Vertex2d(v) => {
                        vertices_2d.insert(e.handle, (vertex.references.owner, v));
                    }
                    Geometry::Vertex3d(v) => {
                        vertices_3d.insert(e.handle, (vertex.references.owner, v));
                    }
                    _ => {}
                }
            }
        }
        // The handles of the vertices a polyline owns: all of them from R2004; in
        // R2000 the first and the last, and the vertices of that owner between.
        let owned_vertices =
            |polyline: u64, listed: &[u64], owners: Vec<(u64, Option<u64>)>| -> Vec<u64> {
                if version >= Version::R2004 {
                    listed.to_vec()
                } else {
                    let (first, last) = (listed[0], listed[listed.len() - 1]);
                    owners
                        .into_iter()
                        .filter(|(h, o)| *h >= first && *h <= last && *o == Some(polyline))
                        .map(|(h, _)| h)
                        .collect()
                }
            };
        for entry in &entries {
            if entry.offset >= objects.len() as u64 {
                continue;
            }
            // Only objects whose frame holds are candidates.
            let Ok(frame) = read_object_frame(&objects, *entry, version) else {
                continue;
            };
            if !is_hatch(frame.type_code)
                && !matches!(frame.type_code, 1..=3 | 7 | 10 | 11 | 15 | 16 | 17..=19 | 20..=26 | 27 | 28 | 31 | 32 | 35 | 36 | 40 | 41 | 44 | 46 | 77)
            {
                continue;
            }
            let entity = match read_entity_with_classes(&objects, *entry, version, &classes) {
                Ok(Some(e)) => e,
                Ok(None) => panic!(
                    "{}: type {} gave no entity",
                    path.display(),
                    frame.type_code
                ),
                Err(e) => {
                    failed += 1;
                    if failed <= 8 {
                        eprintln!(
                            "unreadable entity: {}: {:#x}: {e}",
                            path.display(),
                            entry.handle
                        );
                    }
                    continue;
                }
            };
            // What the entity points at must be an object of the right kind:
            // a layer, a block header, a dictionary, a linetype.
            let refs = &entity.references;
            let expect = |handle: u64, code: u16, what: &str| {
                assert_eq!(
                    types.get(&handle),
                    Some(&code),
                    "{}: {:#x}: {what} {handle:#x}",
                    path.display(),
                    entry.handle
                );
            };
            expect(refs.layer, 51, "layer");
            assert!(
                layers.contains_key(&refs.layer),
                "{}: {:#x}: the layer has no entry",
                path.display(),
                entry.handle
            );
            if let Some(owner) = refs.owner {
                // An attribute belongs to the insertion it follows.
                match entity.geometry {
                    Geometry::Attrib(_) => expect(owner, 7, "owner"),
                    // A vertex belongs to the polyline it is part of.
                    Geometry::Vertex2d(_) => expect(owner, 15, "owner"),
                    Geometry::Vertex3d(_) => expect(owner, 16, "owner"),
                    _ => expect(owner, 49, "owner"),
                }
            }
            if let Some(x) = refs.xdictionary {
                expect(x, 42, "extension dictionary");
            }
            if let Some(l) = refs.linetype {
                expect(l, 57, "linetype");
            }
            if let Geometry::Insert(i) = &entity.geometry {
                expect(i.block, 49, "inserted block");
                assert!(
                    block_names.contains_key(&i.block),
                    "{}: {:#x}: the inserted block has no name",
                    path.display(),
                    entry.handle
                );
            }
            if let Geometry::Text(t) = &entity.geometry {
                expect(t.style, 53, "text style");
            }
            if let Geometry::Tolerance(t) = &entity.geometry {
                if let Some(style) = t.style {
                    expect(style, 69, "tolerance style");
                }
            }
            if let Geometry::Mtext(m) = &entity.geometry {
                expect(m.style, 53, "multiline text style");
            }
            if let Geometry::Dimension(d) = &entity.geometry {
                if let Some(style) = d.style {
                    expect(style, 69, "dimension style");
                }
                if let Some(block) = d.block {
                    expect(block, 49, "dimension block");
                }
            }
            if let Geometry::Attrib(a) | Geometry::Attdef(a) = &entity.geometry {
                expect(a.text.style, 53, "attribute text style");
            }
            references_checked += 1;
            let name = match &entity.geometry {
                Geometry::Line(l) => {
                    lines += 1;
                    assert!(
                        finite(&l.start) && finite(&l.end) && l.thickness.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!("LINE\t{:?}\t{:?}", l.start, l.end)
                }
                Geometry::Circle(c) => {
                    circles += 1;
                    assert!(
                        finite(&c.center) && c.radius.is_finite() && c.radius >= 0.0,
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!("CIRCLE\t{:?}\t{:?}", c.center, c.radius)
                }
                Geometry::Arc(a) => {
                    arcs += 1;
                    assert!(
                        finite(&a.center)
                            && a.radius.is_finite()
                            && a.radius >= 0.0
                            && a.start_angle.is_finite()
                            && a.end_angle.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!(
                        "ARC\t{:?}\t{:?}\t{:?}\t{:?}",
                        a.center, a.radius, a.start_angle, a.end_angle
                    )
                }
                Geometry::Point(p) => {
                    points += 1;
                    assert!(
                        finite(&p.position)
                            && p.thickness.is_finite()
                            && p.x_axis_angle.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!("POINT	{:?}", p.position)
                }
                Geometry::LwPolyline(p) => {
                    polylines += 1;
                    assert!(
                        p.elevation.is_finite()
                            && p.const_width.is_finite()
                            && p.thickness.is_finite()
                            && finite(&p.normal)
                            && p.vertices.iter().all(|v| {
                                v.point.iter().all(|c| c.is_finite())
                                    && v.bulge.is_finite()
                                    && v.start_width.is_finite()
                                    && v.end_width.is_finite()
                            }),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    // closed, elevation, constant width, normal, then x y bulge start-width end-width per vertex
                    let flat: Vec<f64> = p
                        .vertices
                        .iter()
                        .flat_map(|v| [v.point[0], v.point[1], v.bulge, v.start_width, v.end_width])
                        .collect();
                    format!(
                        "LWPOLYLINE\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        i32::from(p.closed),
                        p.elevation,
                        p.const_width,
                        p.normal,
                        flat
                    )
                }
                Geometry::Insert(i) => {
                    inserts += 1;
                    assert!(
                        finite(&i.position)
                            && finite(&i.scale)
                            && i.rotation.is_finite()
                            && finite(&i.extrusion),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    named_blocks += 1;
                    format!(
                        "INSERT\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        i.position,
                        i.scale,
                        i.rotation,
                        i.extrusion,
                        text_hex(&block_names[&i.block])
                    )
                }
                Geometry::Text(t) => {
                    texts += 1;
                    assert!(
                        t.elevation.is_finite()
                            && finite(&t.insertion)
                            && finite(&t.extrusion)
                            && t.height.is_finite()
                            && t.rotation.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!("TEXT\t{}", text_fields(t))
                }
                Geometry::Dimension(d) => {
                    dimensions += 1;
                    // A dimension may have no block (it is drawn when it is shown) or no style.
                    let block = d.block.map(|b| &block_names[&b]);
                    let style = d.style.map(|b| &style_names[&b]);
                    let name = |t: Option<&TextValue>| t.map_or(String::new(), text_hex);
                    assert!(
                        finite(&d.definition_point)
                            && finite(&d.extrusion)
                            && d.measurement.is_finite()
                            && d.text_rotation.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    // kind, block name, style name, user text, 10-point, text middle, 13, 14, 15 and 16-points (empty when
                    // the kind has none), dimension rotation, text rotation, measurement, ordinate flags
                    let opt3 = |p: Option<[f64; 3]>| p.map(|p| p.to_vec()).unwrap_or_default();
                    format!(
                        "DIMENSION\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        format!("{:?}", d.kind),
                        name(block),
                        name(style),
                        text_hex(&d.user_text),
                        d.definition_point,
                        d.text_midpoint,
                        opt3(d.point13),
                        opt3(d.point14),
                        opt3(d.point15),
                        d.point16.map(|p| p.to_vec()).unwrap_or_default(),
                        d.rotation.unwrap_or(0.0),
                        d.text_rotation,
                        d.measurement,
                        d.ordinate_flags.map_or(-1, i32::from)
                    )
                }
                Geometry::Polyline2d(pl) => {
                    polylines_2d += 1;
                    let owners = vertices_2d.iter().map(|(h, (o, _))| (*h, *o)).collect();
                    let handles = owned_vertices(entry.handle, &pl.vertices, owners);
                    // closed, elevation, start and end width, extrusion, then x y bulge start-width end-width per vertex
                    let flat: Vec<f64> = handles
                        .iter()
                        .flat_map(|h| {
                            let v = &vertices_2d[h].1;
                            [v.point[0], v.point[1], v.bulge, v.start_width, v.end_width]
                        })
                        .collect();
                    assert!(
                        flat.iter().all(|v| v.is_finite()),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!(
                        "POLYLINE2D\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        i32::from(pl.flags & 1 != 0),
                        pl.elevation,
                        pl.start_width,
                        pl.end_width,
                        pl.extrusion,
                        pl.curve_type,
                        flat
                    )
                }
                Geometry::Polyline3d(pl) => {
                    polylines_3d += 1;
                    let owners = vertices_3d.iter().map(|(h, (o, _))| (*h, *o)).collect();
                    let handles = owned_vertices(entry.handle, &pl.vertices, owners);
                    let flat: Vec<f64> = handles
                        .iter()
                        .flat_map(|h| vertices_3d[h].1.point)
                        .collect();
                    assert!(
                        flat.iter().all(|v| v.is_finite()),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!(
                        "POLYLINE3D\t{:?}\t{:?}\t{:?}",
                        i32::from(pl.closed_flags & 1 != 0),
                        pl.spline_flags,
                        flat
                    )
                }
                Geometry::Hatch(h) => {
                    hatches += 1;
                    // A token string: the paths, edge by edge, then the fill and the pattern.
                    let mut t: Vec<String> = Vec::new();
                    for path in &h.paths {
                        if path.flags & 2 != 0 {
                            t.push(format!(
                                "P{} {}",
                                i32::from(path.closed),
                                path.polyline.len()
                            ));
                            for (p, b) in &path.polyline {
                                t.push(format!("{:?} {:?} {:?}", p[0], p[1], b));
                            }
                        } else {
                            t.push(format!("E{}", path.edges.len()));
                            for e in &path.edges {
                                t.push(match e {
                                    HatchEdge::Line { start, end } => {
                                        format!(
                                            "L {:?} {:?} {:?} {:?}",
                                            start[0], start[1], end[0], end[1]
                                        )
                                    }
                                    HatchEdge::Arc {
                                        center,
                                        radius,
                                        start_angle,
                                        end_angle,
                                        counter_clockwise,
                                    } => format!(
                                        "A {:?} {:?} {:?} {:?} {:?} {}",
                                        center[0],
                                        center[1],
                                        radius,
                                        start_angle,
                                        end_angle,
                                        i32::from(*counter_clockwise)
                                    ),
                                    HatchEdge::Ellipse {
                                        center,
                                        major_axis,
                                        ratio,
                                        start_angle,
                                        end_angle,
                                        counter_clockwise,
                                    } => format!(
                                        "N {:?} {:?} {:?} {:?} {:?} {:?} {:?} {}",
                                        center[0],
                                        center[1],
                                        major_axis[0],
                                        major_axis[1],
                                        ratio,
                                        start_angle,
                                        end_angle,
                                        i32::from(*counter_clockwise)
                                    ),
                                    HatchEdge::Spline {
                                        degree,
                                        rational,
                                        periodic,
                                        knots,
                                        control_points,
                                        ..
                                    } => format!(
                                        "S {degree} {} {} {:?} {:?}",
                                        i32::from(*rational),
                                        i32::from(*periodic),
                                        knots,
                                        control_points
                                            .iter()
                                            .map(|(p, w)| (p[0], p[1], *w))
                                            .collect::<Vec<_>>()
                                    ),
                                    _ => "?".to_string(),
                                });
                            }
                        }
                    }
                    let lines: Vec<String> = h
                        .pattern_lines
                        .iter()
                        .map(|l| {
                            format!(
                                "{:?} {:?} {:?} {:?}",
                                l.angle, l.base_point, l.offset, l.dashes
                            )
                        })
                        .collect();
                    format!(
                        "HATCH\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        h.elevation,
                        i32::from(h.solid_fill),
                        h.style,
                        h.extrusion,
                        t.join(" | "),
                        lines.join(" | "),
                        text_hex(&h.name)
                    )
                }
                Geometry::Trace(d) => {
                    others += 1;
                    assert!(
                        d.corners.iter().all(|c| c.iter().all(|v| v.is_finite())),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let flat: Vec<f64> = d.corners.iter().flatten().copied().collect();
                    format!("TRACE\t{:?}\t{:?}\t{:?}", d.elevation, flat, d.extrusion)
                }
                Geometry::Ray(l) | Geometry::Xline(l) => {
                    others += 1;
                    assert!(
                        finite(&l.point) && finite(&l.vector),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let kind = if matches!(entity.geometry, Geometry::Ray(_)) {
                        "RAY"
                    } else {
                        "XLINE"
                    };
                    format!("{kind}\t{:?}\t{:?}", l.point, l.vector)
                }
                Geometry::Tolerance(t) => {
                    others += 1;
                    assert!(
                        finite(&t.insertion) && finite(&t.x_direction),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!(
                        "TOLERANCE\t{:?}\t{:?}\t{:?}",
                        t.insertion,
                        t.x_direction,
                        text_hex(&t.text)
                    )
                }
                Geometry::Vertex2d(_) | Geometry::Vertex3d(_) => {
                    vertices += 1;
                    continue;
                }
                Geometry::Spline(sp) => {
                    splines += 1;
                    assert!(
                        sp.knots.iter().all(|k| k.is_finite())
                            && sp.weights.iter().all(|k| k.is_finite())
                            && sp.control_points.iter().all(|p| finite(p))
                            && sp.fit_points.iter().all(|p| finite(p)),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let flat = |v: &[[f64; 3]]| v.iter().flatten().copied().collect::<Vec<f64>>();
                    let tangent = |t: Option<[f64; 3]>| t.map(|t| t.to_vec()).unwrap_or_default();
                    // scenario, degree, closed, periodic, knots, weights, control points, fit points, start and end tangent
                    format!(
                        "SPLINE\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        if sp.fit_data { 2 } else { 1 },
                        sp.degree,
                        i32::from(sp.closed),
                        i32::from(sp.periodic),
                        sp.knots,
                        sp.weights,
                        flat(&sp.control_points),
                        flat(&sp.fit_points),
                        tangent(sp.start_tangent),
                        tangent(sp.end_tangent)
                    )
                }
                Geometry::Mtext(m) => {
                    mtexts += 1;
                    assert!(
                        finite(&m.insertion)
                            && finite(&m.extrusion)
                            && finite(&m.x_axis)
                            && m.text_height.is_finite()
                            && m.rect_width.is_finite()
                            && m.extents_width.is_finite()
                            && m.extents_height.is_finite()
                            && m.line_spacing_factor.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    // insertion, x axis, rectangle width, text height, attachment,
                    // extents width and height, line spacing factor, text
                    format!(
                        "MTEXT\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        m.insertion,
                        m.x_axis,
                        m.rect_width,
                        m.text_height,
                        m.attachment,
                        m.extents_width,
                        m.extents_height,
                        m.line_spacing_factor,
                        text_hex(&m.value)
                    )
                }
                Geometry::Attrib(a) | Geometry::Attdef(a) => {
                    attributes += 1;
                    assert!(
                        a.text.elevation.is_finite()
                            && finite(&a.text.insertion)
                            && a.text.height.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let (kind, prompt) = match &entity.geometry {
                        Geometry::Attdef(_) => ("ATTDEF", a.prompt.as_ref().map(text_hex)),
                        _ => ("ATTRIB", None),
                    };
                    format!(
                        "{kind}\t{}\t{:?}\t{:?}\t{:?}",
                        text_fields(&a.text),
                        text_hex(&a.tag),
                        a.flags,
                        prompt.unwrap_or_default()
                    )
                }
                Geometry::Ellipse(e) => {
                    ellipses += 1;
                    assert!(
                        finite(&e.center)
                            && finite(&e.major_axis)
                            && finite(&e.extrusion)
                            && e.axis_ratio.is_finite()
                            && e.start_angle.is_finite()
                            && e.end_angle.is_finite(),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    format!(
                        "ELLIPSE\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                        e.center,
                        e.major_axis,
                        e.extrusion,
                        e.axis_ratio,
                        e.start_angle,
                        e.end_angle
                    )
                }
                Geometry::Solid(d) => {
                    solids += 1;
                    assert!(
                        d.thickness.is_finite()
                            && d.elevation.is_finite()
                            && d.corners.iter().all(|c| c.iter().all(|v| v.is_finite()))
                            && finite(&d.extrusion),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let flat: Vec<f64> = d.corners.iter().flatten().copied().collect();
                    format!("SOLID\t{:?}\t{:?}\t{:?}", d.elevation, flat, d.extrusion)
                }
                Geometry::Face3d(f) => {
                    faces += 1;
                    assert!(
                        f.corners.iter().all(|c| finite(c)),
                        "{}: {:#x}",
                        path.display(),
                        entry.handle
                    );
                    let flat: Vec<f64> = f.corners.iter().flatten().copied().collect();
                    format!("3DFACE\t{:?}\t{:?}", flat, f.invisible_edges)
                }
                _ => continue,
            };
            if let Some(out) = dump.as_mut() {
                writeln!(out, "{}\t{:X}\t{}", path.display(), entry.handle, name).unwrap();
            }
            if let Some(out) = dump.as_mut() {
                // What every entity states: its layer, colour, linetype, scale,
                // visibility and line weight; an absent value is written as -1.
                let c = &entity.common;
                let layer_name = layers
                    .get(&refs.layer)
                    .map(|l| text_hex(&l.name))
                    .unwrap_or_default();
                let linetype = refs
                    .linetype
                    .and_then(|h| linetype_names.get(&h))
                    .map(text_hex)
                    .unwrap_or_default();
                writeln!(
                    out,
                    "{}\t{:X}\tCOMMON\t{:?}\t{}\t{}\t{}\t{:?}\t{}\t{:?}\t{}\t{}",
                    path.display(),
                    entry.handle,
                    layer_name,
                    c.color,
                    c.rgb.map_or(-1, i64::from),
                    c.transparency.map_or(-1, i64::from),
                    c.linetype_scale,
                    c.linetype_flags,
                    linetype,
                    c.invisible,
                    c.lineweight
                )
                .unwrap();
            }
        }
    }
    eprintln!(
        "{files_read} files: {lines} lines, {circles} circles, {arcs} arcs, {points} points, {polylines} polylines, {inserts} inserts, {ellipses} ellipses, {solids} solids, {faces} 3D faces, {texts} texts, {attributes} attributes and definitions, {mtexts} multiline texts, {dimensions} dimensions, {splines} splines, {polylines_2d} 2D and {polylines_3d} 3D polylines with {vertices} vertices, {others} traces, rays, infinite lines and tolerances, {hatches} hatches read ({named_blocks} insertions with a block name); {failed} unreadable; {references_checked} entities' references point at objects of the right kind"
    );
    assert!(files_read > 0);
    assert_eq!(failed, 0, "entities whose frame holds must read");
}
