# NetworksCraft work list

Status: **M1 alpha, playable**. The sample lab, same-L2 ping, packet log, save/open, command line and
headless CLI work. The entries below are the honest gaps against the rebuild brief, in priority
order.

## Done in this build

- [x] Desktop app with a devices palette, topology canvas, terminal, packet log, inspector and status
      bar; draggable split panels.
- [x] Topology model with ids, IPv4 validation, caps, one validator and content-only equality.
- [x] Deterministic L2 simulation: switch MAC learning, flood, known-unicast forward, ARP/ICMP trace,
      loopback, unknown target, no path, router termination, event/graph caps.
- [x] Public `.nclab` v1 writer/reader with deterministic bytes, bounded reads, atomic saves,
      duplicate/dangling/self-link rejection, hostile-input tests.
- [x] Session command registry and command line; panic guard restores the lab.
- [x] A small device-dialect subset: `show ip interface brief`, `show ip route`, `show version`.
- [x] `networkscraft-cli run/info`, script and script-file, JSON output, usable exit codes.
- [x] `--screenshot` and `--self-test`; self-test covers move/act/hud.
- [x] Code-drawn icons/theme, attribution, format documentation, mechanics sheet.

## Next (M1 polish)

1. **Native file dialogs.** Replace the in-app path picker with `rfd` on native builds, keeping the
   in-app picker for WASM. The brief lists `rfd` for the UI layer.
2. **Undo/redo and delete confirmation** for topology edits. Dragging currently dirties on every
   frame; coalesce a drag into one undo entry.
3. **Drag-and-drop from the palette** (click-to-place works today) and snapping.
4. **Persisted workspace layout** (`networkscraft.prefs`).
5. **Split the modules into the six crates** from the brief and add the `xtask`/layering gate. The
   current app keeps the boundaries as modules (`model`, `sim`, `io`, `engine`, `ui`).

## Next (M1 routing, then M2)

1. **Interfaces + routing:** per-link interface addresses, connected routes, static routes,
   longest-prefix match, TTL, ICMP unreachable, cross-subnet ping.
2. **Extend the device CLI** (the `show ip interface brief`, `show ip route` and `show version`
   subset is in): `ip address …`, a small config mode, interface up/down.
3. **VLANs:** 802.1Q trunk tags, access ports, SVI and router-on-a-stick.
4. **OSPF single area:** neighbours, simplified LSAs, SPF, deterministic convergence.
5. **ACLs:** numbered extended, inbound interface, first-match.
6. **DHCP/DNS stub server nodes.**
7. **STP minimal, traceroute, ICMP errors.**

## Automation and distribution

1. **Control channel** on `127.0.0.1:PORT` (`cmdline.input`, `engine.execute`, `ui.screenshot`).
2. **MCP server** (`execute`, `cmdline.input`, `inspect_topology`, `capture_packets`, `render`,
   `run_lab`, `check_lab`).
3. **WASM browser demo** for the L0–L4 core. The pure modules are free of egui; the UI and file
   dialogs need a web adapter.
4. **Curriculum corpus** in a separate repository (3–5 CCNA-intro labs) and a grading pack.
5. **Packaging** for Linux/macOS/Windows/FreeBSD, accessibility audit, fuzzing, i18n hooks.

## Known deliberate simplifications

- `/24` is assumed; there is no prefix field yet.
- One IPv4 address per device; no interface list.
- Routers do not route in this slice.
- The packet animation is a UI visualisation of the deterministic trace, not a live event queue.
- File dialogs are in-app rather than `rfd`.
- The app is one crate with layered modules rather than six workspace crates.
