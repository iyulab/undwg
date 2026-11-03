//! The same bytes must give the same answer, byte for byte, every time.

use undwg::{detect_version, Version};

/// Enough repetitions that an answer depending on a randomized iteration
/// order would not pass by luck.
const RUNS: usize = 24;

/// Every shape of input the version reader distinguishes.
fn inputs() -> Vec<Vec<u8>> {
    let mut all: Vec<Vec<u8>> = Version::ALL.iter().map(|v| v.magic().to_vec()).collect();
    all.extend([
        b"AC1012".to_vec(),
        b"AC1014".to_vec(),
        b"AC1009".to_vec(),
        b"AC10".to_vec(),
        Vec::new(),
        vec![0xFF; 64],
    ]);
    all
}

fn render(bytes: &[u8]) -> String {
    match detect_version(bytes) {
        Ok(version) => format!("ok {version} {:?}", version.magic()),
        Err(error) => format!("err {error} {error:?}"),
    }
}

#[test]
fn repeated_detection_is_identical() {
    for input in inputs() {
        let first = render(&input);
        for run in 1..RUNS {
            assert_eq!(first, render(&input), "run {run}: {input:?}");
        }
    }
}

#[test]
fn the_list_of_readable_versions_has_one_fixed_order() {
    let names: Vec<_> = Version::ALL.iter().map(|v| v.name()).collect();
    assert_eq!(
        names,
        ["R2000", "R2004", "R2007", "R2010", "R2013", "R2018"]
    );
}
