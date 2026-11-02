# NetworksCraft

A free, open, education-first network lab simulator in Rust. Place routers, switches and hosts,
cable them, set IPv4 addresses, run a deterministic ping, and read the whole packet story in the
console. Labs are plain JSON `.nclab` files; no account, no cloud.

This build is the **M1 alpha**: the first playable slice from the rebuild brief. It is native
(Linux/macOS/Windows/FreeBSD with egui + glow) and has a headless CLI. Routing, VLANs, OSPF, ACLs,
DHCP/DNS, the MCP server and the browser build are the next milestones; see `docs/gaps.md`.

## Run it

```sh
cargo run --release
```

The first launch opens the sample lab with PC1 selected and one ping already in the packet log.

### Checks the build tool uses

```sh
cargo run --release -- --screenshot shots/now.png
cargo run --release -- --self-test shots/selftest.json
```

`--screenshot` draws 120 frames, saves the window picture as a PNG and exits. `--self-test` starts
the sample lab, moves a device through the same command the canvas uses, selects PC1, pings PC2,
draws a real UI frame offscreen, writes a JSON report and exits.

### Headless CLI

```sh
cargo run --release --bin networkscraft-cli -- info lab.nclab
cargo run --release --bin networkscraft-cli -- run lab.nclab --script 'select 2; ping 10.0.0.2'
cargo run --release --bin networkscraft-cli -- run lab.nclab --script-file checks.txt --json
```

Exit 0 only when every scripted command succeeds and every ping succeeds. A ping with outcome
`unknown target`, `unreachable`, `different subnet`, `no route` or `not a host` is a failure.

## The first lesson

1. Click **PC1**.
2. Click **Ping** (or type `ping 10.0.0.2` in the terminal).
3. The packet log shows ARP who-has, SW1 learning the source MAC, the flooded ARP, the reply, SW1
   learning the destination MAC, the ICMP request, the reply and the completion line.
4. Drag R1 or SW1 to rearrange the lab; the canvas auto-fits on launch and whenever you press **Fit**.
5. Type `adddevice host PC3`, then cable it with the **Cable** tool and set an address in the
   inspector.

The sample is deliberately a single L2 domain. Pinging across R1 returns a clear “different subnet”
or “router terminates the L2 domain” trace rather than pretending to route.

## Commands

Every action is a command. The terminal, menus, toolbar and CLI resolve the same id and JSON params.

| id | what it does |
|---|---|
| `newlab` | empty lab |
| `sample`, `campus` | load a built-in practice lab |
| `adddevice` | add router/switch/host at canvas coordinates |
| `link` | cable two devices |
| `select` | select a device (or clear) |
| `setipv4` | validate and set a host address |
| `rename`, `movedevice`, `settitle` | edit a device or lab |
| `deletedevice`, `deletelink` | remove topology |
| `ping` | deterministic ping by IPv4, source = selected host |
| `openlab`, `savelab` | `.nclab` read/write |
| `sim.run`, `sim.pause`, `sim.reset` | animation state and packet log |
| `show` | selected-device view: `show ip interface brief`, `show ip route`, `show version` |
| `info`, `help`, `clearlog` | inspect the lab / list commands / clear console |

`help` in the terminal prints the full reference with aliases and params.

## Layout

- **Library** (left): one tree folder for everything — Routers, Switches, End Devices, Wireless, WAN
  Cloud and Connections. Pick any model (2911/4331 router, 2960/3560 switch, hub, PC, laptop,
  server, printer, IP phone, access point, cloud, modem) or cable type (straight-through,
  crossover, console, serial, fiber); select/cable/delete tools; built-in labs.
- **Topology** (centre): code-drawn devices, links with ids, MAC/ARP/ICMP pulse animation, zoom/pan.
- **Terminal** (bottom left): command log and the same command line as the CLI.
- **Packet log** (bottom right): the last operation's events, newest last, with link ids.
- **Inspector** (right): title, file state, name/IPv4 fields, neighbours, lab goal and sim controls.
- **Status bar**: message, file name and dirty marker, node/link/event counts.

## Architecture

The single app crate deliberately keeps the layer boundaries as modules so they can be split into
the six crates from the brief without changing behaviour:

- `src/model.rs` — topology, ids, IPv4, limits, the one validator (`Topology::from_parts`).
- `src/sim.rs` — bounded deterministic L2/ARP/ICMP trace.
- `src/io.rs` — the public `.nclab` v1 wire format, atomic save, bounded load.
- `src/engine.rs` — `Session`, command registry, command line, panic guard.
- `src/theme.rs`, `src/ui.rs` — code-drawn panels, icons and colours.
- `src/selftest.rs` — headless player checks.
- `src/main.rs` and `src/bin/networkscraft-cli.rs` — desktop and headless entry points.

Nothing below the UI depends on egui. The public format is owned by `io`, not by the model.

## Quality gates

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The production crate roots deny `unwrap`, `expect`, `panic`, `todo`, `unimplemented`, `unreachable`
and forbid `unsafe`. Tests cover the model invariants, the simulation (including caps), the file
format (hostile input, oversized and truncated files, atomic save failure), the engine commands and
panic guard, and the pure UI helpers. Regression tests named in the brief
(`hostile_lab_never_panic`, `hostile_params_never_panic`, etc.) are in the test modules.

## Clean room and assets

This is original work. Every icon is drawn in code; every colour is a theme token. There is no Cisco,
GNS3, Packet Tracer or other vendor artwork, font, help text or saved lab in the tree. The only
third-party visual assets are egui's bundled default fonts, which are attributed in `ATTRIBUTION.md`.
See `docs/file-format-lab.md` for the public lab format.
