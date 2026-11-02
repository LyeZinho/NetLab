//! Headless runner. Uses the same `Session`, command registry and lab format as the desktop app.
//!
//! Exit 0 only when every scripted command succeeds and every ping succeeds.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use networkscraft::engine::{Session, find_command};
use networkscraft::{io, model};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("run") => run_lab(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("--help" | "-h") | None => {
            usage();
            0
        }
        Some(other) => {
            eprintln!("unknown command '{other}'");
            usage();
            2
        }
    }
}

fn usage() {
    println!(
        r#"networkscraft-cli — headless lab runner

Usage:
  networkscraft-cli run LAB.nclab --script 'ping 10.0.0.2' [--json]
  networkscraft-cli run LAB.nclab --script-file checks.txt [--json]
  networkscraft-cli info LAB.nclab [--json]

A script is one command per line (or separated by ';'). Comments start with '#'."#
    );
}
fn run_lab(args: &[String]) -> i32 {
    let mut lab: Option<String> = None;
    let mut script: Option<String> = None;
    let mut script_file: Option<String> = None;
    let mut json_out = false;
    let mut index = 0usize;
    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--script") => {
                script = args.get(index.saturating_add(1)).cloned();
                index = index.saturating_add(2);
            }
            Some("--script-file") => {
                script_file = args.get(index.saturating_add(1)).cloned();
                index = index.saturating_add(2);
            }
            Some("--json") => {
                json_out = true;
                index = index.saturating_add(1);
            }
            Some(value) if lab.is_none() => {
                lab = Some(value.to_string());
                index = index.saturating_add(1);
            }
            Some(value) => {
                eprintln!("unexpected argument: {value}");
                return 2;
            }
            None => break,
        }
    }

    let Some(lab_path) = lab else {
        eprintln!("run needs a lab file");
        return 2;
    };
    if script.is_some() && script_file.is_some() {
        eprintln!("use either --script or --script-file, not both");
        return 2;
    }
    let text = match (script, script_file) {
        (Some(text), None) => text,
        (None, Some(path)) => match read_bounded(Path::new(&path)) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        },
        (None, None) => {
            eprintln!("run needs --script or --script-file");
            return 2;
        }
        (Some(_), Some(_)) => return 2,
    };

    let topology = match io::load(Path::new(&lab_path)) {
        Ok(topology) => topology,
        Err(error) => {
            eprintln!("could not open {lab_path}: {error}");
            return 1;
        }
    };
    let mut session = Session::from_topology(topology, Some(PathBuf::from(&lab_path)));
    let mut steps: Vec<Value> = Vec::new();
    let mut all_ok = true;

    for line in text.lines().flat_map(|line| line.split(';')) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let result = session.cmdline(line);
        let command = line
            .split_whitespace()
            .next()
            .and_then(find_command)
            .map(|spec| spec.id)
            .unwrap_or("unknown");
        let (ok, value_text) = match &result {
            Ok(value) => {
                let ping_ok = if command == "ping" {
                    value.get("outcome").and_then(Value::as_str) == Some("success")
                } else {
                    true
                };
                (ping_ok, value.clone())
            }
            Err(error) => (false, json!({ "error": error.to_string() })),
        };
        if !ok {
            all_ok = false;
        }
        if json_out {
            steps.push(json!({ "command": line, "ok": ok, "result": value_text }));
        } else if ok {
            println!("[ok]   {line}");
        } else {
            let reason = match &value_text {
                Value::Object(map) => {
                    if let Some(error) = map.get("error").and_then(Value::as_str) {
                        error.to_string()
                    } else if let Some(outcome) = map.get("outcome").and_then(Value::as_str) {
                        format!("ping outcome: {outcome}")
                    } else {
                        "command did not succeed".to_string()
                    }
                }
                _ => "command did not succeed".to_string(),
            };
            println!("[FAIL] {line} — {reason}");
        }
    }

    if json_out {
        let report = json!({
            "version": 1,
            "ok": all_ok,
            "lab": lab_path,
            "steps": steps,
        });
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("could not serialise results: {error}");
                return 1;
            }
        }
    } else if all_ok {
        println!(
            "PASS — {} device{} · {} link{} · {} event{}",
            session.topology.nodes.len(),
            plural(session.topology.nodes.len()),
            session.topology.links.len(),
            plural(session.topology.links.len()),
            session.events.len(),
            plural(session.events.len())
        );
    } else {
        println!("FAIL — one or more commands did not succeed");
    }

    if all_ok { 0 } else { 1 }
}

fn info(args: &[String]) -> i32 {
    let mut path: Option<String> = None;
    let mut json_out = false;
    for arg in args {
        if arg == "--json" {
            json_out = true;
        } else if path.is_none() {
            path = Some(arg.clone());
        } else {
            eprintln!("unexpected argument: {arg}");
            return 2;
        }
    }
    let Some(path) = path else {
        eprintln!("info needs a lab file");
        return 2;
    };
    let topology = match io::load(Path::new(&path)) {
        Ok(topology) => topology,
        Err(error) => {
            eprintln!("could not open {path}: {error}");
            return 1;
        }
    };
    let hosts: Vec<&model::Node> = topology
        .nodes
        .iter()
        .filter(|node| node.kind == model::DeviceKind::Host)
        .collect();
    if json_out {
        let nodes: Vec<Value> = topology
            .nodes
            .iter()
            .map(|node| {
                json!({
                    "id": node.id.0,
                    "kind": node.kind.as_str(),
                    "name": node.name,
                    "ipv4": node.ipv4,
                    "x": node.x,
                    "y": node.y,
                })
            })
            .collect();
        let report = json!({
            "version": 1,
            "format": io::FORMAT,
            "formatVersion": io::FORMAT_VERSION,
            "path": path,
            "title": topology.title,
            "nodes": nodes,
            "links": topology.links.iter().map(|link| json!({
                "id": link.id.0, "a": link.a.0, "b": link.b.0
            })).collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("could not serialise info: {error}");
                return 1;
            }
        }
    } else {
        println!("{}", io::describe(&topology));
        println!("file: {path}");
        println!("hosts:");
        for host in hosts {
            println!(
                "  {:<6} id {:<4} {}",
                host.name,
                host.id.0,
                host.ipv4.as_deref().unwrap_or("no address")
            );
        }
        for link in &topology.links {
            let a = topology.node(link.a).map_or("?", |node| node.name.as_str());
            let b = topology.node(link.b).map_or("?", |node| node.name.as_str());
            println!("  L{:<4} {} ↔ {}", link.id.0, a, b);
        }
    }
    0
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if metadata.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "script file is too large (limit {} KiB)",
            MAX_SCRIPT_BYTES / 1024
        ));
    }
    std::fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}
