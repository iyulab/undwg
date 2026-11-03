//! Version detection against real files.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`)
//! holding `.dwg` files; the test reads the first bytes of every file under
//! them and checks that the answer agrees, file by file, with the six bytes
//! themselves. Without the variable the test says it skipped and passes --
//! the files are not part of this repository.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use undwg::{detect_version, Error, Version};

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
fn every_real_file_gets_the_answer_its_first_six_bytes_give() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");

    let mut by_magic: BTreeMap<String, usize> = BTreeMap::new();
    let (mut read, mut refused) = (0usize, 0usize);
    for path in &files {
        let mut head = Vec::new();
        File::open(path)
            .and_then(|f| f.take(6).read_to_end(&mut head))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        *by_magic
            .entry(String::from_utf8_lossy(&head).into_owned())
            .or_default() += 1;

        let readable = Version::ALL
            .iter()
            .any(|v| v.magic().as_slice() == head.as_slice());
        match detect_version(&head) {
            Ok(version) => {
                assert!(
                    readable,
                    "{}: read as {version} but the bytes are not a listed magic",
                    path.display()
                );
                assert_eq!(
                    version.magic().as_slice(),
                    head.as_slice(),
                    "{}",
                    path.display()
                );
                read += 1;
            }
            Err(Error::UnsupportedVersion { magic, .. }) => {
                assert!(!readable, "{}: refused a listed magic", path.display());
                assert_eq!(magic.as_slice(), head.as_slice(), "{}", path.display());
                refused += 1;
            }
            Err(Error::Truncated { len }) => {
                assert!(head.len() < 6 && len == head.len(), "{}", path.display());
                refused += 1;
            }
            Err(other) => panic!("{}: {other}", path.display()),
        }
    }
    eprintln!("{} files: {read} read, {refused} refused", files.len());
    for (magic, count) in &by_magic {
        eprintln!("  {magic:?}: {count}");
    }
    assert_eq!(read + refused, files.len());
}
