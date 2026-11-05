//! Header variables of real files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files. Every R2000, R2004, R2010, R2013 and R2018 file must
//! give header variables: the checksum holds and the main, string and handle
//! streams each end where the format puts their end. With `UNDWG_DUMP_HEADER`
//! set to a file name, the values another reader also exports are written there,
//! one file per line, so they can be compared. Without `UNDWG_CORPUS` the test
//! says it skipped and passes -- the files are not part of this repository.

use std::io::Write;
use std::path::{Path, PathBuf};

use undwg::{
    read_directory, read_directory_r2007, read_header, read_header_variables, read_section,
    read_section_r2007, FileHeader, TextValue, Version,
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

fn text(t: &TextValue) -> String {
    match t {
        TextValue::Ansi(b) => String::from_utf8_lossy(b).into_owned(),
        TextValue::Unicode(s) => s.clone(),
    }
}

#[test]
fn every_real_header_reads_and_is_plausible() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");
    let mut dump = std::env::var_os("UNDWG_DUMP_HEADER")
        .map(|p| std::fs::File::create(p).expect("cannot create UNDWG_DUMP_HEADER"));

    let mut read = 0usize;
    let mut by_version = std::collections::BTreeMap::<String, usize>::new();
    for path in &files {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let (version, section) = match read_header(&bytes) {
            Ok(FileHeader::R2004(version, header)) => {
                let dir = read_directory(&bytes, &header)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let section = read_section(&bytes, &dir, "AcDb:Header")
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .unwrap_or_else(|| panic!("{}: no header section", path.display()));
                (version, section)
            }
            Ok(FileHeader::R2007(header)) => {
                let dir = read_directory_r2007(&bytes, &header)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let get = |name: &str| {
                    read_section_r2007(&bytes, &dir, name)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                        .unwrap_or_else(|| panic!("{}: no {name}", path.display()))
                };
                (Version::R2007, get("AcDb:Header"))
            }
            Ok(FileHeader::R2000(header)) => {
                let section = header
                    .section(&bytes, 0)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .unwrap_or_else(|| panic!("{}: no header locator", path.display()));
                (Version::R2000, section.to_vec())
            }
            _ => continue,
        };
        let vars = read_header_variables(&section, version)
            .unwrap_or_else(|e| panic!("{} ({version}): {e}", path.display()));

        // Plausible values: finite numbers, sane enumerations, a handle seed
        // above every handle the file names.
        assert!(vars.line_type_scale.is_finite() && vars.line_type_scale > 0.0);
        assert!((0..=4).contains(&vars.angular_units), "{}", path.display());
        assert!((1..=6).contains(&vars.linear_units), "{}", path.display());
        assert!(vars.handle_seed > 0, "{}", path.display());
        for h in [
            vars.model_space_block,
            vars.paper_space_block,
            vars.controls.layer,
            vars.controls.named_objects,
            vars.current_layer,
        ] {
            assert!(h != 0 && h < vars.handle_seed, "{}", path.display());
        }
        if let Some(dump) = dump.as_mut() {
            writeln!(
                dump,
                "{}\t{}\t{}\t{:?}\t{:?}\t{}\t{}",
                path.display(),
                version,
                vars.insertion_units,
                vars.point_mode,
                vars.point_size,
                text(&vars.fingerprint_guid),
                text(&vars.version_guid)
            )
            .unwrap();
        }
        read += 1;
        *by_version.entry(version.to_string()).or_default() += 1;
    }
    eprintln!("{read} files read: {by_version:?}");
    assert!(read > 0, "the corpus holds no readable file");
}
