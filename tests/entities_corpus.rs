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
    read_block_name, read_classes, read_directory, read_entity, read_header, read_object_frame,
    read_object_map, read_section, FileHeader, Geometry, TextValue, Version,
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
        mut failed,
    ) = (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0usize);
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
        let _ = (read_classes, 0);
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
        for entry in &entries {
            if entry.offset >= objects.len() as u64 {
                continue;
            }
            // Only objects whose frame holds are candidates.
            let Ok(frame) = read_object_frame(&objects, *entry, version) else {
                continue;
            };
            if !matches!(frame.type_code, 1..=3 | 7 | 17..=19 | 27 | 28 | 31 | 35 | 44 | 77) {
                continue;
            }
            let entity = match read_entity(&objects, *entry, version) {
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
            if let Some(owner) = refs.owner {
                // An attribute belongs to the insertion it follows.
                let kind = if matches!(entity.geometry, Geometry::Attrib(_)) {
                    7
                } else {
                    49
                };
                expect(owner, kind, "owner");
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
            if let Geometry::Mtext(m) = &entity.geometry {
                expect(m.style, 53, "multiline text style");
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
        }
    }
    eprintln!(
        "{files_read} files: {lines} lines, {circles} circles, {arcs} arcs, {points} points, {polylines} polylines, {inserts} inserts, {ellipses} ellipses, {solids} solids, {faces} 3D faces, {texts} texts, {attributes} attributes and definitions, {mtexts} multiline texts read ({named_blocks} insertions with a block name); {failed} unreadable; {references_checked} entities' references point at objects of the right kind"
    );
    assert!(files_read > 0);
    assert_eq!(failed, 0, "entities whose frame holds must read");
}
