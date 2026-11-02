# `.nclab` lab format — version 1

NetworksCraft labs are UTF-8 JSON with one flat object and the magic `"format":
"networkscraft-lab"`. The format is deliberately hand-editable. Unknown fields are ignored on read,
so a newer writer can add hints without breaking older readers. The reader rejects a different magic
or a different major version.

## Example

```json
{
  "format": "networkscraft-lab",
  "formatVersion": 1,
  "title": "Sample lab",
  "nodes": [
    { "id": 0, "kind": "router", "name": "R1", "x": 300.0, "y": 300.0, "ipv4": null },
    { "id": 1, "kind": "switch", "name": "SW1", "x": 610.0, "y": 300.0, "ipv4": null },
    { "id": 2, "kind": "host", "name": "PC1", "x": 920.0, "y": 190.0, "ipv4": "10.0.0.1" },
    { "id": 3, "kind": "host", "name": "PC2", "x": 920.0, "y": 410.0, "ipv4": "10.0.0.2" }
  ],
  "links": [
    { "id": 0, "a": 0, "b": 1 },
    { "id": 1, "a": 1, "b": 2 },
    { "id": 2, "a": 1, "b": 3 }
  ]
}
```

## Fields

### Root

| Field | Type | Required | Default | Notes |
|---|---|---|---|---|
| `format` | string | yes | — | must be exactly `networkscraft-lab` |
| `formatVersion` | number | yes | — | must be `1` |
| `title` | string | no | `"Untitled lab"` | at most 256 Unicode scalar values |
| `nodes` | array | no | `[]` | at most 4096 |
| `links` | array | no | `[]` | at most 8192 |

### Node

| Field | Type | Required | Default | Notes |
|---|---|---|---|---|
| `id` | number | no | `0` | unique, `0..=1_000_000` |
| `kind` | string | yes | — | `router`, `switch` or `host` |
| `name` | string | no | kind prefix + id | at most 256 characters, non-empty after defaulting |
| `x` | number | no | `0.0` | finite, `|x| <= 1_000_000` |
| `y` | number | no | `0.0` | finite, `|y| <= 1_000_000` |
| `ipv4` | string or null | no | `null` | a bare IPv4 address, no prefix or port |

### Link

| Field | Type | Required | Default | Notes |
|---|---|---|---|---|
| `id` | number | no | `0` | unique, `0..=1_000_000` |
| `a` | number | no | `0` | must reference a node, not equal to `b` |
| `b` | number | no | `0` | must reference a node, not equal to `a` |

## Limits

| Limit | Value |
|---|---|
| File size | 8 MiB (8,388,608 bytes) |
| Devices | 4096 |
| Cables | 8192 |
| Device/cable id | 1,000,000 |
| Title/name | 256 Unicode scalar values |
| Coordinate | finite, ±1,000,000 |
| IPv4 | `std::net::Ipv4Addr` syntax, no prefix |

The reader checks the file size before reading and caps the read at the limit. It never indexes by
an attacker-provided id. The writer validates the full topology through the same
`Topology::from_parts` function used by the editor, so a lab that can be built can be saved, and an
invalid in-memory lab is refused rather than written.

## Output

- Struct field order is stable (`format`, `formatVersion`, `title`, `nodes`, `links`).
- Two-space pretty JSON, one trailing newline.
- Ids are file-local and stable across saves. Allocation counters are never stored; on load they are
  derived as `max(id) + 1`.
- Unknown fields are ignored on read.

## Saves and opens

- **Save** is best-effort atomic: write a sibling temp file, flush/sync it, then rename it over the
  destination. If serialisation or writing fails, the previous file is untouched.
- **Open** is all-or-nothing: the new lab replaces the session only after the whole file validates.
  A failed open leaves the current lab and selection intact.
- The desktop file picker forces the `.nclab` extension on Save-as. The headless CLI
  uses the path exactly as written.

## Errors

The error type is `net_io::LabError` (`src/io.rs`):

| Variant | Meaning |
|---|---|
| `Io` | the file could not be read/written |
| `Json` | malformed JSON |
| `BadFormat` | missing or wrong magic |
| `UnsupportedVersion(n)` | major version other than 1 |
| `TooLarge` | file exceeds 8 MiB |
| `Invalid(message)` | an invariant failed (ids, links, IPv4, coordinates, names, counts) |

Messages are user-facing and do not echo unbounded attacker-controlled text.

## Compatibility policy

- Version 1 is **frozen** for the M1/M2 line. Additive optional fields are allowed because the reader
  ignores unknown fields; a change that removes or reinterprets an existing field requires a new
  major version.
- A reader must reject an unknown magic or a higher major version with a clear message, never guess.
- A future migration tool will read v1 and write the new version; v1 files stay readable by v1
  builds.
- The wire structs live in `src/io.rs` (`LabDoc`, `LabNode`, `LabLink`) and are not the model structs,
  so renaming an internal field cannot silently change the public format.

## Current first-slice scope

The model stores one optional IPv4 address per device. Interfaces, prefixes, VLANs, static routes and
ACLs are future file-format additions. The format was designed so optional interface records can be
added without changing the meaning of the existing `ipv4` field.
