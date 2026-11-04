//! Whole drawings of real files, read into the entity model.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every R2000, R2004, R2010, R2013 and R2018 file must
//! read into a drawing: the same bytes give the same drawing, every entity
//! names a layer the drawing has, the top level lists exactly what model space
//! and the paper spaces own, and the diagnostics say nothing except which
//! kinds of entity are not converted yet. Without `UNDWG_CORPUS` the test says
//! it skipped and passes -- the files are not part of this repository.
//! (`tools/undwg-oracle/model_diff.py` compares the drawings with another
//! reader's.)

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use uncad_model::{Ref, ToJsonOptions};

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
fn every_real_file_reads_into_a_consistent_drawing() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let (mut read, mut skipped, mut entities) = (0usize, 0usize, 0usize);
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let db = match undwg::read_database(&bytes) {
            Ok(db) => db,
            // R2007 and the versions before R2000 are named, not read.
            Err(undwg::DatabaseError::Drawing(undwg::DrawingError::Header(_))) => {
                skipped += 1;
                continue;
            }
            Err(e) => panic!("{}: {e}", path.display()),
        };
        // The same bytes, the same drawing.
        let again = undwg::read_database(&bytes).expect("a second read");
        assert_eq!(db, again, "{}: two reads differ", path.display());
        let json = db.to_json(ToJsonOptions::default()).expect("json");
        assert_eq!(
            json,
            again.to_json(ToJsonOptions::default()).expect("json"),
            "{}",
            path.display()
        );

        for entity in db.all_entities() {
            // Every entity names a layer the drawing has.
            match &entity.common().layer {
                Ref::Resolved(name) => assert!(
                    db.tables.layers.contains_key(name),
                    "{}: layer {name:?} is not in the table",
                    path.display()
                ),
                other => panic!("{}: layer {other:?}", path.display()),
            }
        }
        // The top level is what model space and the paper spaces own.
        let spaces: Vec<_> = db
            .tables
            .block_records
            .iter()
            .filter(|(n, _)| n.starts_with("*Model_Space") || n.starts_with("*Paper_Space"))
            .flat_map(|(_, b)| b.entities.iter())
            .map(|e| e.common().id)
            .collect();
        let top: Vec<_> = db.entities.iter().map(|e| e.common().id).collect();
        assert_eq!(
            top.iter().collect::<BTreeSet<_>>(),
            spaces.iter().collect::<BTreeSet<_>>(),
            "{}: the top level is not what the spaces own",
            path.display()
        );
        // The diagnostics only name what is not converted yet, and the leaders of
        // R2010 and later, which are named as not read.
        for w in &db.read_diagnostics.warnings {
            assert!(
                w.contains("entity not converted")
                    || w.contains("a leader of R2010 and later is not read"),
                "{}: unexpected diagnostic {w:?}",
                path.display()
            );
        }
        entities += db.all_entities().count();
        read += 1;
    }
    eprintln!("{read} drawings read ({entities} entities), {skipped} of other versions");
    assert!(read > 0, "the corpus holds no readable file");
}
