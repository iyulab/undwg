//! Class definitions of real R2004-family files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every R2000, R2004, R2010, R2013 and R2018 file must give
//! class definitions: the section's checksum holds, the main stream and (from
//! R2010) the string stream meet exactly where they should, and what is read
//! is plausible -- numbers from 500 up and distinct, each class producing
//! entities or objects. Without the variable the test says it
//! skipped and passes -- the files are not part of this repository.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use undwg::{read_classes, read_directory, read_header, read_section, FileHeader, Version};

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
fn every_real_class_section_reads_and_is_plausible() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let (mut files_read, mut classes_read, mut with_empty_name) = (0usize, 0usize, 0usize);
    let mut names: BTreeSet<String> = BTreeSet::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (version, section) = match read_header(&bytes) {
            Ok(FileHeader::R2004(version, header)) => {
                let dir = read_directory(&bytes, &header)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let section = read_section(&bytes, &dir, "AcDb:Classes")
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .unwrap_or_else(|| panic!("{}: no class section", path.display()));
                (version, section)
            }
            Ok(FileHeader::R2000(header)) => {
                let section = header
                    .section(&bytes, 1)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .unwrap_or_else(|| panic!("{}: no class locator", path.display()));
                (Version::R2000, section.to_vec())
            }
            _ => continue,
        };
        let classes = read_classes(&section, version)
            .unwrap_or_else(|e| panic!("{} ({version}): {e}", path.display()));

        let mut numbers = BTreeSet::new();
        for class in &classes {
            assert!(
                class.number >= 500,
                "{}: class {}",
                path.display(),
                class.number
            );
            assert!(
                numbers.insert(class.number),
                "{}: class {} twice",
                path.display(),
                class.number
            );
            assert!(
                class.item_class_id == 0x1F2 || class.item_class_id == 0x1F3,
                "{}: class {} item id {:#x}",
                path.display(),
                class.number,
                class.item_class_id
            );
            // Some writers leave names empty; that is measured, not asserted.
            if class.app_name.is_empty()
                || class.cplusplus_name.is_empty()
                || class.dxf_name.is_empty()
            {
                with_empty_name += 1;
            }
            assert!(
                class.instance_count.is_none_or(|n| n >= 0),
                "{}",
                path.display()
            );
            names.insert(class.dxf_name.clone());
        }
        files_read += 1;
        classes_read += classes.len();
    }
    eprintln!(
        "{files_read} files: {classes_read} class definitions ({with_empty_name} with an empty name), {} distinct DXF names",
        names.len()
    );
    assert!(files_read > 0, "the corpus holds no R2004-family file");
}
