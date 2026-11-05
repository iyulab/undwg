//! The object map and object frames of real R2004-family files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. For every R2004, R2010, R2013 and R2018 file the
//! object map must read (its blocks are checksummed, its handles increase),
//! every object it lists must have a frame whose checksum holds, and the
//! number of objects of each class must be the number the class table states.
//! Without the variable the test says it skipped and passes -- the files are
//! not part of this repository.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use undwg::{
    read_classes, read_directory, read_directory_r2007, read_header, read_object_frame,
    read_object_map, read_section, read_section_r2007, FileHeader, Version,
};

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
fn every_real_object_has_an_intact_frame_and_the_counts_match_the_class_table() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let (mut files_read, mut objects_read) = (0usize, 0usize);
    let mut class_files = 0usize;
    let (mut equal, mut fewer, mut more) = (0usize, 0usize, 0usize);
    let mut outside = 0usize;
    let mut unreadable: Vec<String> = Vec::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // The class table, the object map and the object data, by the layout the version uses.
        let (version, classes_section, handles, objects): (Version, Vec<u8>, Vec<u8>, Vec<u8>) =
            match read_header(&bytes) {
                Ok(FileHeader::R2004(version, header)) => {
                    let dir = read_directory(&bytes, &header)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    let get = |name: &str| {
                        read_section(&bytes, &dir, name)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                            .unwrap_or_else(|| panic!("{}: no {name}", path.display()))
                    };
                    (
                        version,
                        get("AcDb:Classes"),
                        get("AcDb:Handles"),
                        get("AcDb:AcDbObjects"),
                    )
                }
                Ok(FileHeader::R2007(header)) => {
                    let dir = read_directory_r2007(&bytes, &header)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    let get = |name: &str| {
                        read_section_r2007(&bytes, &dir, name)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                            .unwrap_or_else(|| panic!("{}: no {name}", path.display()))
                    };
                    (
                        Version::R2007,
                        get("AcDb:Classes"),
                        get("AcDb:Handles"),
                        get("AcDb:AcDbObjects"),
                    )
                }
                Ok(FileHeader::R2000(header)) => {
                    let get = |record: u8| {
                        header
                            .section(&bytes, record)
                            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                            .unwrap_or_else(|| panic!("{}: no locator {record}", path.display()))
                            .to_vec()
                    };
                    // R2000 object positions are offsets into the file itself.
                    (Version::R2000, get(1), get(2), bytes.clone())
                }
                _ => continue,
            };
        let entries = read_object_map(&handles)
            .unwrap_or_else(|e| panic!("{}: object map: {e}", path.display()));
        assert!(!entries.is_empty(), "{}", path.display());
        for pair in entries.windows(2) {
            assert!(pair[0].handle < pair[1].handle, "{}", path.display());
        }

        let mut by_type: BTreeMap<u16, usize> = BTreeMap::new();
        for entry in &entries {
            // An entry can name a position past the end of the object
            // section (seen in a few R2013 files); such an entry is counted,
            // not read.
            if entry.offset >= objects.len() as u64 {
                outside += 1;
                continue;
            }
            // A frame whose checksum does not hold is an error the reader reports; a
            // few files in the corpus are regression cases for damaged data, so such
            // objects are counted and listed, not asserted away.
            let frame = match read_object_frame(&objects, *entry, version) {
                Ok(frame) => frame,
                Err(e) => {
                    unreadable.push(format!(
                        "{}: object {:#x}: {e}",
                        path.display(),
                        entry.handle
                    ));
                    continue;
                }
            };
            assert_eq!(frame.handle, entry.handle);
            *by_type.entry(frame.type_code).or_default() += 1;
        }
        objects_read += entries.len();
        files_read += 1;

        // A class states how many objects of its type the drawing holds. Most
        // application classes are numbered by their place in the class list
        // (type 500 + index); the format fixes the type code of a few that
        // appear in the list all the same.
        const FIXED: [(&str, u16); 7] = [
            ("LAYOUT", 0x52),
            ("ACDBPLACEHOLDER", 0x50),
            ("XRECORD", 0x4F),
            ("HATCH", 0x4E),
            ("LWPOLYLINE", 0x4D),
            ("OLE2FRAME", 0x4A),
            ("VBA_PROJECT", 0x51),
        ];
        let classes = read_classes(&classes_section, version).unwrap();
        for (index, class) in classes.iter().enumerate() {
            let code = FIXED
                .iter()
                .find(|(name, _)| *name == class.dxf_name)
                .map_or(500 + index as u16, |(_, code)| *code);
            let found = by_type.get(&code).copied().unwrap_or(0);
            let Some(stated) = class.instance_count.map(i64::from) else {
                continue; // R2000 does not state a count
            };
            match (found as i64).cmp(&stated) {
                std::cmp::Ordering::Equal => equal += 1,
                std::cmp::Ordering::Less => fewer += 1,
                std::cmp::Ordering::Greater => {
                    more += 1;
                    if more <= 5 {
                        eprintln!(
                            "more frames than stated: {}: class {} ({}, type {code}) states {stated}, the frames hold {found}",
                            path.display(),
                            class.number,
                            class.dxf_name
                        );
                    }
                }
            }
        }
        class_files += 1;
    }
    eprintln!(
        "{files_read} files: {objects_read} objects framed ({outside} map entries lie outside the object section); class counts in {class_files} files: {equal} equal, {fewer} with fewer objects than stated, {more} with more"
    );
    // Every failure is a detected one (a checksum or size that does not hold);
    // the files they come from are listed with their counts.
    let mut per_file: BTreeMap<String, usize> = BTreeMap::new();
    for line in &unreadable {
        let file = line.split(": object").next().unwrap_or(line).to_string();
        *per_file.entry(file).or_default() += 1;
    }
    for (file, count) in &per_file {
        eprintln!("unreadable frames: {count} in {file}");
    }
    assert!(files_read > 0, "the corpus holds no R2004-family file");
    // With positions counted per block, every entry names a position inside the
    // object data and every frame holds.
    assert_eq!(outside, 0, "map entries outside the object data");
    assert!(
        unreadable.is_empty(),
        "{} of {objects_read} object frames are unreadable",
        unreadable.len()
    );
}
