//! L2 — deterministic, bounded simulation.
//!
//! The first classroom slice is deliberately honest about what it does: switches learn source MACs,
//! flood unknown unicast, and forward known unicast; routers terminate the broadcast domain. Hosts
//! do ARP and ICMP. No routing, no wall-clock timing, no randomness.

use crate::model::{DeviceKind, LinkId, NodeId, Topology, mac_for, parse_ipv4, subnet24};
use std::collections::{HashMap, HashSet, VecDeque};

/// A traversal may visit at most this many devices. Protects against hostile graphs.
pub const MAX_GRAPH_VISITS: usize = 4096;
/// A single operation emits at most this many events.
pub const MAX_EVENTS: usize = 256;
/// A switch may hold at most this many MAC entries in the classroom slice.
pub const MAX_MAC_PER_SWITCH: usize = 512;
/// A frame may cross at most this many layer-2 links.
pub const MAX_L2_STEPS: usize = 2048;

/// A short, user-facing trace line. Prefixes are stable so scripts can assert on them.
#[derive(Clone, Debug, PartialEq)]
pub struct SimEvent {
    pub seq: u32,
    pub summary: String,
    pub link_id: Option<u64>,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Arp,
    Learn,
    Icmp,
    Info,
    Error,
}

/// Result of a ping. Never panics, never blocks; every outcome carries a clear trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PingOutcome {
    Success { rtt_ms: u32 },
    Unreachable,
    NoRoute,
    NotHost,
    UnknownTarget,
    DifferentSubnet,
}

impl PingOutcome {
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }

    pub const fn label(&self) -> &'static str {
        match self {
            Self::Success { .. } => "success",
            Self::Unreachable => "unreachable",
            Self::NoRoute => "no route",
            Self::NotHost => "not a host",
            Self::UnknownTarget => "unknown target",
            Self::DifferentSubnet => "different subnet",
        }
    }

    pub const fn rtt_ms(&self) -> Option<u32> {
        match self {
            Self::Success { rtt_ms } => Some(*rtt_ms),
            _ => None,
        }
    }
}

/// Ping a host by IPv4 address.
pub fn ping(topology: &Topology, source: NodeId, target_ip: &str) -> (PingOutcome, Vec<SimEvent>) {
    let mut trace = Trace::new();
    let Some(target_addr) = parse_ipv4(target_ip) else {
        trace.push(
            format!("invalid target address {target_ip}"),
            None,
            EventKind::Error,
        );
        return (PingOutcome::UnknownTarget, trace.finish());
    };

    let Some(source_node) = topology.node(source) else {
        trace.push("source device is missing", None, EventKind::Error);
        return (PingOutcome::NotHost, trace.finish());
    };

    let target = topology
        .nodes
        .iter()
        .find(|node| node.ipv4.as_deref().and_then(parse_ipv4) == Some(target_addr));

    let Some(target_node) = target else {
        trace.push(
            format!("no device has address {target_addr}"),
            None,
            EventKind::Error,
        );
        return (PingOutcome::UnknownTarget, trace.finish());
    };

    if source == target_node.id {
        trace.push(
            format!("ICMP echo request {target_addr} -> {target_addr} (loopback)"),
            None,
            EventKind::Icmp,
        );
        trace.push(
            format!("ICMP echo reply {target_addr} -> {target_addr} (loopback)"),
            None,
            EventKind::Icmp,
        );
        return (PingOutcome::Success { rtt_ms: 0 }, trace.finish());
    }

    if source_node.kind != DeviceKind::Host {
        trace.push(
            format!(
                "{} is not a host; select a host to ping from",
                source_node.name
            ),
            None,
            EventKind::Error,
        );
        return (PingOutcome::NotHost, trace.finish());
    }
    if target_node.kind != DeviceKind::Host {
        trace.push(
            format!("{} is not a host", target_node.name),
            None,
            EventKind::Error,
        );
        return (PingOutcome::NotHost, trace.finish());
    }

    let Some(source_addr) = source_node.ipv4.as_deref().and_then(parse_ipv4) else {
        trace.push(
            format!("{} has no IPv4 address", source_node.name),
            None,
            EventKind::Error,
        );
        return (PingOutcome::NotHost, trace.finish());
    };

    if subnet24(source_addr) != subnet24(target_addr) {
        trace.push(
            format!(
                "no route from {source_addr} to {target_addr} — different subnet (routing is not configured)"
            ),
            None,
            EventKind::Error,
        );
        return (PingOutcome::DifferentSubnet, trace.finish());
    }

    let path = find_l2_path(topology, source, target_node.id);
    if path.nodes.is_empty() {
        if let Some(router) = path.blocked_by {
            let name = topology
                .node(router)
                .map_or("router", |node| node.name.as_str());
            trace.push(
                format!("{name} terminates the L2 domain — routing is not configured"),
                None,
                EventKind::Error,
            );
            return (PingOutcome::Unreachable, trace.finish());
        }
        if path.visit_limit {
            trace.push(
                "graph visit limit reached — trace truncated",
                None,
                EventKind::Error,
            );
        } else {
            trace.push(
                format!(
                    "no layer-2 path from {} to {}",
                    source_node.name, target_node.name
                ),
                None,
                EventKind::Error,
            );
        }
        return (PingOutcome::Unreachable, trace.finish());
    }

    if path.links.len() > MAX_L2_STEPS {
        trace.push(
            "layer-2 path exceeds the step limit",
            None,
            EventKind::Error,
        );
        return (PingOutcome::Unreachable, trace.finish());
    }

    let switches: Vec<&crate::model::Node> = path
        .nodes
        .iter()
        .filter_map(|id| topology.node(*id))
        .filter(|node| node.kind == DeviceKind::Switch)
        .collect();

    let source_mac = mac_for(source, 0);
    let target_mac = mac_for(target_node.id, 0);
    let first_link = path.links.first().copied();
    let last_link = path.links.last().copied();

    trace.push(
        format!("ARP who-has {target_addr}"),
        first_link.map(|link| link.0),
        EventKind::Arp,
    );

    for (index, switch) in switches.iter().enumerate() {
        let Some(link) = path.links.get(index) else {
            break;
        };
        trace.push(
            format!("{} learned {source_mac} on link L{}", switch.name, link.0),
            Some(link.0),
            EventKind::Learn,
        );
        if let Some(next) = path.links.get(index.saturating_add(1)) {
            trace.push(
                format!("{} flooded ARP to link L{}", switch.name, next.0),
                Some(next.0),
                EventKind::Arp,
            );
        }
    }

    trace.push(
        format!("ARP reply from {target_addr} ({target_mac})"),
        last_link.map(|link| link.0),
        EventKind::Arp,
    );

    for (index, switch) in switches.iter().enumerate() {
        let Some(link) = path.links.get(index.saturating_add(1)) else {
            break;
        };
        trace.push(
            format!("{} learned {target_mac} on link L{}", switch.name, link.0),
            Some(link.0),
            EventKind::Learn,
        );
    }

    trace.push(
        format!("ICMP echo request {source_addr} -> {target_addr}"),
        first_link.map(|link| link.0),
        EventKind::Icmp,
    );
    trace.push(
        format!("ICMP echo reply {target_addr} -> {source_addr}"),
        last_link.map(|link| link.0),
        EventKind::Icmp,
    );

    let rtt_ms = 1u32.saturating_add((switches.len() as u32).saturating_mul(2));
    let hops = switches.len().saturating_add(1);
    trace.push(
        format!(
            "ping complete — {hops} hop{} in {rtt_ms} ms",
            if hops == 1 { "" } else { "s" }
        ),
        last_link.map(|link| link.0),
        EventKind::Info,
    );

    (PingOutcome::Success { rtt_ms }, trace.finish())
}

/// A bounded breadth-first walk over switches only. Routers stop the walk.
#[derive(Clone, Debug, Default)]
struct L2Path {
    nodes: Vec<NodeId>,
    links: Vec<LinkId>,
    blocked_by: Option<NodeId>,
    visit_limit: bool,
}

fn find_l2_path(topology: &Topology, source: NodeId, target: NodeId) -> L2Path {
    if source == target {
        return L2Path {
            nodes: vec![source],
            links: Vec::new(),
            blocked_by: None,
            visit_limit: false,
        };
    }

    let mut queue: VecDeque<NodeId> = VecDeque::new();
    queue.push_back(source);
    let mut seen: HashSet<NodeId> = HashSet::new();
    seen.insert(source);
    let mut parent: HashMap<NodeId, (NodeId, LinkId)> = HashMap::new();
    let mut visits = 0usize;
    let mut found = false;
    let mut blocked_by = None;
    let mut visit_limit = false;

    while let Some(current) = queue.pop_front() {
        visits = visits.saturating_add(1);
        if visits > MAX_GRAPH_VISITS {
            visit_limit = true;
            break;
        }
        for link in &topology.links {
            let other = if link.a == current {
                link.b
            } else if link.b == current {
                link.a
            } else {
                continue;
            };
            if seen.contains(&other) {
                continue;
            }
            if other == target {
                parent.insert(other, (current, link.id));
                seen.insert(other);
                found = true;
                break;
            }
            let Some(node) = topology.node(other) else {
                continue;
            };
            match node.kind {
                DeviceKind::Switch => {
                    seen.insert(other);
                    parent.insert(other, (current, link.id));
                    queue.push_back(other);
                }
                DeviceKind::Router => {
                    blocked_by = Some(other);
                }
                DeviceKind::Host => {}
            }
        }
        if found {
            break;
        }
    }

    if !found {
        return L2Path {
            nodes: Vec::new(),
            links: Vec::new(),
            blocked_by,
            visit_limit,
        };
    }

    let mut rev_nodes = vec![target];
    let mut rev_links = Vec::new();
    let mut current = target;
    let mut guard = 0usize;
    while current != source {
        let Some((previous, link)) = parent.get(&current).copied() else {
            break;
        };
        rev_nodes.push(previous);
        rev_links.push(link);
        current = previous;
        guard = guard.saturating_add(1);
        if guard > MAX_GRAPH_VISITS {
            break;
        }
    }
    rev_nodes.reverse();
    rev_links.reverse();

    L2Path {
        nodes: rev_nodes,
        links: rev_links,
        blocked_by,
        visit_limit,
    }
}

struct Trace {
    events: Vec<SimEvent>,
    truncated: bool,
    seq: u32,
}

impl Trace {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            truncated: false,
            seq: 0,
        }
    }

    fn push(&mut self, summary: impl Into<String>, link_id: Option<u64>, kind: EventKind) {
        if self.events.len().saturating_add(1) >= MAX_EVENTS {
            self.truncated = true;
            return;
        }
        self.events.push(SimEvent {
            seq: self.seq,
            summary: summary.into(),
            link_id,
            kind,
        });
        self.seq = self.seq.saturating_add(1);
    }

    fn finish(mut self) -> Vec<SimEvent> {
        if self.truncated && self.events.len() < MAX_EVENTS {
            self.events.push(SimEvent {
                seq: self.seq,
                summary: "trace truncated — event limit reached".to_string(),
                link_id: None,
                kind: EventKind::Info,
            });
        }
        self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceKind, NodeId, Topology};

    fn build_chain(switches: usize) -> (Topology, NodeId, NodeId) {
        let mut topo = Topology::new("Chain");
        let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 0.0, 0.0);
        let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 1.0, 0.0);
        let (Ok(pc1), Ok(pc2)) = (pc1, pc2) else {
            return (topo, NodeId(0), NodeId(1));
        };
        let mut previous = pc1;
        for index in 0..switches {
            let name = format!("SW{}", index.saturating_add(1));
            let Ok(switch) = topo.add_node(DeviceKind::Switch, Some(&name), index as f32, 1.0)
            else {
                break;
            };
            let _ = topo.add_link(previous, switch);
            previous = switch;
        }
        let _ = topo.add_link(previous, pc2);
        let _ = topo.set_ipv4(pc1, "10.0.0.1");
        let _ = topo.set_ipv4(pc2, "10.0.0.2");
        (topo, pc1, pc2)
    }

    #[test]
    fn same_l2_ping_succeeds_with_a_full_trace() {
        let topo = crate::model::sample_lab();
        let (outcome, events) = ping(&topo, NodeId(2), "10.0.0.2");
        assert!(outcome.is_success());
        assert!(
            events
                .iter()
                .any(|event| event.summary.starts_with("ARP who-has"))
        );
        assert!(events.iter().any(|event| event.summary.contains("learned")));
        assert!(
            events
                .iter()
                .any(|event| event.summary.contains("ICMP echo request"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.summary.contains("ICMP echo reply"))
        );
        assert!(events.len() >= 5);
    }

    #[test]
    fn router_blocks_the_broadcast_domain() {
        let mut topo = Topology::new("Routed");
        let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 0.0, 0.0);
        let router = topo.add_node(DeviceKind::Router, Some("R1"), 1.0, 0.0);
        let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 2.0, 0.0);
        let (Ok(pc1), Ok(router), Ok(pc2)) = (pc1, router, pc2) else {
            return;
        };
        let _ = topo.add_link(pc1, router);
        let _ = topo.add_link(router, pc2);
        let _ = topo.set_ipv4(pc1, "10.0.0.1");
        let _ = topo.set_ipv4(pc2, "10.0.0.2");
        let (outcome, events) = ping(&topo, pc1, "10.0.0.2");
        assert_eq!(outcome, PingOutcome::Unreachable);
        assert!(
            events
                .iter()
                .any(|event| event.summary.contains("terminates"))
        );
    }

    #[test]
    fn different_subnet_is_reported() {
        let mut topo = Topology::new("Subnets");
        let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 0.0, 0.0);
        let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 1.0, 0.0);
        let (Ok(pc1), Ok(pc2)) = (pc1, pc2) else {
            return;
        };
        let _ = topo.set_ipv4(pc1, "10.0.0.1");
        let _ = topo.set_ipv4(pc2, "10.0.1.2");
        let (outcome, events) = ping(&topo, pc1, "10.0.1.2");
        assert_eq!(outcome, PingOutcome::DifferentSubnet);
        assert!(
            events
                .iter()
                .any(|event| event.summary.contains("different subnet"))
        );
    }

    #[test]
    fn loopback_succeeds_with_two_events() {
        let topo = crate::model::sample_lab();
        let (outcome, events) = ping(&topo, NodeId(2), "10.0.0.1");
        assert_eq!(outcome, PingOutcome::Success { rtt_ms: 0 });
        assert_eq!(events.len(), 2);
        assert!(events[0].summary.contains("loopback"));
    }

    #[test]
    fn unknown_target_is_reported() {
        let topo = crate::model::sample_lab();
        let (outcome, events) = ping(&topo, NodeId(2), "10.0.0.99");
        assert_eq!(outcome, PingOutcome::UnknownTarget);
        assert!(events[0].summary.contains("no device has address"));
    }

    #[test]
    fn no_path_is_reported() {
        let mut topo = Topology::new("Islands");
        let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 0.0, 0.0);
        let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 1.0, 0.0);
        let (Ok(pc1), Ok(pc2)) = (pc1, pc2) else {
            return;
        };
        let _ = topo.set_ipv4(pc1, "10.0.0.1");
        let _ = topo.set_ipv4(pc2, "10.0.0.2");
        let (outcome, events) = ping(&topo, pc1, "10.0.0.2");
        assert_eq!(outcome, PingOutcome::Unreachable);
        assert!(
            events
                .iter()
                .any(|event| event.summary.contains("no layer-2 path"))
        );
    }

    #[test]
    fn source_must_be_a_host() {
        let topo = crate::model::sample_lab();
        let (outcome, events) = ping(&topo, NodeId(1), "10.0.0.2");
        assert_eq!(outcome, PingOutcome::NotHost);
        assert!(events[0].summary.contains("not a host"));
    }

    #[test]
    fn event_limit_is_enforced() {
        let (topo, pc1, _pc2) = build_chain(300);
        let (outcome, events) = ping(&topo, pc1, "10.0.0.2");
        assert!(outcome.is_success());
        assert!(events.len() <= MAX_EVENTS);
        assert!(
            events
                .last()
                .is_some_and(|event| event.summary.contains("truncated"))
        );
    }

    #[test]
    fn macs_are_deterministic() {
        let topo = crate::model::sample_lab();
        let (_, first) = ping(&topo, NodeId(2), "10.0.0.2");
        let (_, second) = ping(&topo, NodeId(2), "10.0.0.2");
        assert_eq!(first, second);
    }
}
