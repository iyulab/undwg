# Clean-room rules

`undwg` is licensed MIT. For that license to mean something, the code has to come from sources whose terms allow it. These are the rules the project follows. They reduce the chance of derived code and cannot eliminate it, and they are written down so that anyone can check a change against them.

## What the code is written from

- **The public file specification**: the Open Design Alliance's *Open Design Specification for .dwg files*, which is published for download.
- **Real files**: measuring what a file actually contains, byte by byte, is the arbiter wherever the specification is silent or wrong.
- **Public references for related formats** (for example the DXF reference), where they describe the same data.
- **Algorithm descriptions from permissively licensed implementations** (MIT or Apache-2.0), only at a point where the specification is silent, and each such point is recorded in [THIRD_PARTY.md](THIRD_PARTY.md) with what was learned and from where. No source text is copied.

## What the code is not written from

- The source code, headers or vendored copies of any GPL- or LGPL-licensed DWG implementation, including LibreDWG.
- Any implementation whose license does not permit what this project does with it.

Such implementations may be **observed as separate processes**: run them on a file and compare their output with ours. Their output is data about the file. Their code is not read.

## How a change is checked

- A change that relies on something other than the sources above does not go in.
- Where a change needed a source beyond the specification and real files, the change says which.
- A contributor who has read GPL- or LGPL-licensed DWG source says so before contributing to the same area.
- Tests compare against real files and against an independent observation of them, not against our own previous output.
