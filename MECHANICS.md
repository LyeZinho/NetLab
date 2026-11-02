# NetworksCraft — how it plays

This is the play-by-play for the M1 alpha build. Every number below is a real value in the
code; the source column says whether it came from the rebuild brief or is an original design choice
(marked **estimated**). The same values are mirrored in `mechanics.json` for the build checker.

## What this is

NetworksCraft is an education-first network **lab simulator**, not a twitch game. You place routers,
switches and hosts on a canvas, cable them, give hosts IPv4 addresses, and run a deterministic ping.
The packet log explains the whole trip: ARP, switch MAC learning, flooding, the ARP reply, the ICMP
request and the reply. The first launch opens a working sample lab and has already run one ping so
the screen is self-explanatory.

The M1 slice deliberately stops at one broadcast domain. Routers are placeholders that terminate L2;
routing, VLANs, OSPF, ACLs, DHCP/DNS and the MCP/control surfaces are the next milestones.

## The 30-second loop

1. Pick **PC1** on the canvas.
2. Press **Ping** (or type `ping 10.0.0.2` in the terminal).
3. Watch the packet log fill with 8 deterministic events.
4. Drag a device, add another host, cable it, set its address, and ping again.
5. Save the lab as a `.nclab` file, restart, open it, and ping again.

## Controls

| Action | Binding | Source |
|---|---|---|
| Select tool | `V` or the Select chip | estimated / designed |
| Cable tool | `C` or the Cable chip | estimated / designed |
| Delete tool | `X` or the Delete chip | estimated / designed |
| Select a device | click it | designed |
| Move a device | left-drag it | designed |
| Pan the canvas | middle-drag | estimated |
| Zoom | mouse wheel (0.45×–1.8×) | estimated |
| Fit the topology | toolbar **Fit** / View ▸ Fit | designed |
| Ping selected host | `F5` / **Ping** | designed |
| Run animation | `F6` | designed |
| Pause animation | `F7` | designed |
| Reset packet log | `F8` | designed |
| Save / Save as | `Ctrl+S` / `Ctrl+Shift+S` | designed |
| Open | `Ctrl+O` | designed |
| New lab | `Ctrl+N` | designed |
| Command reference | `F1` | designed |
| Delete selected | `Delete` | designed |

The terminal accepts the same command ids as the menus, toolbar and CLI. Type `help` for the list.

## Model numbers

| Number | Value | Source |
|---|---|---|
| Device kinds | 3 (router, switch, host) | REBUILD.md §3 |
| Maximum devices | 4096 | REBUILD.md §3 (fixed) |
| Maximum cables | 8192 | REBUILD.md §3 (fixed) |
| Maximum device/cable id | 1,000,000 | REBUILD.md §3 (fixed) |
| Maximum name or title | 256 characters | REBUILD.md §3 (fixed) |
| Maximum coordinate magnitude | ±1,000,000 | REBUILD.md §3 (fixed) |
| IPv4 prefix in the first slice | `/24` | **estimated**; the brief's sample uses `10.0.0.x` |
| Deterministic MAC | `02:4e:43:xx:xx:xx` from id + port | designed |

## Simulation numbers

| Number | Value | Source |
|---|---|---|
| Graph visits per traversal | 4096 | REBUILD.md §4 (fixed) |
| Events per operation | 256 | REBUILD.md §4 (fixed) |
| MAC entries per switch | 512 | REBUILD.md §4 (fixed) |
| L2 steps per frame | 2048 | REBUILD.md §4 (fixed) |
| Loopback RTT | 0 ms | designed |
| RTT for a normal ping | 1 ms + 2 ms per switch | **estimated**; fixed so runs reproduce |
| Sample lab | 4 devices, 3 cables, 8 events | REBUILD.md sample + measured |
| Router behaviour | terminates the broadcast domain | REBUILD.md §4 |
| Switch behaviour | learn source MAC, flood unknown, forward known | REBUILD.md §4 |

## Screen numbers

| Number | Value | Source |
|---|---|---|
| Window | 1440 × 900 (min 1080 × 680) | **estimated** |
| Canvas grid | 32 units | **estimated** |
| Node icon radius | 30 units at 100% zoom | **estimated** |
| Zoom range | 0.45×–1.8× | **estimated** |
| Fit margins | 260 px horizontal, 220 px vertical | **estimated** |
| Default console height | 310 px | **estimated** |
| Console history | 200 lines | **estimated** |
| Trace links kept for the pulse | 64 | **estimated** |
| Packet pulse cycle | 1.8 s | **estimated** |
| Packet log rows | newest last, auto-scroll to newest | REBUILD.md §7 / designed |
| Goal | one successful ping, tracked as COMPLETE | designed |

## Colour and asset rules

Every colour comes from `src/theme.rs`; every icon is drawn with egui shapes in `src/ui.rs`. There is
no vendor artwork, icon, font, documentation text or saved lab in the tree. The only third-party
assets are the fonts that ship with egui's `epaint_default_fonts` crate; they are attributed in
`ATTRIBUTION.md`.

## What is not in this slice

- L3 routing (static routes, connected routes, TTL, ICMP unreachable)
- VLANs, 802.1Q, ROAS/SVI
- OSPF, ACLs, DHCP/DNS server roles, STP
- Drag-and-drop from the palette (click-to-place is in), undo/redo
- Native `rfd` file dialogs (an in-app path picker is used instead; still UI-only)
- Control channel, MCP server, WASM build, course corpus
- Six separate crates (the same layers live as modules in one app crate for this build)

These are recorded in `docs/gaps.md`.
