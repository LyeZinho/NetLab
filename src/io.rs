//! L3 — the public `.nclab` lab file format.
//!
//! The format is flat, tolerant, versioned JSON. This module owns private wire structs so that an
//! internal model rename cannot silently change a public file. Unknown fields are ignored; unknown
//! formats and major versions are rejected with a human message.

use crate::model::{DeviceKind, Link, LinkId, MAX_LINKS, MAX_NODES, Node, NodeId, Topology};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

/// Magic value stored in every lab.
pub const FORMAT: &str = "networkscraft-lab";
/// Major format version. The reader rejects anything else.
pub const FORMAT_VERSION: u32 = 1;
/// Maximum file size on disk (8 MiB).
pub const MAX_FILE_BYTES: u64 = 8388608;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum LabError {
    #[error("could not access lab file: {0}")]
    Io(#[from] std::io::Error),
    #[error("lab file is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("this is not a NetworksCraft lab file")]
    BadFormat,
    #[error("lab format version {0} is not supported (this build reads version {FORMAT_VERSION})")]
    UnsupportedVersion(u32),
    #[error("lab file is too large (limit 8 MiB)")]
    TooLarge,
    #[error("invalid lab: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, Serialize, Deserialize)]
struct LabDoc {
    #[serde(default)]
    format: String,
    #[serde(rename = "formatVersion", default)]
    format_version: u32,
    #[serde(default)]
    title: String,
    #[serde(default)]
    nodes: Vec<LabNode>,
    #[serde(default)]
    links: Vec<LabLink>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LabNode {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    ipv4: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct LabLink {
    #[serde(default)]
    id: u64,
    #[serde(default)]
    a: u64,
    #[serde(default)]
    b: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cable: Option<String>,
}

/// Serialise a validated lab. The result is deterministic: struct field order, two-space pretty
/// JSON, one trailing newline.
pub fn to_string(lab: &Topology) -> Result<String, LabError> {
    let validated = Topology::from_parts(lab.title.clone(), lab.nodes.clone(), lab.links.clone())
        .map_err(LabError::Invalid)?;

    let doc = LabDoc {
        format: FORMAT.to_string(),
        format_version: FORMAT_VERSION,
        title: validated.title,
        nodes: validated
            .nodes
            .iter()
            .map(|node| LabNode {
                id: node.id.0,
                kind: node.kind.as_str().to_string(),
                model: Some(node.model.as_str().to_string()),
                name: node.name.clone(),
                x: f64::from(node.x),
                y: f64::from(node.y),
                ipv4: node.ipv4.clone(),
            })
            .collect(),
        links: validated
            .links
            .iter()
            .map(|link| LabLink {
                id: link.id.0,
                a: link.a.0,
                b: link.b.0,
                cable: Some(link.cable.as_str().to_string()),
            })
            .collect(),
    };

    let mut text = serde_json::to_string_pretty(&doc)?;
    text.push('\n');
    Ok(text)
}

/// Parse a lab from text. Bounded and all-or-nothing: on error no partial lab escapes.
pub fn from_str(text: &str) -> Result<Topology, LabError> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(LabError::TooLarge);
    }

    let doc: LabDoc = serde_json::from_str(text)?;
    if doc.format != FORMAT {
        return Err(LabError::BadFormat);
    }
    if doc.format_version != FORMAT_VERSION {
        return Err(LabError::UnsupportedVersion(doc.format_version));
    }

    let mut nodes = Vec::with_capacity(doc.nodes.len().min(MAX_NODES));
    for wire in doc.nodes {
        if nodes.len() >= MAX_NODES {
            return Err(LabError::Invalid("too many devices"));
        }
        let kind = DeviceKind::parse(&wire.kind).ok_or(LabError::Invalid("unknown device kind"))?;
        let model = match wire.model.as_deref() {
            Some(text) => {
                let model = crate::model::DeviceModel::parse(text)
                    .ok_or(LabError::Invalid("unknown device model"))?;
                if model.kind() != kind {
                    return Err(LabError::Invalid("device model does not match its kind"));
                }
                model
            }
            None => crate::model::DeviceModel::default_for(kind),
        };
        let name = if wire.name.trim().is_empty() {
            format!("{}{}", model.name_prefix(), wire.id)
        } else {
            wire.name
        };
        nodes.push(Node {
            id: NodeId(wire.id),
            kind,
            model,
            name,
            x: wire.x as f32,
            y: wire.y as f32,
            ipv4: wire.ipv4,
        });
    }

    let mut links = Vec::with_capacity(doc.links.len().min(MAX_LINKS));
    for wire in doc.links {
        if links.len() >= MAX_LINKS {
            return Err(LabError::Invalid("too many links"));
        }
        let cable = match wire.cable.as_deref() {
            Some(text) => crate::model::CableKind::parse(text)
                .ok_or(LabError::Invalid("unknown cable type"))?,
            None => crate::model::CableKind::StraightThrough,
        };
        links.push(Link {
            id: LinkId(wire.id),
            a: NodeId(wire.a),
            b: NodeId(wire.b),
            cable,
        });
    }

    Topology::from_parts(doc.title, nodes, links).map_err(LabError::Invalid)
}

/// Best-effort atomic save: a sibling temp file is written, then renamed over the destination. A
/// failed save leaves the previous file untouched.
pub fn save(path: &Path, lab: &Topology) -> Result<(), LabError> {
    let text = to_string(lab)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(LabError::TooLarge);
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(LabError::Invalid("save path has no file name"))?;
    let pid = std::process::id();
    let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(".{file_name}.{pid}.{nonce}.tmp"));

    let write_result = (|| -> Result<(), LabError> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

/// Load a lab with a size check before reading and a bounded read afterwards.
pub fn load(path: &Path) -> Result<Topology, LabError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(LabError::TooLarge);
    }
    let mut file = std::fs::File::open(path)?;
    let mut text = String::new();
    let limit = MAX_FILE_BYTES.saturating_add(1);
    std::io::Read::by_ref(&mut file)
        .take(limit)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(LabError::TooLarge);
    }
    from_str(&text)
}

/// Append the public extension when a path has none. Pure helper, unit-tested.
pub fn with_lab_extension(path: &Path) -> PathBuf {
    if path.extension().is_some() {
        path.to_path_buf()
    } else {
        let mut out = path.to_path_buf();
        out.set_extension("nclab");
        out
    }
}

/// Human-readable summary used by `info` commands.
pub fn describe(lab: &Topology) -> String {
    let hosts = lab
        .nodes
        .iter()
        .filter(|node| node.kind == DeviceKind::Host)
        .count();
    format!(
        "{} — {} devices ({} hosts), {} links",
        lab.title,
        lab.nodes.len(),
        hosts,
        lab.links.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample_lab;
    use crate::sim::ping;

    fn temp_lab(name: &str) -> PathBuf {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "networkscraft-{name}-{}-{nonce}.nclab",
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_is_stable_and_deterministic() {
        let lab = sample_lab();
        let first = to_string(&lab);
        let second = to_string(&lab);
        assert!(first.is_ok());
        if let (Ok(a), Ok(b)) = (first, second) {
            assert_eq!(a, b);
            assert!(a.ends_with('\n'));
            assert!(a.starts_with("{\n  \"format\": \"networkscraft-lab\""));
            let parsed = from_str(&a);
            assert_eq!(parsed.ok(), Some(lab));
        }
    }

    #[test]
    fn device_models_and_cable_types_roundtrip() {
        let mut lab = sample_lab();
        let laptop = lab.add_node_model(
            crate::model::DeviceModel::Laptop,
            Some("LAP1"),
            1200.0,
            200.0,
        );
        assert!(laptop.is_ok());
        if let Ok(laptop) = laptop {
            assert!(
                lab.add_link_with_cable(NodeId(1), laptop, crate::model::CableKind::Crossover,)
                    .is_ok()
            );
        }
        let text = to_string(&lab);
        assert!(text.is_ok());
        if let Ok(text) = text {
            let parsed = from_str(&text);
            assert_eq!(parsed.ok(), Some(lab));
        }
    }

    #[test]
    fn legacy_files_default_models_and_cables() {
        let lab = from_str(
            r#"{
              "format": "networkscraft-lab",
              "formatVersion": 1,
              "nodes": [
                { "id": 0, "kind": "switch", "name": "SW1", "x": 0, "y": 0 },
                { "id": 1, "kind": "host", "name": "PC1", "x": 1, "y": 1 }
              ],
              "links": [ { "id": 0, "a": 0, "b": 1 } ]
            }"#,
        );
        assert!(lab.is_ok());
        if let Ok(lab) = lab {
            assert_eq!(
                lab.nodes.first().map(|node| node.model),
                Some(crate::model::DeviceModel::Switch2960)
            );
            assert_eq!(
                lab.links.first().map(|link| link.cable),
                Some(crate::model::CableKind::StraightThrough)
            );
        }
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"{
          "format": "networkscraft-lab",
          "formatVersion": 1,
          "futureField": { "whatever": true },
          "title": "Future",
          "nodes": [
            { "id": 0, "kind": "host", "name": "PC1", "x": 1.0, "y": 2.0, "ipv4": "10.0.0.1", "colour": "red" }
          ],
          "links": []
        }"#;
        let lab = from_str(text);
        assert!(lab.is_ok());
        if let Ok(lab) = lab {
            assert_eq!(lab.nodes.len(), 1);
            assert_eq!(lab.title, "Future");
        }
    }

    #[test]
    fn missing_optional_fields_default() {
        let lab = from_str(r#"{"format":"networkscraft-lab","formatVersion":1}"#);
        assert!(lab.is_ok());
        if let Ok(lab) = lab {
            assert!(lab.nodes.is_empty());
            assert_eq!(lab.title, "Untitled lab");
        }
    }

    #[test]
    fn wrong_format_and_version_are_rejected() {
        assert!(matches!(
            from_str(r#"{"format":"something-else","formatVersion":1}"#),
            Err(LabError::BadFormat)
        ));
        assert!(matches!(
            from_str(r#"{"format":"networkscraft-lab","formatVersion":2}"#),
            Err(LabError::UnsupportedVersion(2))
        ));
        assert!(from_str(r#"{"formatVersion":1}"#).is_err());
    }

    #[test]
    fn truncated_json_is_an_error() {
        assert!(matches!(
            from_str(r#"{"format":"networkscraft-lab""#),
            Err(LabError::Json(_))
        ));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let text = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[
            {"id":1,"kind":"host","name":"A","x":0,"y":0},
            {"id":1,"kind":"host","name":"B","x":0,"y":0}
          ]
        }"#;
        assert!(matches!(from_str(text), Err(LabError::Invalid(_))));
    }

    #[test]
    fn dangling_and_self_links_are_rejected() {
        let dangling = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[{"id":0,"kind":"host","name":"A","x":0,"y":0}],
          "links":[{"id":0,"a":0,"b":9}]
        }"#;
        assert!(from_str(dangling).is_err());
        let self_link = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[{"id":0,"kind":"host","name":"A","x":0,"y":0}],
          "links":[{"id":0,"a":0,"b":0}]
        }"#;
        assert!(from_str(self_link).is_err());
    }

    #[test]
    fn bad_values_are_rejected() {
        let bad_ip = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[{"id":0,"kind":"host","name":"A","x":0,"y":0,"ipv4":"999.0.0.1"}]
        }"#;
        assert!(from_str(bad_ip).is_err());

        let huge_coord = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[{"id":0,"kind":"host","name":"A","x":1e300,"y":0}]
        }"#;
        assert!(from_str(huge_coord).is_err());

        let huge_id = r#"{
          "format":"networkscraft-lab","formatVersion":1,
          "nodes":[{"id":1000001,"kind":"host","name":"A","x":0,"y":0}]
        }"#;
        assert!(from_str(huge_id).is_err());
    }

    #[test]
    fn oversized_file_is_rejected_before_reading() {
        let path = temp_lab("oversized");
        let file = std::fs::File::create(&path);
        assert!(file.is_ok());
        if let Ok(file) = file {
            let result = file.set_len(MAX_FILE_BYTES.saturating_add(1));
            assert!(result.is_ok());
        }
        assert!(matches!(load(&path), Err(LabError::TooLarge)));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_and_load_are_atomic_and_roundtrip() {
        let path = temp_lab("roundtrip");
        let lab = sample_lab();
        assert!(save(&path, &lab).is_ok());
        let loaded = load(&path);
        assert_eq!(loaded.ok(), Some(lab));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_validation_leaves_old_file_valid() {
        let path = temp_lab("failed-save");
        let lab = sample_lab();
        assert!(save(&path, &lab).is_ok());

        let mut broken = lab.clone();
        broken.title = "x".repeat(crate::model::MAX_STRING_CHARS.saturating_add(1));
        assert!(save(&path, &broken).is_err());
        assert_eq!(load(&path).ok(), Some(lab));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn alpha_gate_save_open_ping() {
        let path = temp_lab("alpha");
        let lab = sample_lab();
        assert!(save(&path, &lab).is_ok());
        let loaded = load(&path);
        assert!(loaded.is_ok());
        if let Ok(loaded) = loaded {
            let (outcome, events) = ping(&loaded, NodeId(2), "10.0.0.2");
            assert!(outcome.is_success());
            assert!(
                events
                    .iter()
                    .any(|event| event.summary.contains("ICMP echo reply"))
            );
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn hostile_lab_never_panic() {
        let hostile = [
            "",
            "null",
            "[]",
            "{}",
            "{\"format\":\"networkscraft-lab\",\"formatVersion\":1,\"nodes\":[{}]}",
            "{\"format\":\"networkscraft-lab\",\"formatVersion\":1,\"nodes\":[{\"id\":99999999999999999999}]}",
            "{\"format\":\"networkscraft-lab\",\"formatVersion\":1,\"links\":[{\"a\":1,\"b\":2}]}",
            "{\"format\":\"networkscraft-lab\",\"formatVersion\":1,\"nodes\":\"nope\"}",
            "not json at all",
        ];
        for text in hostile {
            let _ = from_str(text);
        }
        // A small deterministic fuzz: no panic on arbitrary bytes-as-text.
        let mut seed = 0x1234_5678_9abc_def0u64;
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = (seed % 128) as usize;
            let mut text = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let byte = (seed % 96) as u8 + 32;
                text.push(char::from(byte));
            }
            let _ = from_str(&text);
        }
    }

    #[test]
    fn extension_helper_is_pure() {
        assert_eq!(
            with_lab_extension(Path::new("lab")),
            PathBuf::from("lab.nclab")
        );
        assert_eq!(
            with_lab_extension(Path::new("lab.nclab")),
            PathBuf::from("lab.nclab")
        );
        assert_eq!(
            with_lab_extension(Path::new("lab.txt")),
            PathBuf::from("lab.txt")
        );
    }

    #[test]
    fn model_limits_are_single_source_of_truth() {
        // Compile-time sanity: the public caps are the documented caps.
        assert_eq!(crate::model::MAX_NODES, 4096);
        assert_eq!(crate::model::MAX_LINKS, 8192);
        assert_eq!(crate::model::MAX_ID, 1_000_000);
        assert_eq!(MAX_FILE_BYTES, 8388608);
    }

    #[test]
    fn description_mentions_counts() {
        let text = describe(&sample_lab());
        assert!(text.contains("4 devices"));
        assert!(text.contains("2 hosts"));
        assert!(text.contains("3 links"));
    }
}
