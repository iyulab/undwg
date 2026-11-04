# undwg

A pure-Rust reader for the DWG file format, written from the public file specification and real files. Safe Rust only (`#![forbid(unsafe_code)]` through the lint table), no native dependencies, and a reader that says what it cannot read instead of guessing.

## Usage

```rust
use undwg::{detect_version, Version};

let bytes = std::fs::read("drawing.dwg")?;
match detect_version(&bytes) {
    Ok(version) => println!("{version}"),          // e.g. "R2013 (AC1027)"
    Err(error) => println!("not read: {error}"),  // names what it found
}
# Ok::<(), std::io::Error>(())
```

## Status

Pre-release. The reader is built layer by layer, and each layer is checked against real files before the next one is trusted.

| Layer | State |
|---|---|
| Version detection (R2000 to R2018) | done |
| File header (R2000, R2004, R2010, R2013, R2018) with checksums | done |
| File header of R2007 | planned |
| Section page map, section map, decompression, section assembly (R2004 and later) | done |
| Bit-coded values, class definitions (R2000, R2004 and later; R2007 excepted) | done |
| Object map and object frames (R2000, R2004 and later; size, type, checksum) | done |
| Lines, circles, arcs and points (R2000, R2004 and later) | done |
| Lightweight polylines: vertices, bulges, widths, closed flag, plane (R2000, R2004 and later) | done |
| Block insertions: point, scale, rotation, extrusion (R2000, R2004 and later; the inserted block's name is a handle and is not read yet) | done |
| Ellipses and elliptical arcs: centre, major axis, ratio, angles (R2000, R2004 and later) | done |
| Solids (2D filled areas) and 3D faces: corners, extrusion, invisible edges (R2000, R2004 and later) | done |
| Layers: name, flags, colour, linetype (R2000, R2004 and later); the colour, linetype, scale, visibility and line weight every entity states | done |
| Block headers: name, base point, external reference (R2000, R2004 and later) | done |
| Viewports: window, view, frozen layers (R2000, R2004 and later) | done |
| Other entities, other tables, header variables | planned |

Files older than R2000 are an explicit error that names the version bytes it found. Writing DWG is out of scope.

## Clean-room rules

How the code is written, and which sources it may and may not use, is in [CLEANROOM.md](CLEANROOM.md). Third-party material is listed in [THIRD_PARTY.md](THIRD_PARTY.md).

## License

MIT. See [LICENSE](LICENSE).
