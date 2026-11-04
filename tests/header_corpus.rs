//! File headers against real files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every file whose version this reader reads must give
//! a header that passes its own checksum and whose stored offsets lie inside
//! the file; R2007 files must be named as not yet read. Without the variable
//! the test says it skipped and passes -- the files are not part of this
//! repository.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use undwg::{read_header, FileHeader, HeaderError, Version, SECTION_PAGE_MAP_BIAS};

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
fn every_real_header_passes_its_checksum_and_points_inside_the_file() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let mut by_version: BTreeMap<String, usize> = BTreeMap::new();
    let mut not_yet = 0usize;
    let mut refused = 0usize;
    let mut past_end = 0usize;
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let len = bytes.len() as u64;
        match read_header(&bytes) {
            Ok(FileHeader::R2000(h)) => {
                *by_version.entry("R2000".into()).or_default() += 1;
                assert!(
                    (3..=6).contains(&h.locators.len()),
                    "{}: {} locators",
                    path.display(),
                    h.locators.len()
                );
                for l in &h.locators {
                    assert!(
                        u64::from(l.seeker) + u64::from(l.size) <= len,
                        "{}: locator {} runs past the end ({} + {} > {len})",
                        path.display(),
                        l.record,
                        l.seeker,
                        l.size
                    );
                }
                let records: Vec<u8> = h.locators.iter().map(|l| l.record).collect();
                assert_eq!(&records[..3], &[0, 1, 2], "{}", path.display());
            }
            Ok(FileHeader::R2004(version, h)) => {
                *by_version.entry(version.name().into()).or_default() += 1;
                assert!(
                    h.section_page_map_address + SECTION_PAGE_MAP_BIAS < len,
                    "{}: page map at {:#x} is outside {len} bytes",
                    path.display(),
                    h.section_page_map_address + SECTION_PAGE_MAP_BIAS
                );
                assert!(
                    h.second_header_address < len,
                    "{}: second header at {:#x} is outside {len} bytes",
                    path.display(),
                    h.second_header_address
                );
                // The specification calls this the end address of the last
                // section page; real files do not keep it inside the file
                // (it is measured here, not asserted).
                if h.last_section_page_end_address > len {
                    past_end += 1;
                }
            }
            Err(HeaderError::NotYetRead(Version::R2007)) => not_yet += 1,
            Err(HeaderError::Version(_)) => refused += 1,
            Ok(other) => panic!("{}: unexpected header form {other:?}", path.display()),
            Err(other) => panic!("{}: {other}", path.display()),
        }
    }
    eprintln!(
        "{} files: headers read {:?}; R2007 named not yet read: {not_yet}; other versions refused: {refused}; last-page end address past the file: {past_end}",
        files.len(),
        by_version
    );
    let read: usize = by_version.values().sum();
    assert_eq!(read + not_yet + refused, files.len());
}
