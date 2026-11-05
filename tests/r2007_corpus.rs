//! The container of real R2007 files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. For every R2007 file the error-corrected file header
//! must read and state the file's length, the page map and the section map
//! must read, every section must assemble to exactly its stated size with
//! every page's Reed-Solomon parity and 32-bit checksum holding, and the
//! sections every drawing has must be there. A byte damaged inside a page or
//! inside the header must be an error, not a different answer. Without the
//! variable the test says it skipped and passes -- the files are not part of
//! this repository.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use undwg::{
    read_directory_r2007, read_header, read_header_r2007, read_section_r2007, FileHeader,
    PAGES_START,
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
fn every_real_r2007_container_reads_and_every_page_is_intact() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let mut containers = 0usize;
    let mut pages = 0usize;
    let mut assembled = 0usize;
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut damaged_page = None;
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let Ok(FileHeader::R2007(header)) = read_header(&bytes) else {
            continue;
        };
        let shown = path.display();
        assert_eq!(header.file_size, bytes.len() as u64, "{shown}: file size");
        assert_eq!(header.stream_version, 0x60100, "{shown}: stream version");
        let directory =
            read_directory_r2007(&bytes, &header).unwrap_or_else(|e| panic!("{shown}: {e}"));
        containers += 1;

        for section in &directory.sections {
            let data = read_section_r2007(&bytes, &directory, &section.name)
                .unwrap_or_else(|e| panic!("{shown}: section {:?}: {e}", section.name))
                .unwrap_or_else(|| panic!("{shown}: section {:?} vanished", section.name));
            assert_eq!(
                data.len() as u64,
                section.size,
                "{shown}: section {:?} size",
                section.name
            );
            pages += section.pages.len();
            assembled += data.len();
            *by_name.entry(section.name.clone()).or_default() += 1;
        }
        for needed in [
            "AcDb:Header",
            "AcDb:Classes",
            "AcDb:Handles",
            "AcDb:AcDbObjects",
        ] {
            assert!(
                directory.section(needed).is_some(),
                "{shown}: no {needed} section"
            );
        }
        if damaged_page.is_none() {
            damaged_page = Some((bytes, directory));
        }
    }
    eprintln!(
        "{containers} R2007 containers · {pages} pages · {assembled} bytes assembled · sections: {by_name:?}"
    );
    if containers == 0 {
        eprintln!("note: the corpus holds no R2007 file");
        return;
    }

    // Damage one byte and see that it is refused.
    let (bytes, directory) = damaged_page.expect("at least one container");
    let section = directory.section("AcDb:Header").expect("checked above");
    let page = section.pages[0];
    let entry = directory.page(page.id).expect("the page map lists it");
    let at = (PAGES_START + entry.offset) as usize + 40;
    let mut bad = bytes.clone();
    bad[at] ^= 0x01;
    assert!(
        read_section_r2007(&bad, &directory, "AcDb:Header").is_err(),
        "a damaged page must not assemble"
    );
    let mut bad = bytes;
    bad[0x80 + 100] ^= 0x01;
    assert!(
        read_header_r2007(&bad).is_err(),
        "a damaged file header must not read"
    );
}
