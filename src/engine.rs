//! L4 — the session, the command registry and the command line.
//!
//! Every user-visible action is a command. The UI, command line, scripts and headless CLI resolve
//! the same id and JSON params. Commands never open dialogs.

use crate::io;
use crate::model::{CableKind, DeviceKind, DeviceModel, LinkId, NodeId, Topology, parse_ipv4};
use crate::sim::{EventKind, PingOutcome, SimEvent, ping};
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use thiserror::Error;

/// Maximum number of console lines retained in one session.
pub const MAX_LOG_LINES: usize = 200;
/// Maximum number of links kept for the packet animation.
pub const MAX_TRACE_LINKS: usize = 64;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error(transparent)]
    Lab(#[from] io::LabError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Msg(message.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogKind {
    Input,
    Output,
    Error,
    Info,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub kind: LogKind,
    pub text: String,
}

/// Everything one open lab needs. The UI reads these fields and acts only through commands.
#[derive(Clone, Debug)]
pub struct Session {
    pub topology: Topology,
    pub selected: Option<NodeId>,
    pub selected_link: Option<LinkId>,
    pub events: Vec<SimEvent>,
    pub log: Vec<LogLine>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub running: bool,
    pub message: String,
    pub message_kind: MessageKind,
    pub last_outcome: Option<PingOutcome>,
    pub goal_done: bool,
    pub trace_links: Vec<u64>,
    pub trace_generation: u64,
}

impl Session {
    pub fn empty() -> Self {
        let mut session = Self::from_topology(Topology::new("Untitled lab"), None);
        session.message =
            "New lab — add a device from the palette or type adddevice host PC1".into();
        session
    }

    pub fn from_topology(topology: Topology, path: Option<PathBuf>) -> Self {
        let selected = first_host(&topology);
        let mut session = Self {
            topology,
            selected,
            selected_link: None,
            events: Vec::new(),
            log: Vec::new(),
            path,
            dirty: false,
            running: true,
            message: String::new(),
            message_kind: MessageKind::Info,
            last_outcome: None,
            goal_done: false,
            trace_links: Vec::new(),
            trace_generation: 0,
        };
        session.log(LogKind::Info, "NetworksCraft 0.1.0 — M1 alpha");
        session.log(LogKind::Info, io::describe(&session.topology));
        if let Some(id) = selected {
            if let Some(node) = session.topology.node(id) {
                session.message = format!(
                    "Selected {} — type ping <address> to test the lab",
                    node.name
                );
            }
        } else {
            session.message = "Add a device from the palette to begin".into();
        }
        session
    }

    /// First-launch sample: R1–SW1–PC1/PC2, PC1 selected, and a demonstration ping already in the
    /// packet log so the screen explains itself.
    pub fn sample() -> Self {
        let mut session = Self::from_topology(crate::model::sample_lab(), None);
        session.log(LogKind::Info, "Sample lab: R1 · SW1 · PC1 · PC2");
        session.selected = Some(NodeId(2));
        if let Some(node) = session.topology.node(NodeId(2)) {
            session.log(
                LogKind::Info,
                format!(
                    "Selected {} ({})",
                    node.name,
                    node.ipv4.as_deref().unwrap_or("—")
                ),
            );
        }
        let _ = session.cmdline("ping 10.0.0.2");
        session.dirty = false;
        session.message = "Sample lab ready — try ping 10.0.0.2 from PC1".into();
        session.message_kind = MessageKind::Success;
        session
    }

    pub fn campus() -> Self {
        let mut session = Self::from_topology(crate::model::campus_lab(), None);
        session.log(LogKind::Info, "Campus lab loaded");
        session.message = "Campus lab ready — select a host and run a ping".into();
        session.message_kind = MessageKind::Info;
        session
    }

    pub fn log(&mut self, kind: LogKind, text: impl Into<String>) {
        self.log.push(LogLine {
            kind,
            text: text.into(),
        });
        if self.log.len() > MAX_LOG_LINES {
            let drop = self.log.len().saturating_sub(MAX_LOG_LINES);
            self.log.drain(0..drop);
        }
    }

    pub fn set_message(&mut self, kind: MessageKind, text: impl Into<String>) {
        self.message_kind = kind;
        self.message = text.into();
    }

    pub fn file_label(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .map_or_else(|| self.topology.title.clone(), str::to_string)
    }

    pub fn select_first_host(&mut self) {
        self.selected = first_host(&self.topology);
        self.selected_link = None;
    }

    pub fn clear_events(&mut self) {
        self.events.clear();
        self.trace_links.clear();
        self.last_outcome = None;
        self.trace_generation = self.trace_generation.saturating_add(1);
    }

    /// The programmatic command entry point. Never opens a dialog; missing optional params use
    /// defaults. A panic is caught, the previous topology is restored, and an error is returned.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, Error> {
        let Some(spec) = find_command(id) else {
            return Err(Error::msg(format!(
                "unknown command '{id}' — type help for the command list"
            )));
        };
        if !(spec.enabled)(self) {
            return Err(Error::msg(format!(
                "{} is not available right now",
                spec.label
            )));
        }
        self.execute_spec_guarded(spec, params)
    }

    fn execute_spec_guarded(&mut self, spec: &CommandSpec, params: &Value) -> Result<Value, Error> {
        let before_topology = self.topology.clone();
        let before_selected = self.selected;
        let before_link = self.selected_link;
        let before_events = self.events.clone();
        let result = catch_unwind(AssertUnwindSafe(|| (spec.run)(self, params)));
        match result {
            Ok(value) => value,
            Err(_) => {
                self.topology = before_topology;
                self.selected = before_selected;
                self.selected_link = before_link;
                self.events = before_events;
                self.set_message(
                    MessageKind::Error,
                    "The command failed internally; your lab was restored.",
                );
                self.log(
                    LogKind::Error,
                    format!("{} failed internally — lab restored", spec.label),
                );
                Err(Error::msg(format!(
                    "{} failed internally; the lab was restored",
                    spec.label
                )))
            }
        }
    }

    /// Parse and run one line exactly as the terminal does. Returns the command result.
    pub fn cmdline(&mut self, text: &str) -> Result<Value, Error> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(Value::Null);
        }
        self.log(LogKind::Input, format!("› {trimmed}"));

        let tokens = tokenize(trimmed)?;
        let Some(first) = tokens.first() else {
            return Ok(Value::Null);
        };
        let Some(spec) = find_command(first) else {
            let message = format!("unknown command '{first}' — type help for the command list");
            self.log(LogKind::Error, &message);
            self.set_message(MessageKind::Error, &message);
            return Err(Error::msg(message));
        };
        let params = params_for(spec, &tokens[1..])?;
        let result = self.execute(spec.id, &params);
        match result {
            Ok(value) => {
                self.log(LogKind::Output, summarize(spec.id, &value));
                Ok(value)
            }
            Err(error) => {
                self.log(LogKind::Error, error.to_string());
                self.set_message(MessageKind::Error, error.to_string());
                Err(error)
            }
        }
    }

    fn replace_topology(&mut self, topology: Topology, path: Option<PathBuf>) {
        self.topology = topology;
        self.path = path;
        self.selected = first_host(&self.topology);
        self.selected_link = None;
        self.events.clear();
        self.trace_links.clear();
        self.last_outcome = None;
        self.goal_done = false;
        self.dirty = false;
        self.trace_generation = self.trace_generation.saturating_add(1);
    }
}

fn first_host(topology: &Topology) -> Option<NodeId> {
    topology
        .nodes
        .iter()
        .find(|node| node.kind == DeviceKind::Host)
        .map(|node| node.id)
}

pub type RunFn = fn(&mut Session, &Value) -> Result<Value, Error>;

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub menu: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub params: &'static str,
    pub enabled: fn(&Session) -> bool,
    pub run: RunFn,
}

fn always(_: &Session) -> bool {
    true
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "newlab",
        label: "New lab",
        menu: Some("File"),
        shortcut: Some("Ctrl+N"),
        aliases: &["new"],
        params: "{ title?: string }",
        enabled: always,
        run: cmd_newlab,
    },
    CommandSpec {
        id: "sample",
        label: "Load sample lab",
        menu: Some("File > Labs"),
        shortcut: None,
        aliases: &["demo"],
        params: "{}",
        enabled: always,
        run: cmd_sample,
    },
    CommandSpec {
        id: "campus",
        label: "Load campus lab",
        menu: Some("File > Labs"),
        shortcut: None,
        aliases: &["campuslab"],
        params: "{}",
        enabled: always,
        run: cmd_campus,
    },
    CommandSpec {
        id: "adddevice",
        label: "Add device",
        menu: Some("Draw > Device"),
        shortcut: Some("A"),
        aliases: &["add"],
        params: "{ kind?: \"router\"|\"switch\"|\"host\", model?, name?, x?, y? }",
        enabled: always,
        run: cmd_adddevice,
    },
    CommandSpec {
        id: "link",
        label: "Connect devices",
        menu: Some("Draw > Cable"),
        shortcut: Some("C"),
        aliases: &["cable", "connect"],
        params: "{ a: number, b: number, cable? }",
        enabled: always,
        run: cmd_link,
    },
    CommandSpec {
        id: "select",
        label: "Select device",
        menu: Some("Edit"),
        shortcut: None,
        aliases: &["sel"],
        params: "{ nodeId?: number|null }",
        enabled: always,
        run: cmd_select,
    },
    CommandSpec {
        id: "setipv4",
        label: "Set IPv4 address",
        menu: Some("Edit"),
        shortcut: None,
        aliases: &["ip"],
        params: "{ nodeId: number, address: string }",
        enabled: always,
        run: cmd_setipv4,
    },
    CommandSpec {
        id: "rename",
        label: "Rename device",
        menu: Some("Edit"),
        shortcut: None,
        aliases: &["name"],
        params: "{ nodeId: number, name: string }",
        enabled: always,
        run: cmd_rename,
    },
    CommandSpec {
        id: "movedevice",
        label: "Move device",
        menu: None,
        shortcut: None,
        aliases: &["move"],
        params: "{ nodeId: number, x: number, y: number }",
        enabled: always,
        run: cmd_movedevice,
    },
    CommandSpec {
        id: "deletedevice",
        label: "Delete device",
        menu: Some("Edit"),
        shortcut: Some("Delete"),
        aliases: &["delete", "rm"],
        params: "{ nodeId: number }",
        enabled: always,
        run: cmd_deletedevice,
    },
    CommandSpec {
        id: "deletelink",
        label: "Delete cable",
        menu: Some("Edit"),
        shortcut: None,
        aliases: &["rmlink"],
        params: "{ linkId: number }",
        enabled: always,
        run: cmd_deletelink,
    },
    CommandSpec {
        id: "settitle",
        label: "Set lab title",
        menu: Some("Edit"),
        shortcut: None,
        aliases: &["title"],
        params: "{ title: string }",
        enabled: always,
        run: cmd_settitle,
    },
    CommandSpec {
        id: "ping",
        label: "Ping host",
        menu: Some("Simulate"),
        shortcut: Some("F5"),
        aliases: &["p"],
        params: "{ target: string, source?: number }",
        enabled: always,
        run: cmd_ping,
    },
    CommandSpec {
        id: "openlab",
        label: "Open lab…",
        menu: Some("File"),
        shortcut: Some("Ctrl+O"),
        aliases: &["open"],
        params: "{ path: string }",
        enabled: always,
        run: cmd_openlab,
    },
    CommandSpec {
        id: "savelab",
        label: "Save lab",
        menu: Some("File"),
        shortcut: Some("Ctrl+S"),
        aliases: &["save"],
        params: "{ path?: string }",
        enabled: always,
        run: cmd_savelab,
    },
    CommandSpec {
        id: "sim.run",
        label: "Run simulation",
        menu: Some("Simulate"),
        shortcut: Some("F6"),
        aliases: &["run", "play"],
        params: "{}",
        enabled: always,
        run: cmd_sim_run,
    },
    CommandSpec {
        id: "sim.pause",
        label: "Pause simulation",
        menu: Some("Simulate"),
        shortcut: Some("F7"),
        aliases: &["pause"],
        params: "{}",
        enabled: always,
        run: cmd_sim_pause,
    },
    CommandSpec {
        id: "sim.reset",
        label: "Reset packet log",
        menu: Some("Simulate"),
        shortcut: Some("F8"),
        aliases: &["reset"],
        params: "{}",
        enabled: always,
        run: cmd_sim_reset,
    },
    CommandSpec {
        id: "show",
        label: "Show device state",
        menu: Some("Help"),
        shortcut: None,
        aliases: &["sh"],
        params: "{ what: \"ip interface brief\"|\"ip route\"|\"version\" }",
        enabled: always,
        run: cmd_show,
    },
    CommandSpec {
        id: "info",
        label: "Lab information",
        menu: Some("Help"),
        shortcut: None,
        aliases: &["status"],
        params: "{}",
        enabled: always,
        run: cmd_info,
    },
    CommandSpec {
        id: "help",
        label: "Command reference",
        menu: Some("Help"),
        shortcut: Some("F1"),
        aliases: &["?"],
        params: "{}",
        enabled: always,
        run: cmd_help,
    },
    CommandSpec {
        id: "clearlog",
        label: "Clear console",
        menu: None,
        shortcut: None,
        aliases: &["clear"],
        params: "{}",
        enabled: always,
        run: cmd_clearlog,
    },
];

pub fn find_command(id_or_alias: &str) -> Option<&'static CommandSpec> {
    let needle = id_or_alias.trim();
    COMMANDS.iter().find(|spec| {
        spec.id.eq_ignore_ascii_case(needle)
            || spec
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(needle))
    })
}

fn cmd_newlab(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled lab");
    let topology =
        Topology::from_parts(title.to_string(), Vec::new(), Vec::new()).map_err(Error::msg)?;
    let title = topology.title.clone();
    session.replace_topology(topology, None);
    session.log(LogKind::Info, format!("New lab: {title}"));
    session.set_message(
        MessageKind::Success,
        "New lab ready — add a device to begin",
    );
    Ok(json!({ "title": title }))
}

fn cmd_sample(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    let sample = Session::sample();
    let events = sample.events.clone();
    let log = sample.log.clone();
    session.replace_topology(crate::model::sample_lab(), None);
    session.events = events;
    session.log = log;
    session.selected = Some(NodeId(2));
    session.last_outcome = sample.last_outcome;
    session.goal_done = sample.goal_done;
    session.trace_links = sample.trace_links;
    session.trace_generation = session.trace_generation.saturating_add(1);
    session.set_message(
        MessageKind::Success,
        "Sample lab ready — ping 10.0.0.2 from PC1",
    );
    Ok(
        json!({ "title": session.topology.title, "nodes": session.topology.nodes.len(), "links": session.topology.links.len() }),
    )
}

fn cmd_campus(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    let campus = Session::campus();
    session.replace_topology(crate::model::campus_lab(), None);
    session.log = campus.log;
    session.selected = first_host(&session.topology);
    session.set_message(
        MessageKind::Info,
        "Campus lab ready — select a host and run a ping",
    );
    Ok(
        json!({ "title": session.topology.title, "nodes": session.topology.nodes.len(), "links": session.topology.links.len() }),
    )
}

fn cmd_adddevice(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let model = match params.get("model").and_then(Value::as_str) {
        Some(text) => {
            Some(DeviceModel::parse(text).ok_or_else(|| Error::msg("unknown device model"))?)
        }
        None => None,
    };
    let kind = match params.get("kind").and_then(Value::as_str) {
        Some(text) => DeviceKind::parse(text)
            .ok_or_else(|| Error::msg("kind must be router, switch or host"))?,
        None => model.map(DeviceModel::kind).ok_or_else(|| {
            Error::msg("adddevice needs a kind (router, switch, host) or a model")
        })?,
    };
    if let Some(model) = model
        && model.kind() != kind
    {
        return Err(Error::msg("device model does not match its kind"));
    }
    let model = model.unwrap_or(DeviceModel::default_for(kind));
    let name = params.get("name").and_then(Value::as_str);
    let count = session.topology.nodes.len();
    let default_x = 360.0 + (count % 4) as f32 * 210.0;
    let default_y = 230.0 + (count / 4) as f32 * 170.0;
    let x = coordinate_param(params, "x", default_x)?;
    let y = coordinate_param(params, "y", default_y)?;
    if !x.is_finite() || !y.is_finite() {
        return Err(Error::msg("device coordinates must be finite numbers"));
    }
    let id = session
        .topology
        .add_node_model(model, name, x, y)
        .map_err(Error::msg)?;
    session.selected = Some(id);
    session.selected_link = None;
    session.dirty = true;
    let name = session
        .topology
        .node(id)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| "device".to_string());
    session.set_message(MessageKind::Success, format!("Added {name}"));
    Ok(json!({ "nodeId": id.0, "name": name }))
}

fn cmd_link(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let a = node_id_param(params, "a")?;
    let b = node_id_param(params, "b")?;
    let cable = match params.get("cable").and_then(Value::as_str) {
        Some(text) => CableKind::parse(text).ok_or_else(|| Error::msg("unknown cable type"))?,
        None => CableKind::StraightThrough,
    };
    let id = session
        .topology
        .add_link_with_cable(a, b, cable)
        .map_err(Error::msg)?;
    session.dirty = true;
    session.set_message(
        MessageKind::Success,
        format!("Cable L{} created ({})", id.0, cable.label()),
    );
    Ok(json!({ "linkId": id.0, "cable": cable.as_str() }))
}

fn cmd_select(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let selected = match params.get("nodeId") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let id = value
                .as_u64()
                .map(NodeId)
                .ok_or_else(|| Error::msg("nodeId must be a whole number or null"))?;
            if session.topology.node(id).is_none() {
                return Err(Error::msg("device not found"));
            }
            Some(id)
        }
    };
    session.selected = selected;
    session.selected_link = None;
    let message = match selected.and_then(|id| session.topology.node(id)) {
        Some(node) => format!("Selected {} ({})", node.name, node.model.label()),
        None => "Selection cleared".to_string(),
    };
    session.set_message(MessageKind::Info, message);
    Ok(json!({ "selected": selected.map(|id| id.0) }))
}

fn cmd_setipv4(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let id = node_id_param(params, "nodeId")?;
    let address = params
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("address must be a string"))?;
    if parse_ipv4(address).is_none() {
        return Err(Error::msg("target must be a valid IPv4 address"));
    }
    session.topology.set_ipv4(id, address).map_err(Error::msg)?;
    session.dirty = true;
    let canonical = session
        .topology
        .node(id)
        .and_then(|node| node.ipv4.clone())
        .unwrap_or_else(|| address.to_string());
    session.set_message(MessageKind::Success, format!("Address set to {canonical}"));
    Ok(json!({ "nodeId": id.0, "address": canonical }))
}

fn cmd_rename(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let id = node_id_param(params, "nodeId")?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("name must be a string"))?;
    session.topology.set_name(id, name).map_err(Error::msg)?;
    session.dirty = true;
    let name = session
        .topology
        .node(id)
        .map(|node| node.name.clone())
        .unwrap_or_else(|| name.to_string());
    session.set_message(MessageKind::Success, format!("Renamed to {name}"));
    Ok(json!({ "nodeId": id.0, "name": name }))
}

fn cmd_movedevice(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let id = node_id_param(params, "nodeId")?;
    let x = number_param(params, "x").ok_or_else(|| Error::msg("x must be a number"))?;
    let y = number_param(params, "y").ok_or_else(|| Error::msg("y must be a number"))?;
    session.topology.move_node(id, x, y).map_err(Error::msg)?;
    session.dirty = true;
    Ok(json!({ "nodeId": id.0, "x": x, "y": y }))
}

fn cmd_deletedevice(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let id = node_id_param(params, "nodeId")?;
    let name = session
        .topology
        .node(id)
        .map(|node| node.name.clone())
        .ok_or_else(|| Error::msg("device not found"))?;
    let links_before = session.topology.links.len();
    if !session.topology.remove_node(id) {
        return Err(Error::msg("device not found"));
    }
    let removed_links = links_before.saturating_sub(session.topology.links.len());
    if session.selected == Some(id) {
        session.selected = None;
    }
    session.selected_link = None;
    session.clear_events();
    session.dirty = true;
    session.set_message(
        MessageKind::Success,
        format!("Deleted {name} ({removed_links} cables removed)"),
    );
    Ok(json!({ "nodeId": id.0, "removedLinks": removed_links }))
}

fn cmd_deletelink(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let link_id = params
        .get("linkId")
        .and_then(Value::as_u64)
        .map(LinkId)
        .ok_or_else(|| Error::msg("linkId must be a whole number"))?;
    if !session.topology.remove_link(link_id) {
        return Err(Error::msg("cable not found"));
    }
    if session.selected_link == Some(link_id) {
        session.selected_link = None;
    }
    session.dirty = true;
    session.set_message(
        MessageKind::Success,
        format!("Cable L{} removed", link_id.0),
    );
    Ok(json!({ "linkId": link_id.0 }))
}

fn cmd_settitle(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let title = params
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("title must be a string"))?;
    session.topology.set_title(title).map_err(Error::msg)?;
    session.dirty = true;
    let title = session.topology.title.clone();
    session.set_message(MessageKind::Success, format!("Lab title set to {title}"));
    Ok(json!({ "title": title }))
}

fn cmd_ping(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let target = params
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("target must be a valid IPv4 address"))?;
    let canonical = parse_ipv4(target)
        .ok_or_else(|| Error::msg("target must be a valid IPv4 address"))?
        .to_string();
    let source = match params.get("source") {
        Some(value) => value
            .as_u64()
            .map(NodeId)
            .ok_or_else(|| Error::msg("source must be a device id"))?,
        None => session
            .selected
            .ok_or_else(|| Error::msg("select a host on the topology, then ping"))?,
    };
    let source_node = session
        .topology
        .node(source)
        .ok_or_else(|| Error::msg("select a host on the topology, then ping"))?;
    if source_node.kind != DeviceKind::Host {
        return Err(Error::msg("select a host on the topology, then ping"));
    }
    if source_node.ipv4.is_none() {
        return Err(Error::msg("the selected host has no IPv4 address"));
    }
    let source_name = source_node.name.clone();

    let (outcome, events) = ping(&session.topology, source, &canonical);
    session.events = events.clone();
    session.trace_links.clear();
    for event in &events {
        if let Some(link) = event.link_id
            && !session.trace_links.contains(&link)
            && session.trace_links.len() < MAX_TRACE_LINKS
        {
            session.trace_links.push(link);
        }
    }
    session.trace_generation = session.trace_generation.saturating_add(1);
    session.last_outcome = Some(outcome.clone());
    if outcome.is_success() {
        session.goal_done = true;
        session.set_message(
            MessageKind::Success,
            format!(
                "Ping {} — success ({} ms)",
                canonical,
                outcome.rtt_ms().unwrap_or(0)
            ),
        );
    } else {
        session.set_message(
            MessageKind::Error,
            format!("Ping {canonical} — {}", outcome.label()),
        );
    }

    let event_values: Vec<Value> = events
        .iter()
        .map(|event| {
            json!({
                "seq": event.seq,
                "summary": event.summary,
                "linkId": event.link_id,
                "kind": event_kind(event.kind),
            })
        })
        .collect();

    Ok(json!({
        "outcome": outcome.label(),
        "rttMs": outcome.rtt_ms(),
        "sourceNodeId": source.0,
        "sourceName": source_name,
        "target": canonical,
        "events": event_values,
    }))
}

fn cmd_openlab(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| Error::msg("openlab needs a file path"))?;
    let path = PathBuf::from(path);
    let topology = io::load(&path)?;
    let title = topology.title.clone();
    let nodes = topology.nodes.len();
    let links = topology.links.len();
    session.replace_topology(topology, Some(path.clone()));
    session.set_message(
        MessageKind::Success,
        format!("Opened {title} — {nodes} devices, {links} links"),
    );
    Ok(
        json!({ "path": path.display().to_string(), "title": title, "nodes": nodes, "links": links }),
    )
}

fn cmd_savelab(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => session
            .path
            .clone()
            .ok_or_else(|| Error::msg("choose a file path first"))?,
    };
    io::save(&path, &session.topology)?;
    session.path = Some(path.clone());
    session.dirty = false;
    session.set_message(MessageKind::Success, format!("Saved {}", path.display()));
    Ok(json!({ "path": path.display().to_string() }))
}

fn cmd_sim_run(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    session.running = true;
    session.set_message(MessageKind::Info, "Simulation running");
    Ok(json!({ "running": true }))
}

fn cmd_sim_pause(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    session.running = false;
    session.set_message(
        MessageKind::Info,
        "Simulation paused — packet animation frozen",
    );
    Ok(json!({ "running": false }))
}

fn cmd_sim_reset(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    session.clear_events();
    session.goal_done = false;
    session.set_message(MessageKind::Info, "Packet log cleared");
    Ok(json!({ "events": 0 }))
}

fn cmd_show(session: &mut Session, params: &Value) -> Result<Value, Error> {
    let what = params
        .get("what")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let id = session
        .selected
        .ok_or_else(|| Error::msg("select a device first"))?;
    let node = session
        .topology
        .node(id)
        .cloned()
        .ok_or_else(|| Error::msg("select a device first"))?;
    let ports = session.topology.ports(id).len();
    session.set_message(MessageKind::Info, format!("show {what}"));
    let text = match what.as_str() {
        "ip interface brief" | "ip int brief" | "interfaces" | "interface" => {
            let address = node.ipv4.as_deref().unwrap_or("unassigned");
            let status = if ports > 0 { "up" } else { "down" };
            let prefix = if node.ipv4.is_some() { "/24" } else { "" };
            format!(
                "Interface  IP-Address       Status  Protocol\n{:<10} {:<15} {:<7} {}\n{} cable{} attached",
                "eth0",
                format!("{address}{prefix}"),
                status,
                if ports > 0 { "up" } else { "down" },
                ports,
                if ports == 1 { "" } else { "s" }
            )
        }
        "ip route" | "route" | "routes" => match session.topology.ipv4_of(id) {
            Some(address) if node.kind == DeviceKind::Host => {
                let octets = address.octets();
                format!(
                    "Codes: C connected\nC    {}.{}.{}.0/24 is directly connected, eth0",
                    octets[0], octets[1], octets[2]
                )
            }
            _ => "No routes. Routing is not configured in this M1 slice.".to_string(),
        },
        "version" => "NetworksCraft 0.1.0 — M1 alpha\nnetworkscraft-lab format v1".to_string(),
        _ => {
            return Err(Error::msg(
                "show supports: ip interface brief, ip route, version",
            ));
        }
    };
    Ok(json!({ "text": text }))
}

fn cmd_info(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    let hosts = session
        .topology
        .nodes
        .iter()
        .filter(|node| node.kind == DeviceKind::Host)
        .count();
    Ok(json!({
        "title": session.topology.title,
        "nodes": session.topology.nodes.len(),
        "links": session.topology.links.len(),
        "hosts": hosts,
        "selected": session.selected.map(|id| id.0),
        "path": session.path.as_ref().map(|path| path.display().to_string()),
        "dirty": session.dirty,
        "events": session.events.len(),
    }))
}

fn cmd_help(_session: &mut Session, _params: &Value) -> Result<Value, Error> {
    let entries: Vec<Value> = COMMANDS
        .iter()
        .map(|spec| {
            json!({
                "id": spec.id,
                "label": spec.label,
                "aliases": spec.aliases,
                "params": spec.params,
            })
        })
        .collect();
    Ok(json!(entries))
}

fn cmd_clearlog(session: &mut Session, _params: &Value) -> Result<Value, Error> {
    session.log.clear();
    session.set_message(MessageKind::Info, "Console cleared");
    Ok(json!(null))
}

fn event_kind(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Arp => "arp",
        EventKind::Learn => "learn",
        EventKind::Icmp => "icmp",
        EventKind::Info => "info",
        EventKind::Error => "error",
    }
}

fn number_param(params: &Value, key: &str) -> Option<f32> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn coordinate_param(params: &Value, key: &str, default: f32) -> Result<f32, Error> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_f64()
            .map(|number| number as f32)
            .filter(|number| number.is_finite())
            .ok_or_else(|| Error::msg(format!("{key} must be a finite number"))),
    }
}

fn node_id_param(params: &Value, key: &str) -> Result<NodeId, Error> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(NodeId)
        .ok_or_else(|| Error::msg(format!("{key} must be a whole number")))
}

fn tokenize(text: &str) -> Result<Vec<String>, Error> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for ch in text.chars() {
        match quote {
            Some(active) => {
                if ch == active {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '"' | '\'' => {
                    quote = Some(ch);
                    started = true;
                }
                ch if ch.is_whitespace() => {
                    if started {
                        tokens.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                ch => {
                    current.push(ch);
                    started = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err(Error::msg("unterminated quote — close the quoted text"));
    }
    if started {
        tokens.push(current);
    }
    Ok(tokens)
}

fn params_for(spec: &CommandSpec, args: &[String]) -> Result<Value, Error> {
    match spec.id {
        "newlab" => {
            if args.is_empty() {
                Ok(json!({}))
            } else {
                Ok(json!({ "title": args.join(" ") }))
            }
        }
        "sample" | "campus" | "sim.run" | "sim.pause" | "sim.reset" | "info" | "help"
        | "clearlog" => Ok(json!({})),
        "adddevice" => {
            if args.is_empty() {
                return Err(Error::msg(
                    "adddevice needs a kind or model: router, switch, host, laptop, server…",
                ));
            }
            let kind = args[0].clone();
            let mut name: Option<String> = None;
            let mut coords: Option<(f32, f32)> = None;
            if args.len() >= 3 {
                let x = parse_f32_arg(&args[args.len() - 2], "x")?;
                let y = parse_f32_arg(&args[args.len() - 1], "y")?;
                coords = Some((x, y));
                if args.len() > 3 {
                    name = Some(args[1..args.len() - 2].join(" "));
                }
            } else if args.len() == 2 {
                name = Some(args[1].clone());
            }
            let mut params = if DeviceKind::parse(&kind).is_some() {
                json!({ "kind": kind })
            } else {
                json!({ "model": kind })
            };
            if let Some(name) = name {
                params["name"] = json!(name);
            }
            if let Some((x, y)) = coords {
                params["x"] = json!(x);
                params["y"] = json!(y);
            }
            Ok(params)
        }
        "link" => {
            if args.len() != 2 {
                return Err(Error::msg("link needs two device ids: link <a> <b>"));
            }
            Ok(json!({
                "a": parse_u64_arg(&args[0], "a")?,
                "b": parse_u64_arg(&args[1], "b")?,
            }))
        }
        "select" => {
            if args.is_empty() {
                Ok(json!({ "nodeId": null }))
            } else if args.len() == 1 {
                Ok(json!({ "nodeId": parse_u64_arg(&args[0], "nodeId")? }))
            } else {
                Err(Error::msg("select takes one device id, or no id to clear"))
            }
        }
        "setipv4" => {
            if args.len() != 2 {
                return Err(Error::msg("setipv4 needs a device id and an address"));
            }
            Ok(json!({
                "nodeId": parse_u64_arg(&args[0], "nodeId")?,
                "address": args[1],
            }))
        }
        "rename" => {
            if args.len() < 2 {
                return Err(Error::msg("rename needs a device id and a name"));
            }
            Ok(json!({
                "nodeId": parse_u64_arg(&args[0], "nodeId")?,
                "name": args[1..].join(" "),
            }))
        }
        "movedevice" => {
            if args.len() != 3 {
                return Err(Error::msg("movedevice needs an id, x and y"));
            }
            Ok(json!({
                "nodeId": parse_u64_arg(&args[0], "nodeId")?,
                "x": parse_f32_arg(&args[1], "x")?,
                "y": parse_f32_arg(&args[2], "y")?,
            }))
        }
        "deletedevice" => {
            if args.len() != 1 {
                return Err(Error::msg("deletedevice needs one device id"));
            }
            Ok(json!({ "nodeId": parse_u64_arg(&args[0], "nodeId")? }))
        }
        "deletelink" => {
            if args.len() != 1 {
                return Err(Error::msg("deletelink needs one cable id"));
            }
            Ok(json!({ "linkId": parse_u64_arg(&args[0], "linkId")? }))
        }
        "settitle" => {
            if args.is_empty() {
                return Err(Error::msg("settitle needs a title"));
            }
            Ok(json!({ "title": args.join(" ") }))
        }
        "show" => {
            if args.is_empty() {
                return Err(Error::msg(
                    "show needs a topic, for example: show ip interface brief",
                ));
            }
            Ok(json!({ "what": args.join(" ") }))
        }
        "ping" => {
            if args.is_empty() {
                return Err(Error::msg("ping needs a target IPv4 address"));
            }
            if args.len() == 1 {
                Ok(json!({ "target": args[0] }))
            } else if args.len() == 2 {
                Ok(json!({
                    "source": parse_u64_arg(&args[0], "source")?,
                    "target": args[1],
                }))
            } else {
                Err(Error::msg(
                    "ping takes a target address, or a source id and a target address",
                ))
            }
        }
        "openlab" => {
            if args.is_empty() {
                return Err(Error::msg("openlab needs a file path"));
            }
            Ok(json!({ "path": args.join(" ") }))
        }
        "savelab" => {
            if args.is_empty() {
                Ok(json!({}))
            } else {
                Ok(json!({ "path": args.join(" ") }))
            }
        }
        _ => Ok(json!({})),
    }
}

fn parse_u64_arg(text: &str, what: &str) -> Result<u64, Error> {
    text.parse::<u64>()
        .map_err(|_| Error::msg(format!("{what} must be a whole number")))
}

fn parse_f32_arg(text: &str, what: &str) -> Result<f32, Error> {
    text.parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| Error::msg(format!("{what} must be a finite number")))
}

fn summarize(id: &str, value: &Value) -> String {
    match id {
        "ping" => {
            let outcome = value
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let target = value
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("target");
            if let Some(rtt) = value.get("rttMs").and_then(Value::as_u64) {
                format!("ping {target} — {outcome} ({rtt} ms)")
            } else {
                format!("ping {target} — {outcome}")
            }
        }
        "newlab" => format!(
            "new lab: {}",
            value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled lab")
        ),
        "sample" | "campus" => format!(
            "loaded {} ({} devices, {} links)",
            value.get("title").and_then(Value::as_str).unwrap_or("lab"),
            value.get("nodes").and_then(Value::as_u64).unwrap_or(0),
            value.get("links").and_then(Value::as_u64).unwrap_or(0)
        ),
        "rename" => format!(
            "renamed device {} to {}",
            value.get("nodeId").and_then(Value::as_u64).unwrap_or(0),
            value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("device")
        ),
        "settitle" => format!(
            "lab renamed to {}",
            value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled lab")
        ),
        "deletedevice" => format!(
            "deleted device {} ({} cables removed)",
            value.get("nodeId").and_then(Value::as_u64).unwrap_or(0),
            value
                .get("removedLinks")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        ),
        "deletelink" => format!(
            "removed cable L{}",
            value.get("linkId").and_then(Value::as_u64).unwrap_or(0)
        ),
        "movedevice" => format!(
            "moved device {} to {:.0}, {:.0}",
            value.get("nodeId").and_then(Value::as_u64).unwrap_or(0),
            value.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            value.get("y").and_then(Value::as_f64).unwrap_or(0.0)
        ),
        "clearlog" => "console cleared".to_string(),
        "adddevice" => format!(
            "added {} (device {})",
            value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("device"),
            value.get("nodeId").and_then(Value::as_u64).unwrap_or(0)
        ),
        "link" => format!(
            "cable L{} created",
            value.get("linkId").and_then(Value::as_u64).unwrap_or(0)
        ),
        "select" => match value.get("selected").and_then(Value::as_u64) {
            Some(id) => format!("selected device {id}"),
            None => "selection cleared".to_string(),
        },
        "setipv4" => format!(
            "address {} set on device {}",
            value.get("address").and_then(Value::as_str).unwrap_or("—"),
            value.get("nodeId").and_then(Value::as_u64).unwrap_or(0)
        ),
        "savelab" => format!(
            "saved to {}",
            value.get("path").and_then(Value::as_str).unwrap_or("—")
        ),
        "openlab" => format!(
            "opened {} ({} devices, {} links)",
            value.get("title").and_then(Value::as_str).unwrap_or("lab"),
            value.get("nodes").and_then(Value::as_u64).unwrap_or(0),
            value.get("links").and_then(Value::as_u64).unwrap_or(0)
        ),
        "help" => format!(
            "{} commands — type the id or an alias",
            value.as_array().map_or(0, Vec::len)
        ),
        "show" => value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("no output")
            .to_string(),
        "info" => "lab information ready".to_string(),
        "sim.reset" => "packet log cleared".to_string(),
        "sim.run" => "simulation running".to_string(),
        "sim.pause" => "simulation paused".to_string(),
        _ => "ok".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample_lab;

    fn temp_path(name: &str) -> PathBuf {
        let nonce = std::process::id();
        std::env::temp_dir().join(format!("networkscraft-engine-{name}-{nonce}.nclab"))
    }

    #[test]
    fn command_ids_and_aliases_are_unique() {
        let mut seen: Vec<String> = Vec::new();
        for spec in COMMANDS {
            for name in std::iter::once(spec.id).chain(spec.aliases.iter().copied()) {
                let key = name.to_ascii_lowercase();
                assert!(!seen.contains(&key), "duplicate command name: {key}");
                seen.push(key);
            }
        }
    }

    #[test]
    fn add_link_ping_roundtrip_through_commands() {
        let mut session = Session::empty();
        let switch = session.execute(
            "adddevice",
            &json!({"kind":"switch","name":"SW1","x":400.0,"y":300.0}),
        );
        let pc1 = session.execute(
            "adddevice",
            &json!({"kind":"host","name":"PC1","x":700.0,"y":200.0}),
        );
        let pc2 = session.execute(
            "adddevice",
            &json!({"kind":"host","name":"PC2","x":700.0,"y":400.0}),
        );
        let (Ok(switch), Ok(pc1), Ok(pc2)) = (switch, pc1, pc2) else {
            return;
        };
        let switch = switch["nodeId"].as_u64().unwrap_or(0);
        let pc1 = pc1["nodeId"].as_u64().unwrap_or(0);
        let pc2 = pc2["nodeId"].as_u64().unwrap_or(0);

        assert!(
            session
                .execute("link", &json!({"a":switch,"b":pc1}))
                .is_ok()
        );
        assert!(
            session
                .execute("link", &json!({"a":switch,"b":pc2}))
                .is_ok()
        );
        assert!(
            session
                .execute("setipv4", &json!({"nodeId":pc1,"address":"10.0.0.1"}))
                .is_ok()
        );
        assert!(
            session
                .execute("setipv4", &json!({"nodeId":pc2,"address":"10.0.0.2"}))
                .is_ok()
        );
        assert!(session.execute("select", &json!({"nodeId":pc1})).is_ok());
        let result = session.execute("ping", &json!({"target":"10.0.0.2"}));
        assert!(result.is_ok());
        if let Ok(result) = result {
            assert_eq!(
                result.get("outcome").and_then(Value::as_str),
                Some("success")
            );
            assert!(
                result
                    .get("events")
                    .and_then(Value::as_array)
                    .is_some_and(|events| !events.is_empty())
            );
        }
        assert!(session.goal_done);
    }

    #[test]
    fn adddevice_accepts_models_and_link_accepts_cables() {
        let mut session = Session::empty();
        let switch = session.execute(
            "adddevice",
            &json!({"kind":"switch","model":"switch3560","x":100.0,"y":100.0}),
        );
        let laptop = session.execute(
            "adddevice",
            &json!({"kind":"host","model":"laptop","x":400.0,"y":100.0}),
        );
        let (Ok(switch), Ok(laptop)) = (switch, laptop) else {
            return;
        };
        let switch_id = switch["nodeId"].as_u64().unwrap_or(0);
        let laptop_id = laptop["nodeId"].as_u64().unwrap_or(0);
        assert_eq!(
            session
                .topology
                .node(NodeId(laptop_id))
                .map(|node| node.model),
            Some(crate::model::DeviceModel::Laptop)
        );
        assert!(
            session
                .execute(
                    "link",
                    &json!({"a":switch_id,"b":laptop_id,"cable":"crossover"}),
                )
                .is_ok()
        );
        assert_eq!(
            session.topology.links.first().map(|link| link.cable),
            Some(crate::model::CableKind::Crossover)
        );
    }

    #[test]
    fn adddevice_rejects_bad_coordinates_and_names() {
        let mut session = Session::empty();
        assert!(
            session
                .execute("adddevice", &json!({"kind":"host","x":1e9,"y":0.0}))
                .is_err()
        );
        assert!(
            session
                .execute("adddevice", &json!({"kind":"host","x":"nope","y":0.0}))
                .is_err()
        );
        assert!(
            session
                .execute("adddevice", &json!({"kind":"host","name":"x".repeat(300)}))
                .is_err()
        );
        assert!(
            session
                .execute("adddevice", &json!({"kind":"firewall"}))
                .is_err()
        );
    }

    #[test]
    fn setipv4_validation_leaves_old_value() {
        let mut session = Session::sample();
        let before = session
            .topology
            .node(NodeId(2))
            .and_then(|node| node.ipv4.clone());
        assert!(
            session
                .execute("setipv4", &json!({"nodeId":2,"address":"not-an-ip"}))
                .is_err()
        );
        let after = session
            .topology
            .node(NodeId(2))
            .and_then(|node| node.ipv4.clone());
        assert_eq!(before, after);
    }

    #[test]
    fn ping_requires_a_selected_host() {
        let mut session = Session::sample();
        assert!(session.execute("select", &json!({"nodeId":1})).is_ok());
        let result = session.execute("ping", &json!({"target":"10.0.0.2"}));
        assert!(result.is_err());
    }

    #[test]
    fn dirty_and_path_transitions() {
        let path = temp_path("dirty");
        let mut session = Session::sample();
        assert!(!session.dirty);
        assert!(
            session
                .execute("savelab", &json!({"path":path.display().to_string()}))
                .is_ok()
        );
        assert!(!session.dirty);
        assert!(
            session
                .execute("adddevice", &json!({"kind":"host"}))
                .is_ok()
        );
        assert!(session.dirty);
        assert!(session.execute("savelab", &json!({})).is_ok());
        assert!(!session.dirty);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn failed_open_leaves_lab_untouched() {
        let mut session = Session::sample();
        let before = session.topology.clone();
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        let result = session.execute("openlab", &json!({"path": path.display().to_string()}));
        assert!(result.is_err());
        assert_eq!(session.topology, before);
    }

    #[test]
    fn cmdline_parses_quotes_and_spaces() {
        let mut session = Session::empty();
        let result = session.cmdline("settitle \"My first lab\"");
        assert!(result.is_ok());
        assert_eq!(session.topology.title, "My first lab");
    }

    #[test]
    fn cmdline_adddevice_and_ping() {
        let mut session = Session::empty();
        assert!(session.cmdline("adddevice switch SW1 300 300").is_ok());
        assert!(session.cmdline("adddevice host PC1 600 200").is_ok());
        assert!(session.cmdline("adddevice host PC2 600 400").is_ok());
        assert!(session.cmdline("link 0 1").is_ok());
        assert!(session.cmdline("link 0 2").is_ok());
        assert!(session.cmdline("setipv4 1 10.0.0.1").is_ok());
        assert!(session.cmdline("setipv4 2 10.0.0.2").is_ok());
        assert!(session.cmdline("select 1").is_ok());
        let result = session.cmdline("ping 10.0.0.2");
        assert!(result.is_ok());
        if let Ok(result) = result {
            assert_eq!(
                result.get("outcome").and_then(Value::as_str),
                Some("success")
            );
        }
    }

    #[test]
    fn delete_device_removes_cables() {
        let mut session = Session::sample();
        let before_links = session.topology.links.len();
        assert!(
            session
                .execute("deletedevice", &json!({"nodeId":1}))
                .is_ok()
        );
        assert!(session.topology.links.len() < before_links);
    }

    #[test]
    fn sim_controls_are_deterministic() {
        let mut session = Session::sample();
        assert!(!session.events.is_empty());
        assert!(session.execute("sim.pause", &json!({})).is_ok());
        assert!(!session.running);
        assert!(session.execute("sim.run", &json!({})).is_ok());
        assert!(session.running);
        assert!(session.execute("sim.reset", &json!({})).is_ok());
        assert!(session.events.is_empty());
    }

    #[test]
    fn show_reads_the_selected_device() {
        let mut session = Session::sample();
        assert!(session.execute("select", &json!({"nodeId":2})).is_ok());
        let brief = session.execute("show", &json!({"what":"ip interface brief"}));
        assert!(brief.is_ok());
        if let Ok(value) = brief {
            let text = value.get("text").and_then(Value::as_str).unwrap_or("");
            assert!(text.contains("10.0.0.1/24"));
            assert!(text.contains("eth0"));
        }
        let route = session.execute("show", &json!({"what":"ip route"}));
        assert!(route.is_ok());
        if let Ok(value) = route {
            assert!(
                value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .contains("10.0.0.0/24")
            );
        }
        assert!(
            session
                .execute("show", &json!({"what":"nonsense"}))
                .is_err()
        );
    }

    #[test]
    fn help_lists_every_command() {
        let mut session = Session::empty();
        let result = session.execute("help", &json!({}));
        assert!(result.is_ok());
        if let Ok(value) = result {
            let Some(entries) = value.as_array() else {
                return;
            };
            assert_eq!(entries.len(), COMMANDS.len());
        }
    }

    #[test]
    fn hostile_params_never_panic() {
        let mut session = Session::sample();
        let values = [
            json!(null),
            json!({}),
            json!({"nodeId": -1, "x": "nan", "y": null, "address": 42, "target": []}),
            json!({"kind": 7, "name": {}, "a": -9, "b": 999999}),
            json!({"path": "\u{0000}\u{0001}", "linkId": 1e30}),
        ];
        for spec in COMMANDS {
            if matches!(spec.id, "openlab" | "savelab") {
                continue;
            }
            for value in &values {
                let _ = session.execute(spec.id, value);
            }
        }
    }

    fn panic_command(_session: &mut Session, _params: &Value) -> Result<Value, Error> {
        std::panic::panic_any("intentional test panic")
    }

    static PANIC_SPEC: CommandSpec = CommandSpec {
        id: "paniclab",
        label: "Panic test",
        menu: None,
        shortcut: None,
        aliases: &[],
        params: "{}",
        enabled: always,
        run: panic_command,
    };

    #[test]
    fn panic_guard_restores_topology() {
        let mut session = Session::from_topology(sample_lab(), None);
        let before = session.topology.clone();
        let result = session.execute_spec_guarded(&PANIC_SPEC, &json!({}));
        assert!(result.is_err());
        assert_eq!(session.topology, before);
        assert_eq!(session.message_kind, MessageKind::Error);
    }
}
