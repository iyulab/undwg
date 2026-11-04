# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `detect_version`: the version a DWG file declares in its first six bytes. R2000 to R2018 (`AC1015` to `AC1032`) are read; R13 and R14 are named and refused; anything else is an error that shows the bytes it found.
- A corpus test that checks, file by file, that the answer agrees with the six bytes.
- `read_header`: the file header. For R2000 the section-locator list, with its 16-bit checksum and closing sentinel checked; for R2004, R2010, R2013 and R2018 the masked header block is unmasked, its identifier and 32-bit checksum checked, and the positions of the page map and section map returned. R2007 is an explicit "not read yet" error. A checksum, identifier or sentinel that does not match is an error that says what was found.
- A corpus test over real files: every R2000 to R2018 header passes its checksum and its stored offsets lie inside the file.
- `read_directory`, `read_data_page`, `read_section`: for R2004, R2010, R2013 and R2018, the section page map and the section map (compressed system pages, checksummed), data pages (masked 32-byte header, header and payload checksums), and a section assembled from its pages into exactly its stated size. The decompressor refuses a copy from before the start of the output, an opcode the format does not use, and any output length other than the stated one. A section marked encrypted is refused rather than returned.
- `BitReader`: the bit-packed value forms the format uses -- bit, bit triplet, bitshort, bitlong, bitlonglong, bitdouble (with and without a default), raw values at any bit alignment, modular chars and shorts, handle references, object types. Reading past the end of the data and the codes the format marks as unused are errors, never a zero.
- `read_classes`: the class definitions of an R2000, R2004, R2010, R2013 or R2018 file (R2000 does not state how many objects a class holds), with the section's checksum checked and, from R2010, the main stream and the string stream required to meet where they should.
- A corpus test that reads the class definitions of every R2004-family file.
- `read_object_map`, `read_object_frame`: the object map (checksummed blocks of handle and position increments) and the frame of each object -- its size, its type code and its checksum. An entry whose position lies outside the object data is left for the caller to see; an object whose checksum or size does not hold is an error, not a guess.
- A corpus test over the object maps and object frames of real files.
- `read_entity`: the common data every entity has (handle, extended data, graphic flag, colour, linetype scale, line weight, visibility) and the geometry of lines, circles, arcs and points. Any other kind of object is left alone. The object map and the frames now also check that the object found at a position states the handle the map gave it.
- A corpus test that reads every line, circle, arc, point, lightweight polyline, ellipse, solid, 3D face and block insertion of real files.
- `read_entity` reads solids (thickness, elevation, four corners, extrusion) and 3D faces (the first corner whole, the others difference-coded against the one before, and which edges are invisible).
- `read_entity` reads ellipses and elliptical arcs: the centre and the semi-major axis vector (both in world coordinates), the extrusion, the axis ratio and the start and end angles (eccentric anomalies in radians).
- `read_entity` reads block insertions: the insertion point, the scale (stored as all ones, one value with the others defaulting to it, or three values), the rotation, the extrusion and whether attributes follow. The inserted block is named by a handle in the handle stream, which is not read yet.
- `read_entity` reads lightweight polylines: the vertices (the first stored whole, the rest difference-coded against the previous one), the bulge and width of each segment, the constant width, the closed flag, elevation, thickness and normal. A count the stream states is bounded by the bits left, so damaged data cannot allocate.
- `read_entity` also reads the handle stream of an entity: its `references` (owner, persistent reactors, extension dictionary, layer, and the linetype, material and plot style when the entity has its own) with every relative handle resolved to an absolute one, and for a block insertion the block header it inserts, its attributes and its sequence end. `BitReader`'s `HandleRef::resolve` does the resolving.
- `read_entity` now checks that an entity's fields fill its main data stream exactly and that its handles reach the end of the object's data; an entity that is read wrongly stops being read quietly. For R2007 and later the string stream that precedes the handles is located (its size field may continue in a second short).
- `HeaderR2000::section`: the bytes of a section the R2000 locator list names. The class definitions, the object map and the object frames of R2000 files are read with the same readers as later versions, from those bytes and from the file itself.
- A corpus test that reads the table of contents of every R2004-family file, checks every listed page, assembles every section, and checks that the header and class sections open with their documented sentinels.
