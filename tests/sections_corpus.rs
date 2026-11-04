//! The table of contents of real R2004-family files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. For every R2004, R2010, R2013 and R2018 file the page
//! map and the section map must read (both are checksummed and compressed),
//! every page of every section must be a data page whose header and payload
//! checksums hold and whose header agrees with the section map, and the
//! sections every drawing has must be there. Without the variable the test
//! says it skipped and passes -- the files are not part of this repository.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use undwg::{read_data_page, read_directory, read_header, read_section, FileHeader};

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
fn every_real_table_of_contents_reads_and_every_page_it_lists_is_intact() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let mut directories = 0usize;
    let mut pages_checked = 0usize;
    let mut assembled_bytes = 0usize;
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok(FileHeader::R2004(_, header)) = read_header(&bytes) else {
            continue;
        };
        let dir =
            read_directory(&bytes, &header).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        directories += 1;

        for section in &dir.sections {
            if section.page_count as usize != section.pages.len() {
                panic!("{}: section {:?} page count", path.display(), section.name);
            }
            if !section.name.is_empty() {
                *by_name.entry(section.name.clone()).or_default() += 1;
            }
            for page in &section.pages {
                let entry = dir.page(page.page_number).unwrap_or_else(|| {
                    panic!(
                        "{}: section {:?} lists page {} that the page map lacks",
                        path.display(),
                        section.name,
                        page.page_number
                    )
                });
                let (head, payload) = read_data_page(&bytes, entry.address)
                    .unwrap_or_else(|e| panic!("{}: {:?}: {e}", path.display(), section.name));
                assert_eq!(head.section_number, section.id, "{}", path.display());
                assert_eq!(head.page_size, entry.size, "{}", path.display());
                assert_eq!(head.data_size, page.data_size, "{}", path.display());
                assert_eq!(head.start_offset, page.start_offset, "{}", path.display());
                assert_eq!(payload.len() as u32, head.data_size);
                pages_checked += 1;
            }
        }
        // Assembling each section decompresses its pages: the result must be exactly
        // the stated size, and the two sections that open with a documented sentinel
        // must open with it.
        for section in &dir.sections {
            if section.name.is_empty() || section.encrypted == 1 {
                continue;
            }
            let data = read_section(&bytes, &dir, &section.name)
                .unwrap_or_else(|e| panic!("{}: {:?}: {e}", path.display(), section.name))
                .unwrap();
            assert_eq!(
                data.len() as u64,
                section.size,
                "{}: {:?}",
                path.display(),
                section.name
            );
            assembled_bytes += data.len();
        }
        let header_section = read_section(&bytes, &dir, "AcDb:Header").unwrap().unwrap();
        assert_eq!(
            &header_section[..16],
            &[
                0xCF, 0x7B, 0x1F, 0x23, 0xFD, 0xDE, 0x38, 0xA9, 0x5F, 0x7C, 0x68, 0xB8, 0x4E, 0x6D,
                0x33, 0x5F
            ],
            "{}: header section sentinel",
            path.display()
        );
        let classes = read_section(&bytes, &dir, "AcDb:Classes").unwrap().unwrap();
        assert_eq!(
            &classes[..16],
            &[
                0x8D, 0xA1, 0xC4, 0xB8, 0xC4, 0xA9, 0xF8, 0xC5, 0xC0, 0xDC, 0xF4, 0x5F, 0xE7, 0xCF,
                0xB6, 0x8A
            ],
            "{}: classes section sentinel",
            path.display()
        );
        for name in [
            "AcDb:Header",
            "AcDb:Classes",
            "AcDb:Handles",
            "AcDb:AcDbObjects",
        ] {
            assert!(
                dir.section(name).is_some(),
                "{}: no section {name}",
                path.display()
            );
        }
    }
    eprintln!("{directories} directories read; {pages_checked} data pages intact; {assembled_bytes} section bytes assembled");
    for (name, count) in &by_name {
        eprintln!("  {name}: {count}");
    }
    assert!(directories > 0, "the corpus holds no R2004-family file");
}
