# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `detect_version`: the version a DWG file declares in its first six bytes. R2000 to R2018 (`AC1015` to `AC1032`) are read; R13 and R14 are named and refused; anything else is an error that shows the bytes it found.
- A corpus test that checks, file by file, that the answer agrees with the six bytes.
- `read_header`: the file header. For R2000 the section-locator list, with its 16-bit checksum and closing sentinel checked; for R2004, R2010, R2013 and R2018 the masked header block is unmasked, its identifier and 32-bit checksum checked, and the positions of the page map and section map returned. R2007 is an explicit "not read yet" error. A checksum, identifier or sentinel that does not match is an error that says what was found.
- A corpus test over real files: every R2000 to R2018 header passes its checksum and its stored offsets lie inside the file.
- `read_directory`, `read_data_page`, `read_section`: for R2004, R2010, R2013 and R2018, the section page map and the section map (compressed system pages, checksummed), data pages (masked 32-byte header, header and payload checksums), and a section assembled from its pages into exactly its stated size. The decompressor refuses a copy from before the start of the output, an opcode the format does not use, and any output length other than the stated one. A section marked encrypted is refused rather than returned.
- A corpus test that reads the table of contents of every R2004-family file, checks every listed page, assembles every section, and checks that the header and class sections open with their documented sentinels.
