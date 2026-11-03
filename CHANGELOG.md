# Changelog

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `detect_version`: the version a DWG file declares in its first six bytes. R2000 to R2018 (`AC1015` to `AC1032`) are read; R13 and R14 are named and refused; anything else is an error that shows the bytes it found.
- A corpus test that checks, file by file, that the answer agrees with the six bytes.
