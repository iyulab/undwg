//! Damaged files: whatever the bytes are, reading them ends -- with a drawing or with an error, never with a
//! panic and never by running on.
//!
//! Point `UNDWG_CORPUS` at one or more directories (separated like `PATH`) holding `.dwg` files. Every file is
//! damaged in several ways -- bits flipped anywhere, bits flipped in the header, a cut-off end, a run of zeros,
//! a word set to all ones, a chunk copied over another place -- and read whole (`read_database`). The damage is
//! decided by a fixed seed, so a failure repeats; the failing input is written next to the test binary's output
//! (`target/damaged-*.dwg`). `UNDWG_DAMAGE_ROUNDS` sets how many damaged copies each file gets (default 6).
//! Without `UNDWG_CORPUS` the test says it skipped and passes -- the files are not part of this repository.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A read that has not ended after this long has run on.
const LIMIT: Duration = Duration::from_secs(60);

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

/// SplitMix64: a small generator with a fixed sequence for a seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// The damage `kind` does to `bytes`, and what it was, in words.
fn damage(bytes: &[u8], kind: usize, rng: &mut Rng) -> (Vec<u8>, String) {
    let mut out = bytes.to_vec();
    let len = out.len();
    // The first six bytes name the version; damage there is a different test.
    let first = 6.min(len);
    let note = match kind % 6 {
        0 => {
            let count = 1 + rng.below(16);
            for _ in 0..count {
                let at = first + rng.below(len - first);
                out[at] ^= 1 << rng.below(8);
            }
            format!("{count} bits flipped anywhere")
        }
        1 => {
            let span = len.min(1024) - first;
            let count = 1 + rng.below(8);
            for _ in 0..count {
                let at = first + rng.below(span);
                out[at] ^= 1 << rng.below(8);
            }
            format!("{count} bits flipped in the first 1 KiB")
        }
        2 => {
            let to = 16 + rng.below(len - 16);
            out.truncate(to);
            format!("cut off at {to}")
        }
        3 => {
            let at = first + rng.below(len - first);
            let run = (1 + rng.below(256)).min(len - at);
            out[at..at + run].fill(0);
            format!("{run} zeros at {at}")
        }
        4 => {
            let at = first + rng.below(len - first - 4);
            out[at..at + 4].fill(0xFF);
            format!("all ones at {at}")
        }
        _ => {
            let size = 1 + rng.below(512.min(len - first));
            let from = first + rng.below(len - first - size);
            let to = first + rng.below(len - first - size);
            let chunk = bytes[from..from + size].to_vec();
            out[to..to + size].copy_from_slice(&chunk);
            format!("{size} bytes from {from} copied to {to}")
        }
    };
    (out, note)
}

#[test]
fn damaged_files_end_without_a_panic_and_in_time() {
    let Some(roots) = std::env::var_os("UNDWG_CORPUS") else {
        eprintln!("skipped: UNDWG_CORPUS is not set");
        return;
    };
    let rounds: usize = std::env::var("UNDWG_DAMAGE_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        dwg_files(&root, &mut files);
    }
    assert!(!files.is_empty(), "UNDWG_CORPUS names no .dwg file");
    let (mut reads, mut drawings, mut errors) = (0usize, 0usize, 0usize);
    let mut slowest = (Duration::ZERO, String::new());
    for (index, path) in files.iter().enumerate() {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if bytes.len() < 64 {
            continue;
        }
        for round in 0..rounds {
            let mut rng = Rng(((index as u64) << 20) ^ (round as u64) ^ 0xD1A6_0000);
            let (damaged, note) = damage(&bytes, round, &mut rng);
            let label = format!("{} round {round}: {note}", path.display());
            let (sender, receiver) = mpsc::channel();
            let input = damaged.clone();
            let started = Instant::now();
            std::thread::spawn(move || {
                let outcome = std::panic::catch_unwind(|| undwg::read_database(&input).is_ok());
                let _ = sender.send(outcome);
            });
            let outcome = receiver.recv_timeout(LIMIT);
            let elapsed = started.elapsed();
            if outcome.is_err() || matches!(outcome, Ok(Err(_))) {
                let saved = format!("target/damaged-{index}-{round}.dwg");
                let _ = std::fs::write(&saved, &damaged);
                match outcome {
                    Err(_) => panic!("ran on past {LIMIT:?}: {label} (input saved to {saved})"),
                    _ => panic!("panicked: {label} (input saved to {saved})"),
                }
            }
            if elapsed > slowest.0 {
                slowest = (elapsed, label);
            }
            reads += 1;
            if matches!(outcome, Ok(Ok(true))) {
                drawings += 1;
            } else {
                errors += 1;
            }
        }
    }
    eprintln!(
        "damaged reads {reads}: a drawing {drawings}, an error {errors}; slowest {:?} ({})",
        slowest.0, slowest.1
    );
}
