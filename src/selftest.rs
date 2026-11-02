//! Headless self-test used by the build check and by `--self-test`.
//!
//! It starts a normal sample session, drives the same commands a player's mouse/keyboard would
//! reach, measures the game's own state, draws one real UI frame offscreen, and writes a small JSON
//! report. No window is opened, so it is safe in CI.

use crate::engine::Session;
use crate::model::NodeId;
use crate::theme;
use crate::ui::App;
use eframe::egui;
use serde_json::json;
use std::path::Path;

pub fn run(path: &str) -> Result<(), String> {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let mut app = App::headless(Session::sample());

    // 1. MOVEMENT: move a device the way a canvas drag does.
    let start = app
        .session
        .topology
        .node(NodeId(3))
        .map(|node| (node.x, node.y));
    let mut moved = 0.0f32;
    let mut move_detail = "device PC2 was not found".to_string();
    if let Some((x, y)) = start {
        let result = app.run(
            "movedevice",
            json!({ "nodeId": 3, "x": x + 40.0, "y": y + 12.0 }),
        );
        match result {
            Ok(_) => {
                if let Some(node) = app.session.topology.node(NodeId(3)) {
                    let dx = node.x - x;
                    let dy = node.y - y;
                    moved = (dx * dx + dy * dy).sqrt();
                    move_detail = format!("PC2 moved {moved:.1} canvas units");
                }
            }
            Err(error) => move_detail = format!("move failed: {error}"),
        }
    }
    let move_ok = moved > 30.0;

    // 2. ACTION: select PC1 and ping PC2 through the same command line the player uses.
    let select = app.cmdline("select 2");
    let ping = app.cmdline("ping 10.0.0.2");
    let mut act_ok = false;
    let mut act_detail = "ping did not run".to_string();
    if let (Ok(_), Ok(value)) = (select, ping) {
        let outcome = value
            .get("outcome")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let events = value
            .get("events")
            .and_then(serde_json::Value::as_array)
            .map(|events| {
                events
                    .iter()
                    .filter_map(|event| event.get("summary").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let has_arp = events.iter().any(|line| line.starts_with("ARP who-has"));
        let has_icmp = events.iter().any(|line| line.contains("ICMP echo request"));
        let has_reply = events.iter().any(|line| line.contains("ICMP echo reply"));
        act_ok = outcome == "success" && has_arp && has_icmp && has_reply;
        let rtt = value
            .get("rttMs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        act_detail = format!(
            "ping {outcome} in {rtt} ms with {} events (ARP {}, MAC learning {}, ICMP {})",
            events.len(),
            if has_arp { "yes" } else { "no" },
            if app
                .session
                .events
                .iter()
                .any(|event| event.summary.contains("learned"))
            {
                "yes"
            } else {
                "no"
            },
            if has_icmp && has_reply { "yes" } else { "no" }
        );
    }

    // 3. HUD: draw one real frame offscreen and verify the visible information is populated.
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| app.draw(ui));
    let shapes = output.shapes.len();
    // We are not a renderer: acknowledge the font atlas delta so it can be dropped safely.
    output.textures_delta.clear();
    let selected_host = app
        .session
        .selected
        .and_then(|id| app.session.topology.node(id))
        .is_some_and(|node| node.kind == crate::model::DeviceKind::Host);
    let hud_ok = shapes > 50
        && app.session.topology.nodes.len() == 4
        && !app.session.events.is_empty()
        && app.session.goal_done
        && selected_host;
    let hud_detail = format!(
        "drew {shapes} UI shapes; inspector shows {}, packet log shows {} events, goal {}",
        app.session
            .selected
            .and_then(|id| app.session.topology.node(id))
            .map_or("no selection", |node| node.name.as_str()),
        app.session.events.len(),
        if app.session.goal_done {
            "complete"
        } else {
            "waiting"
        }
    );

    let report = json!({
        "version": 1,
        "steps": [
            { "step": "move", "passed": move_ok, "detail": move_detail },
            { "step": "act", "passed": act_ok, "detail": act_detail },
            { "step": "hud", "passed": hud_ok, "detail": hud_detail }
        ]
    });
    let text = serde_json::to_string_pretty(&report)
        .map_err(|error| format!("could not serialise self-test: {error}"))?;
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create self-test folder: {error}"))?;
    }
    std::fs::write(path, format!("{text}\n"))
        .map_err(|error| format!("could not write self-test report: {error}"))?;

    if move_ok && act_ok && hud_ok {
        Ok(())
    } else {
        Err("one or more self-test steps failed".to_string())
    }
}
