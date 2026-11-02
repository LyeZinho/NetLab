//! L0 — the NetworksCraft lab model.
//!
//! The model is deliberately small and dependency-free. It owns the invariants that the editor,
//! the simulator and the `.nclab` reader/writer all share. The public file format lives in
//! [`crate::io`], so changing an internal field name here can never silently change a saved lab.

use std::collections::HashSet;
use std::fmt;
use std::net::Ipv4Addr;

/// Hard cap on devices in one lab. The editor refuses to create more, and the file reader rejects
/// files above it.
pub const MAX_NODES: usize = 4096;
/// Hard cap on cables in one lab.
pub const MAX_LINKS: usize = 8192;
/// Highest id a device or cable may have. Counters are derived on load and never stored.
pub const MAX_ID: u64 = 1_000_000;
/// Maximum number of Unicode scalar values in a title or device name.
pub const MAX_STRING_CHARS: usize = 256;
/// Maximum absolute canvas coordinate. Input-derived coordinates are hostile: keep them bounded.
pub const MAX_COORD: f32 = 1_000_000.0;

/// Stable, file-local device id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

/// Stable, file-local cable id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkId(pub u64);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for LinkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The three devices in the first classroom slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Router,
    Switch,
    Host,
}

impl DeviceKind {
    pub const ALL: [DeviceKind; 3] = [DeviceKind::Router, DeviceKind::Switch, DeviceKind::Host];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "router" | "r" => Some(Self::Router),
            "switch" | "sw" | "s" => Some(Self::Switch),
            "host" | "pc" | "h" => Some(Self::Host),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Router => "router",
            Self::Switch => "switch",
            Self::Host => "host",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Router => "Router",
            Self::Switch => "Switch",
            Self::Host => "Host",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Router => "Separates broadcast domains",
            Self::Switch => "Learns MAC addresses",
            Self::Host => "Sends and receives packets",
        }
    }

    pub const fn name_prefix(self) -> &'static str {
        match self {
            Self::Router => "R",
            Self::Switch => "SW",
            Self::Host => "PC",
        }
    }
}

/// A device placed on the canvas. Coordinates are canvas units, not screen pixels and not world
/// metres. The first classroom slice uses one implicit port per cable; the `topology.ports` helper
/// is the seam where named interfaces can be introduced later without changing the file format.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub kind: DeviceKind,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub ipv4: Option<String>,
}

/// A cable between two devices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub id: LinkId,
    pub a: NodeId,
    pub b: NodeId,
}

/// One lab. Equality compares lab content only; the id counters are allocation state.
#[derive(Clone, Debug)]
pub struct Topology {
    pub title: String,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    next_node: u64,
    next_link: u64,
}

impl PartialEq for Topology {
    fn eq(&self, other: &Self) -> bool {
        self.title == other.title && self.nodes == other.nodes && self.links == other.links
    }
}

impl Default for Topology {
    fn default() -> Self {
        Self::new("Untitled lab")
    }
}

impl Topology {
    pub fn new(title: impl Into<String>) -> Self {
        let mut title = title.into();
        if title.trim().is_empty() {
            title = "Untitled lab".to_string();
        }
        Self {
            title,
            nodes: Vec::new(),
            links: Vec::new(),
            next_node: 0,
            next_link: 0,
        }
    }

    /// The single semantic validator used by both the editor and the file format.
    ///
    /// It checks every invariant, then derives the allocation counters from `max(id) + 1`. A lab
    /// that fails here is neither saved nor loaded.
    pub fn from_parts(
        mut title: String,
        nodes: Vec<Node>,
        links: Vec<Link>,
    ) -> Result<Self, &'static str> {
        if nodes.len() > MAX_NODES {
            return Err("too many devices");
        }
        if links.len() > MAX_LINKS {
            return Err("too many links");
        }
        if title.trim().is_empty() {
            title = "Untitled lab".to_string();
        }
        if title.chars().count() > MAX_STRING_CHARS {
            return Err("title is too long");
        }

        let mut node_ids: HashSet<u64> = HashSet::with_capacity(nodes.len());
        for node in &nodes {
            if node.id.0 > MAX_ID {
                return Err("device id is too large");
            }
            if !node_ids.insert(node.id.0) {
                return Err("duplicate device id");
            }
            if !valid_text(&node.name, true) {
                return Err("invalid device name");
            }
            if !valid_coord(node.x) || !valid_coord(node.y) {
                return Err("device position is out of range");
            }
            if let Some(address) = &node.ipv4
                && parse_ipv4(address).is_none()
            {
                return Err("invalid device IPv4 address");
            }
        }

        let mut link_ids: HashSet<u64> = HashSet::with_capacity(links.len());
        for link in &links {
            if link.id.0 > MAX_ID {
                return Err("link id is too large");
            }
            if !link_ids.insert(link.id.0) {
                return Err("duplicate link id");
            }
            if link.a == link.b {
                return Err("a link cannot connect a device to itself");
            }
            if !node_ids.contains(&link.a.0) || !node_ids.contains(&link.b.0) {
                return Err("link points at a missing device");
            }
        }

        let next_node = nodes
            .iter()
            .map(|node| node.id.0)
            .max()
            .map_or(0, |id| id.saturating_add(1));
        let next_link = links
            .iter()
            .map(|link| link.id.0)
            .max()
            .map_or(0, |id| id.saturating_add(1));

        Ok(Self {
            title,
            nodes,
            links,
            next_node,
            next_link,
        })
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn find_link(&self, a: NodeId, b: NodeId) -> Option<&Link> {
        self.links
            .iter()
            .find(|link| (link.a == a && link.b == b) || (link.a == b && link.b == a))
    }

    pub fn neighbours(&self, id: NodeId) -> Vec<&Node> {
        let mut out = Vec::new();
        for link in &self.links {
            let other = if link.a == id {
                Some(link.b)
            } else if link.b == id {
                Some(link.a)
            } else {
                None
            };
            if let Some(other_id) = other
                && let Some(node) = self.node(other_id)
            {
                out.push(node);
            }
        }
        out
    }

    pub fn ports(&self, id: NodeId) -> Vec<LinkId> {
        self.links
            .iter()
            .filter(|link| link.a == id || link.b == id)
            .map(|link| link.id)
            .collect()
    }

    pub fn add_node(
        &mut self,
        kind: DeviceKind,
        name: Option<&str>,
        x: f32,
        y: f32,
    ) -> Result<NodeId, &'static str> {
        if self.nodes.len() >= MAX_NODES {
            return Err("device limit reached");
        }
        if !valid_coord(x) || !valid_coord(y) {
            return Err("device position is out of range");
        }
        let id = self.next_node;
        if id > MAX_ID {
            return Err("device id limit reached");
        }
        let name = match name {
            Some(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => self.suggest_name(kind),
        };
        if !valid_text(&name, true) {
            return Err("device name is too long");
        }
        self.nodes.push(Node {
            id: NodeId(id),
            kind,
            name,
            x,
            y,
            ipv4: None,
        });
        self.next_node = id.saturating_add(1);
        Ok(NodeId(id))
    }

    pub fn add_link(&mut self, a: NodeId, b: NodeId) -> Result<LinkId, &'static str> {
        if self.links.len() >= MAX_LINKS {
            return Err("link limit reached");
        }
        if a == b {
            return Err("a device cannot be cabled to itself");
        }
        if self.node(a).is_none() || self.node(b).is_none() {
            return Err("both devices must exist before cabling");
        }
        if self.find_link(a, b).is_some() {
            return Err("these devices are already connected");
        }
        let id = self.next_link;
        if id > MAX_ID {
            return Err("link id limit reached");
        }
        self.links.push(Link {
            id: LinkId(id),
            a,
            b,
        });
        self.next_link = id.saturating_add(1);
        Ok(LinkId(id))
    }

    pub fn remove_node(&mut self, id: NodeId) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|node| node.id != id);
        let removed = self.nodes.len() != before;
        if removed {
            self.links.retain(|link| link.a != id && link.b != id);
        }
        removed
    }

    pub fn remove_link(&mut self, id: LinkId) -> bool {
        let before = self.links.len();
        self.links.retain(|link| link.id != id);
        self.links.len() != before
    }

    pub fn set_ipv4(&mut self, id: NodeId, address: &str) -> Result<(), &'static str> {
        let parsed = parse_ipv4(address).ok_or("not a valid IPv4 address")?;
        let node = self.node_mut(id).ok_or("device not found")?;
        node.ipv4 = Some(parsed.to_string());
        Ok(())
    }

    pub fn clear_ipv4(&mut self, id: NodeId) -> Result<(), &'static str> {
        let node = self.node_mut(id).ok_or("device not found")?;
        node.ipv4 = None;
        Ok(())
    }

    pub fn set_name(&mut self, id: NodeId, name: &str) -> Result<(), &'static str> {
        let name = name.trim();
        if !valid_text(name, true) {
            return Err("device name must be 1–256 characters");
        }
        let node = self.node_mut(id).ok_or("device not found")?;
        node.name = name.to_string();
        Ok(())
    }

    pub fn set_title(&mut self, title: &str) -> Result<(), &'static str> {
        let title = title.trim();
        if !valid_text(title, true) {
            return Err("title must be 1–256 characters");
        }
        self.title = title.to_string();
        Ok(())
    }

    pub fn move_node(&mut self, id: NodeId, x: f32, y: f32) -> Result<(), &'static str> {
        if !valid_coord(x) || !valid_coord(y) {
            return Err("device position is out of range");
        }
        let node = self.node_mut(id).ok_or("device not found")?;
        node.x = x;
        node.y = y;
        Ok(())
    }

    pub fn ipv4_of(&self, id: NodeId) -> Option<Ipv4Addr> {
        self.node(id)
            .and_then(|node| node.ipv4.as_deref())
            .and_then(parse_ipv4)
    }

    fn suggest_name(&self, kind: DeviceKind) -> String {
        let prefix = kind.name_prefix();
        let mut n = 1usize;
        loop {
            let candidate = format!("{prefix}{n}");
            if !self.nodes.iter().any(|node| node.name == candidate) {
                return candidate;
            }
            n = n.saturating_add(1);
            if n > 100_000 {
                return format!("{prefix}{}", self.nodes.len());
            }
        }
    }
}

/// Parse an IPv4 address. Brackets, ports, hostnames and whitespace are rejected.
pub fn parse_ipv4(value: &str) -> Option<Ipv4Addr> {
    let value = value.trim();
    if value.is_empty() || value.contains('/') {
        return None;
    }
    value.parse::<Ipv4Addr>().ok()
}

/// The first three octets of an address, used by the M1 `/24` classroom slice.
pub fn subnet24(address: Ipv4Addr) -> [u8; 3] {
    let octets = address.octets();
    [octets[0], octets[1], octets[2]]
}

/// Deterministic MAC address derived from a device id and port index. No randomness: the same lab
/// always produces the same frames and the same packet trace.
pub fn mac_for(node: NodeId, port: usize) -> String {
    let mixed = node
        .0
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add((port as u64).wrapping_mul(0x85eb_ca6b))
        .rotate_left(17);
    let b3 = (mixed >> 16) as u8;
    let b4 = (mixed >> 8) as u8;
    let b5 = mixed as u8;
    format!("02:4e:43:{b3:02x}:{b4:02x}:{b5:02x}")
}

fn valid_coord(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_COORD
}

fn valid_text(value: &str, non_empty: bool) -> bool {
    let count = value.chars().count();
    count <= MAX_STRING_CHARS && (!non_empty || count > 0)
}

/// The sample lab from the rebuild brief: R1 – SW1 – PC1 / PC2.
pub fn sample_lab() -> Topology {
    let mut topo = Topology::new("Sample lab");
    let router = topo.add_node(DeviceKind::Router, Some("R1"), 300.0, 300.0);
    let switch = topo.add_node(DeviceKind::Switch, Some("SW1"), 610.0, 300.0);
    let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 920.0, 190.0);
    let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 920.0, 410.0);

    if let (Ok(router), Ok(switch), Ok(pc1), Ok(pc2)) = (router, switch, pc1, pc2) {
        let _ = topo.add_link(router, switch);
        let _ = topo.add_link(switch, pc1);
        let _ = topo.add_link(switch, pc2);
        let _ = topo.set_ipv4(pc1, "10.0.0.1");
        let _ = topo.set_ipv4(pc2, "10.0.0.2");
    }
    topo
}

/// A slightly larger all-switched practice lab. Useful for the second lesson (two switches, four
/// hosts) without requiring routing.
pub fn campus_lab() -> Topology {
    let mut topo = Topology::new("Campus lab");
    let r1 = topo.add_node(DeviceKind::Router, Some("R1"), 250.0, 300.0);
    let sw1 = topo.add_node(DeviceKind::Switch, Some("SW1"), 520.0, 230.0);
    let sw2 = topo.add_node(DeviceKind::Switch, Some("SW2"), 520.0, 470.0);
    let pc1 = topo.add_node(DeviceKind::Host, Some("PC1"), 820.0, 130.0);
    let pc2 = topo.add_node(DeviceKind::Host, Some("PC2"), 820.0, 310.0);
    let pc3 = topo.add_node(DeviceKind::Host, Some("PC3"), 820.0, 430.0);
    let pc4 = topo.add_node(DeviceKind::Host, Some("PC4"), 820.0, 610.0);

    if let (Ok(r1), Ok(sw1), Ok(sw2), Ok(pc1), Ok(pc2), Ok(pc3), Ok(pc4)) =
        (r1, sw1, sw2, pc1, pc2, pc3, pc4)
    {
        let _ = topo.add_link(r1, sw1);
        let _ = topo.add_link(sw1, sw2);
        let _ = topo.add_link(sw1, pc1);
        let _ = topo.add_link(sw1, pc2);
        let _ = topo.add_link(sw2, pc3);
        let _ = topo.add_link(sw2, pc4);
        let _ = topo.set_ipv4(pc1, "10.0.0.11");
        let _ = topo.set_ipv4(pc2, "10.0.0.12");
        let _ = topo.set_ipv4(pc3, "10.0.0.13");
        let _ = topo.set_ipv4(pc4, "10.0.0.14");
    }
    topo
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u64, kind: DeviceKind, name: &str, ip: Option<&str>) -> Node {
        Node {
            id: NodeId(id),
            kind,
            name: name.to_string(),
            x: 10.0,
            y: 10.0,
            ipv4: ip.map(str::to_string),
        }
    }

    #[test]
    fn device_kind_parses_aliases() {
        assert_eq!(DeviceKind::parse("R"), Some(DeviceKind::Router));
        assert_eq!(DeviceKind::parse("switch"), Some(DeviceKind::Switch));
        assert_eq!(DeviceKind::parse("pc"), Some(DeviceKind::Host));
        assert_eq!(DeviceKind::parse("firewall"), None);
    }

    #[test]
    fn ipv4_is_strict() {
        assert_eq!(parse_ipv4("10.0.0.1"), Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(parse_ipv4(" 10.0.0.1 "), Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(parse_ipv4("10.0.0.1/24"), None);
        assert_eq!(parse_ipv4("10.0.0"), None);
        assert_eq!(parse_ipv4(""), None);
        assert_eq!(parse_ipv4("999.0.0.1"), None);
    }

    #[test]
    fn from_parts_rejects_duplicate_node_ids() {
        let nodes = vec![
            node(1, DeviceKind::Host, "A", None),
            node(1, DeviceKind::Host, "B", None),
        ];
        assert!(Topology::from_parts("Lab".into(), nodes, vec![]).is_err());
    }

    #[test]
    fn from_parts_rejects_duplicate_link_ids() {
        let nodes = vec![
            node(0, DeviceKind::Host, "A", None),
            node(1, DeviceKind::Host, "B", None),
        ];
        let links = vec![
            Link {
                id: LinkId(4),
                a: NodeId(0),
                b: NodeId(1),
            },
            Link {
                id: LinkId(4),
                a: NodeId(1),
                b: NodeId(0),
            },
        ];
        assert!(Topology::from_parts("Lab".into(), nodes, links).is_err());
    }

    #[test]
    fn from_parts_rejects_bad_links() {
        let nodes = vec![node(0, DeviceKind::Host, "A", None)];
        let self_link = vec![Link {
            id: LinkId(0),
            a: NodeId(0),
            b: NodeId(0),
        }];
        assert!(Topology::from_parts("Lab".into(), nodes.clone(), self_link).is_err());
        let dangling = vec![Link {
            id: LinkId(0),
            a: NodeId(0),
            b: NodeId(9),
        }];
        assert!(Topology::from_parts("Lab".into(), nodes, dangling).is_err());
    }

    #[test]
    fn from_parts_rejects_hostile_values() {
        let mut bad_name = node(0, DeviceKind::Host, "A", None);
        bad_name.name = "x".repeat(MAX_STRING_CHARS + 1);
        assert!(Topology::from_parts("Lab".into(), vec![bad_name], vec![]).is_err());

        let mut bad_coord = node(1, DeviceKind::Host, "A", None);
        bad_coord.x = f32::NAN;
        assert!(Topology::from_parts("Lab".into(), vec![bad_coord], vec![]).is_err());

        let mut huge_coord = node(2, DeviceKind::Host, "A", None);
        huge_coord.y = MAX_COORD + 1.0;
        assert!(Topology::from_parts("Lab".into(), vec![huge_coord], vec![]).is_err());

        let bad_ip = node(3, DeviceKind::Host, "A", Some("10.0.0.999"));
        assert!(Topology::from_parts("Lab".into(), vec![bad_ip], vec![]).is_err());

        let mut huge_id = node(4, DeviceKind::Host, "A", None);
        huge_id.id = NodeId(MAX_ID + 1);
        assert!(Topology::from_parts("Lab".into(), vec![huge_id], vec![]).is_err());
    }

    #[test]
    fn equality_ignores_allocation_counters() {
        let a = Topology::from_parts(
            "Lab".into(),
            vec![node(7, DeviceKind::Host, "A", None)],
            vec![],
        );
        let b = Topology::from_parts(
            "Lab".into(),
            vec![node(7, DeviceKind::Host, "A", None)],
            vec![],
        );
        assert_eq!(a.as_ref().ok(), b.as_ref().ok());

        let mut a = a.unwrap_or_default();
        let b = b.unwrap_or_default();
        let added = a.add_node(DeviceKind::Host, Some("B"), 1.0, 1.0);
        assert!(added.is_ok());
        a.nodes.pop();
        assert_eq!(a, b);
    }

    #[test]
    fn add_node_limits_and_validation() {
        let mut topo = Topology::new("Lab");
        let id = topo.add_node(DeviceKind::Host, None, 4.0, 5.0);
        assert_eq!(id, Ok(NodeId(0)));
        assert_eq!(topo.nodes[0].name, "PC1");
        assert!(
            topo.add_node(DeviceKind::Host, None, f32::INFINITY, 0.0)
                .is_err()
        );
        assert!(
            topo.add_node(
                DeviceKind::Host,
                Some(&"x".repeat(MAX_STRING_CHARS + 1)),
                0.0,
                0.0
            )
            .is_err()
        );
    }

    #[test]
    fn add_link_rejects_self_missing_and_duplicate() {
        let mut topo = Topology::new("Lab");
        let a = topo.add_node(DeviceKind::Host, None, 0.0, 0.0);
        let b = topo.add_node(DeviceKind::Host, None, 1.0, 1.0);
        let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
        assert!(topo.add_link(a, a).is_err());
        assert!(topo.add_link(a, NodeId(99)).is_err());
        assert!(topo.add_link(a, b).is_ok());
        assert!(topo.add_link(b, a).is_err());
    }

    #[test]
    fn neighbours_and_ports_are_deterministic() {
        let topo = sample_lab();
        let sw = topo
            .nodes
            .iter()
            .find(|node| node.name == "SW1")
            .map(|node| node.id);
        let sw = match sw {
            Some(id) => id,
            None => return,
        };
        assert_eq!(topo.neighbours(sw).len(), 3);
        assert_eq!(topo.ports(sw).len(), 3);
    }

    #[test]
    fn mac_is_deterministic_and_well_formed() {
        let one = mac_for(NodeId(42), 0);
        let two = mac_for(NodeId(42), 0);
        assert_eq!(one, two);
        assert_eq!(one.len(), 17);
        assert!(one.starts_with("02:4e:43:"));
        assert_ne!(one, mac_for(NodeId(42), 1));
    }

    #[test]
    fn set_ipv4_validates_and_canonicalises() {
        let mut topo = sample_lab();
        let pc1 = NodeId(2);
        assert!(topo.set_ipv4(pc1, "10.0.0.9").is_ok());
        assert_eq!(
            topo.node(pc1).and_then(|n| n.ipv4.clone()),
            Some("10.0.0.9".to_string())
        );
        assert!(topo.set_ipv4(pc1, "not-an-ip").is_err());
        assert_eq!(
            topo.node(pc1).and_then(|n| n.ipv4.clone()),
            Some("10.0.0.9".to_string())
        );
    }

    #[test]
    fn sample_lab_is_valid_and_pingable_shape() {
        let topo = sample_lab();
        assert_eq!(topo.nodes.len(), 4);
        assert_eq!(topo.links.len(), 3);
        assert_eq!(
            topo.node(NodeId(2)).and_then(|n| n.ipv4.as_deref()),
            Some("10.0.0.1")
        );
        assert!(
            Topology::from_parts(topo.title.clone(), topo.nodes.clone(), topo.links.clone())
                .is_ok()
        );
    }
}
