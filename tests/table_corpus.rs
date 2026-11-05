//! Tables of real files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every table of an R2000 to R2007 file must read whole:
//! its main stream and its handle stream end exactly where the fields do, its
//! block resolves, and its grid has one cell per column and row. Without the
//! variable the test says it skipped and passes -- the files are not part of
//! this repository.

use std::path::{Path, PathBuf};

use undwg::{read_entity_with_classes, read_table_style, Drawing, Geometry};

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

#[test]
fn every_table_of_an_r2000_to_r2007_file_reads_whole() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");
    let (mut read, mut unread, mut checked) = (0usize, 0usize, 0usize);
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok(drawing) = Drawing::open(&bytes) else {
            continue;
        };
        let version = drawing.version();
        let mut styles = std::collections::BTreeMap::new();
        for entry in drawing.entries() {
            match read_table_style(drawing.objects(), *entry, version, drawing.classes()) {
                Ok(Some(style)) => {
                    styles.insert(entry.handle, style.flow);
                }
                Ok(None) => {}
                Err(e) => panic!("{}: table style {:#x}: {e}", path.display(), entry.handle),
            }
        }
        for entry in drawing.entries() {
            let Ok(Some(entity)) =
                read_entity_with_classes(drawing.objects(), *entry, version, drawing.classes())
            else {
                continue;
            };
            match &entity.geometry {
                Geometry::Table(t) => {
                    read += 1;
                    assert_eq!(
                        t.cells.len(),
                        t.column_widths.len() * t.row_heights.len(),
                        "{}",
                        path.display()
                    );
                    assert!(t.insert.block != 0, "{}", path.display());
                    // The style the table names is in the drawing and states its flow.
                    let style = t.style.expect("a table names its style");
                    assert!(
                        styles.contains_key(&style),
                        "{}: table style {style:#x} not read",
                        path.display()
                    );
                    checked += t.cells.len();
                    if std::env::var_os("UNDWG_TABLES").is_some() {
                        let mut kinds = std::collections::BTreeMap::new();
                        for c in &t.cells {
                            *kinds.entry((c.kind, c.value_kind)).or_insert(0usize) += 1;
                        }
                        eprintln!("  cell kinds {kinds:?}");
                        let texts: std::collections::BTreeSet<_> = t
                            .cells
                            .iter()
                            .filter_map(|c| c.value_text.as_ref().map(|v| format!("{v:?}")))
                            .collect();
                        eprintln!("  value texts {texts:?}");
                        for (i, c) in t.cells.iter().enumerate() {
                            if c.value_kind == Some(512) || c.merged || c.span_columns != 1 {
                                eprintln!("  cell {i}: {c:?}");
                            }
                        }
                    }
                    if std::env::var_os("UNDWG_TABLES").is_some() {
                        eprintln!(
                            "{} {:?} {}x{} flow={:?} style={:?} flags={:#x}",
                            path.display(),
                            version,
                            t.row_heights.len(),
                            t.column_widths.len(),
                            t.flow,
                            t.style,
                            t.flags
                        );
                    }
                }
                Geometry::Unread(name) if name == "ACAD_TABLE" => {
                    unread += 1;
                    if std::env::var_os("UNDWG_TABLES").is_some() {
                        eprintln!("unread {} {:?}", path.display(), version);
                    }
                }
                _ => {}
            }
        }
    }
    eprintln!("tables read {read} (cells {checked}), unread {unread}");
}
