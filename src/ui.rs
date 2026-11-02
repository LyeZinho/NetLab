//! L5 — the desktop UI.
//!
//! The UI is a thin shell: panels read [`Session`] state and act through the command registry. All
//! colours come from [`theme`]; all icons are drawn in code.

use crate::engine::{Error, Session};
use crate::model::{DeviceKind, LinkId, NodeId, Topology, mac_for};
use crate::theme::{self, RADIUS, T};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Layout, Margin,
    PointerButton, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui, Vec2, pos2,
    vec2,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = no screenshot, 1 = saved, 2 = failed.
pub static SCREENSHOT_STATE: AtomicU8 = AtomicU8::new(0);
const SCREENSHOT_FRAME: u32 = 120;
const SCREENSHOT_TIMEOUT: u32 = 320;
const PULSE_SECONDS: f32 = 1.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Cable,
    Delete,
}

impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Cable => "Cable",
            Self::Delete => "Delete",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Logo,
    New,
    Sample,
    Campus,
    Open,
    Save,
    SaveAs,
    Run,
    Pause,
    Reset,
    Ping,
    Select,
    Cable,
    Trash,
    Router,
    Switch,
    Host,
    Help,
    Fit,
    Plus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileMode {
    Open,
    SaveAs,
}

struct FileDialog {
    mode: FileMode,
    path: String,
    error: Option<String>,
}

struct Toast {
    text: String,
    ttl: f32,
    kind: crate::engine::MessageKind,
}

#[derive(Clone, Copy, Debug)]
struct Camera {
    pan: Vec2,
    zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            pan: vec2(0.0, 0.0),
            zoom: 0.9,
        }
    }
}

/// The whole application state. `Session` is the model; everything else here is view state.
pub struct App {
    pub session: Session,
    tool: Tool,
    link_start: Option<NodeId>,
    camera: Camera,
    drag_node: Option<(NodeId, (f32, f32), Pos2)>,
    canvas_rect: Rect,
    fit_requested: bool,
    fit_settle: u8,
    show_grid: bool,
    name_buf: String,
    ip_buf: String,
    edit_node: Option<NodeId>,
    title_buf: String,
    term_input: String,
    focus_terminal: bool,
    dialog: Option<FileDialog>,
    help_open: bool,
    about_open: bool,
    toast: Option<Toast>,
    pulse_t: f32,
    last_pulse_generation: u64,
    screenshot_path: Option<String>,
    frames: u32,
    shot_requested: bool,
}

impl App {
    pub fn new(session: Session, screenshot_path: Option<String>) -> Self {
        let selected = session.selected;
        let name_buf = selected
            .and_then(|id| session.topology.node(id))
            .map(|node| node.name.clone())
            .unwrap_or_default();
        let ip_buf = selected
            .and_then(|id| session.topology.node(id))
            .and_then(|node| node.ipv4.clone())
            .unwrap_or_default();
        let title_buf = session.topology.title.clone();
        Self {
            session,
            tool: Tool::Select,
            link_start: None,
            camera: Camera::default(),
            drag_node: None,
            canvas_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0)),
            fit_requested: true,
            fit_settle: 0,
            show_grid: true,
            name_buf,
            ip_buf,
            edit_node: selected,
            title_buf,
            term_input: String::new(),
            focus_terminal: false,
            dialog: None,
            help_open: false,
            about_open: false,
            toast: None,
            pulse_t: 0.0,
            last_pulse_generation: 0,
            screenshot_path,
            frames: 0,
            shot_requested: false,
        }
    }

    /// Used by the headless self-test: draw the same UI into an offscreen egui pass.
    pub fn headless(session: Session) -> Self {
        Self::new(session, None)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        theme::install(&ctx);
        self.animate(&ctx);
        self.handle_shortcuts(&ctx);
        self.sync_edit_buffers();

        ui.painter()
            .rect_filled(ui.max_rect(), CornerRadius::ZERO, T.bg);
        self.top_bar(ui);
        self.status_bar(ui);
        self.left_panel(ui);
        self.right_panel(ui);
        self.bottom_panel(ui);
        self.canvas_panel(ui);
        self.toast(&ctx);
        self.file_dialog(&ctx);
        self.help_window(&ctx);
        self.about_window(&ctx);
    }

    // ── public actions, shared by menus, toolbar, palette and the terminal ──────────────────────

    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, Error> {
        if self.screenshot_path.is_some() {
            return Ok(Value::Null);
        }
        let generation = self.session.trace_generation;
        let result = self.session.execute(id, &params);
        if self.session.trace_generation != generation {
            self.pulse_t = 0.0;
            self.last_pulse_generation = self.session.trace_generation;
        }
        match &result {
            Ok(_) => {
                self.sync_from_session();
                if matches!(
                    id,
                    "adddevice" | "link" | "savelab" | "openlab" | "sample" | "campus"
                ) {
                    self.show_toast(self.session.message.clone(), self.session.message_kind);
                }
            }
            Err(error) => self.show_toast(error.to_string(), crate::engine::MessageKind::Error),
        }
        self.sync_edit_buffers();
        result
    }

    pub fn cmdline(&mut self, text: &str) -> Result<Value, Error> {
        if self.screenshot_path.is_some() {
            return Ok(Value::Null);
        }
        let generation = self.session.trace_generation;
        let result = self.session.cmdline(text);
        if self.session.trace_generation != generation {
            self.pulse_t = 0.0;
            self.last_pulse_generation = self.session.trace_generation;
        }
        if let Err(error) = &result {
            self.show_toast(error.to_string(), crate::engine::MessageKind::Error);
        } else {
            self.sync_from_session();
        }
        self.sync_edit_buffers();
        result
    }

    fn show_toast(&mut self, text: impl Into<String>, kind: crate::engine::MessageKind) {
        self.toast = Some(Toast {
            text: text.into(),
            ttl: 3.2,
            kind,
        });
    }

    fn animate(&mut self, ctx: &egui::Context) {
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.05);
        if let Some(toast) = &mut self.toast {
            toast.ttl -= dt;
            if toast.ttl <= 0.0 {
                self.toast = None;
            }
        }
        if self.session.trace_generation != self.last_pulse_generation {
            self.last_pulse_generation = self.session.trace_generation;
            self.pulse_t = 0.0;
        }
        if self.session.running && !self.session.trace_links.is_empty() {
            self.pulse_t += dt / PULSE_SECONDS;
            if self.pulse_t >= 1.0 {
                self.pulse_t = 0.0;
            }
        }
        if self.session.running || self.toast.is_some() || self.screenshot_path.is_some() {
            ctx.request_repaint();
        }
    }

    fn sync_edit_buffers(&mut self) {
        if self.edit_node != self.session.selected {
            self.edit_node = self.session.selected;
            self.sync_from_session();
        }
    }

    fn sync_from_session(&mut self) {
        self.title_buf = self.session.topology.title.clone();
        if let Some(node) = self
            .session
            .selected
            .and_then(|id| self.session.topology.node(id))
        {
            self.name_buf = node.name.clone();
            self.ip_buf = node.ipv4.clone().unwrap_or_default();
        } else {
            self.name_buf.clear();
            self.ip_buf.clear();
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.screenshot_path.is_some() {
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            if ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S)
            }) {
                self.begin_dialog(FileMode::SaveAs);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
                self.save_current();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O)) {
                self.begin_dialog(FileMode::Open);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
                let _ = self.run("newlab", json!({}));
            }
            return;
        }

        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S)
        }) {
            self.begin_dialog(FileMode::SaveAs);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_current();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O)) {
            self.begin_dialog(FileMode::Open);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
            let _ = self.run("newlab", json!({}));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.dialog.is_some() {
                self.dialog = None;
            } else {
                self.link_start = None;
                self.tool = Tool::Select;
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::V)) {
            self.tool = Tool::Select;
            self.link_start = None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::C)) {
            self.tool = Tool::Cable;
            self.link_start = None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::X)) {
            self.tool = Tool::Delete;
            self.link_start = None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
            self.delete_selected();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.demo_ping();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F6)) {
            let _ = self.run("sim.run", json!({}));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F7)) {
            let _ = self.run("sim.pause", json!({}));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F8)) {
            let _ = self.run("sim.reset", json!({}));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.help_open = true;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.session.selected {
            let _ = self.run("deletedevice", json!({ "nodeId": id.0 }));
        } else if let Some(link) = self.session.selected_link {
            let _ = self.run("deletelink", json!({ "linkId": link.0 }));
        } else {
            self.show_toast(
                "Select a device or cable first",
                crate::engine::MessageKind::Info,
            );
        }
    }

    fn demo_ping(&mut self) {
        let source = match self.session.selected {
            Some(id)
                if self
                    .session
                    .topology
                    .node(id)
                    .is_some_and(|n| n.kind == DeviceKind::Host) =>
            {
                id
            }
            _ => {
                let Some(first) = self
                    .session
                    .topology
                    .nodes
                    .iter()
                    .find(|node| node.kind == DeviceKind::Host)
                    .map(|node| node.id)
                else {
                    self.show_toast(
                        "Add two hosts before pinging",
                        crate::engine::MessageKind::Info,
                    );
                    return;
                };
                let _ = self.run("select", json!({ "nodeId": first.0 }));
                first
            }
        };
        let target = self
            .session
            .topology
            .nodes
            .iter()
            .find(|node| node.kind == DeviceKind::Host && node.id != source && node.ipv4.is_some())
            .and_then(|node| node.ipv4.clone());
        let Some(target) = target else {
            self.show_toast(
                "Add a second host with an IPv4 address",
                crate::engine::MessageKind::Info,
            );
            return;
        };
        let _ = self.run("ping", json!({ "source": source.0, "target": target }));
    }

    fn save_current(&mut self) {
        if self.session.path.is_some() {
            let _ = self.run("savelab", json!({}));
        } else {
            self.begin_dialog(FileMode::SaveAs);
        }
    }

    fn begin_dialog(&mut self, mode: FileMode) {
        let path = self.session.path.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join("lab.nclab")
        });
        self.dialog = Some(FileDialog {
            mode,
            path: path.display().to_string(),
            error: None,
        });
    }

    // ── top bar ─────────────────────────────────────────────────────────────────────────────────

    fn top_bar(&mut self, ui: &mut Ui) {
        egui::Panel::top("nc_top")
            .frame(
                Frame::new()
                    .fill(T.chrome)
                    .inner_margin(Margin::symmetric(10, 5))
                    .stroke(Stroke::new(1.0, T.stroke)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    self.brand(ui);
                    ui.add_space(12.0);
                    egui::MenuBar::new().ui(ui, |ui| {
                        self.menus(ui);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        self.live_pill(ui);
                    });
                });
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    if toolbar_button(ui, Icon::New, "New", false).clicked() {
                        let _ = self.run("newlab", json!({}));
                    }
                    if toolbar_button(ui, Icon::Sample, "Sample", false).clicked() {
                        let _ = self.run("sample", json!({}));
                    }
                    if toolbar_button(ui, Icon::Campus, "Campus", false).clicked() {
                        let _ = self.run("campus", json!({}));
                    }
                    ui.add_space(4.0);
                    if toolbar_button(ui, Icon::Open, "Open", false).clicked() {
                        self.begin_dialog(FileMode::Open);
                    }
                    if toolbar_button(ui, Icon::Save, "Save", false).clicked() {
                        self.save_current();
                    }
                    if toolbar_button(ui, Icon::SaveAs, "Save as", false).clicked() {
                        self.begin_dialog(FileMode::SaveAs);
                    }
                    ui.separator();
                    if toolbar_button(ui, Icon::Ping, "Ping", true).clicked() {
                        self.demo_ping();
                    }
                    if toolbar_button(ui, Icon::Run, "Run", false).clicked() {
                        let _ = self.run("sim.run", json!({}));
                    }
                    if toolbar_button(ui, Icon::Pause, "Pause", false).clicked() {
                        let _ = self.run("sim.pause", json!({}));
                    }
                    if toolbar_button(ui, Icon::Reset, "Reset", false).clicked() {
                        let _ = self.run("sim.reset", json!({}));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if toolbar_button(ui, Icon::Fit, "Fit", false).clicked() {
                            self.fit_requested = true;
                        }
                        ui.label(
                            RichText::new(format!("{:.0}%", self.camera.zoom * 100.0))
                                .monospace()
                                .size(11.0)
                                .color(T.faint),
                        );
                    });
                });
            });
    }

    fn brand(&mut self, ui: &mut Ui) {
        let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        draw_logo(ui.painter(), rect.center(), 19.0);
        ui.add_space(1.0);
        ui.label(
            RichText::new("NetworksCraft")
                .size(14.0)
                .strong()
                .color(T.text),
        );
    }

    fn menus(&mut self, ui: &mut Ui) {
        ui.menu_button("File", |ui| {
            if ui.button("New lab          Ctrl+N").clicked() {
                let _ = self.run("newlab", json!({}));
                ui.close();
            }
            ui.separator();
            if ui.button("Sample lab").clicked() {
                let _ = self.run("sample", json!({}));
                ui.close();
            }
            if ui.button("Campus lab").clicked() {
                let _ = self.run("campus", json!({}));
                ui.close();
            }
            ui.separator();
            if ui.button("Open…            Ctrl+O").clicked() {
                self.begin_dialog(FileMode::Open);
                ui.close();
            }
            if ui.button("Save             Ctrl+S").clicked() {
                self.save_current();
                ui.close();
            }
            if ui.button("Save as…   Ctrl+Shift+S").clicked() {
                self.begin_dialog(FileMode::SaveAs);
                ui.close();
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui.button("Select tool   V").clicked() {
                self.tool = Tool::Select;
                self.link_start = None;
                ui.close();
            }
            if ui.button("Cable tool    C").clicked() {
                self.tool = Tool::Cable;
                self.link_start = None;
                ui.close();
            }
            if ui.button("Delete tool   X").clicked() {
                self.tool = Tool::Delete;
                self.link_start = None;
                ui.close();
            }
            ui.separator();
            if ui.button("Delete selected   Del").clicked() {
                self.delete_selected();
                ui.close();
            }
            if ui.button("Clear packet log").clicked() {
                let _ = self.run("sim.reset", json!({}));
                ui.close();
            }
        });
        ui.menu_button("Draw", |ui| {
            if ui.button("Router").clicked() {
                let _ = self.run("adddevice", json!({ "kind": "router" }));
                ui.close();
            }
            if ui.button("Switch").clicked() {
                let _ = self.run("adddevice", json!({ "kind": "switch" }));
                ui.close();
            }
            if ui.button("Host").clicked() {
                let _ = self.run("adddevice", json!({ "kind": "host" }));
                ui.close();
            }
            ui.separator();
            if ui.button("Cable tool   C").clicked() {
                self.tool = Tool::Cable;
                self.link_start = None;
                ui.close();
            }
            if ui.button("Delete tool   X").clicked() {
                self.tool = Tool::Delete;
                self.link_start = None;
                ui.close();
            }
            if ui.button("Delete selected   Del").clicked() {
                self.delete_selected();
                ui.close();
            }
        });
        ui.menu_button("Simulate", |ui| {
            if ui.button("Ping selected host   F5").clicked() {
                self.demo_ping();
                ui.close();
            }
            ui.separator();
            if ui.button("Run   F6").clicked() {
                let _ = self.run("sim.run", json!({}));
                ui.close();
            }
            if ui.button("Pause   F7").clicked() {
                let _ = self.run("sim.pause", json!({}));
                ui.close();
            }
            if ui.button("Reset packet log   F8").clicked() {
                let _ = self.run("sim.reset", json!({}));
                ui.close();
            }
        });
        ui.menu_button("View", |ui| {
            if ui.button("Fit topology").clicked() {
                self.fit_requested = true;
                ui.close();
            }
            if ui.button("Reset zoom").clicked() {
                self.camera = Camera::default();
                self.fit_requested = true;
                ui.close();
            }
            if ui
                .button(if self.show_grid {
                    "Hide grid"
                } else {
                    "Show grid"
                })
                .clicked()
            {
                self.show_grid = !self.show_grid;
                ui.close();
            }
        });
        ui.menu_button("Help", |ui| {
            if ui.button("Command reference   F1").clicked() {
                self.help_open = true;
                ui.close();
            }
            if ui.button("About NetworksCraft").clicked() {
                self.about_open = true;
                ui.close();
            }
        });
    }

    fn live_pill(&self, ui: &mut Ui) {
        let (running, label) = if self.session.running {
            (T.success, "RUNNING")
        } else {
            (T.warning, "PAUSED")
        };
        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 20.0), Sense::hover());
        ui.painter()
            .circle_filled(pos2(rect.left() + 4.0, rect.center().y), 3.5, running);
        ui.painter().text(
            pos2(rect.left() + 13.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::monospace(9.0),
            T.muted,
        );
    }

    // ── status bar ──────────────────────────────────────────────────────────────────────────────

    fn status_bar(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("nc_status")
            .exact_size(24.0)
            .frame(
                Frame::new()
                    .fill(T.chrome)
                    .inner_margin(Margin::symmetric(10, 2))
                    .stroke(Stroke::new(1.0, T.stroke)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.painter().circle_filled(
                        pos2(ui.cursor().left() + 4.0, ui.cursor().center().y),
                        3.0,
                        theme::message_color(self.session.message_kind),
                    );
                    ui.add_space(14.0);
                    let message = truncate(&self.session.message, 110);
                    ui.label(
                        RichText::new(message)
                            .size(11.0)
                            .color(theme::message_color(self.session.message_kind)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} nodes · {} links · {} events",
                                self.session.topology.nodes.len(),
                                self.session.topology.links.len(),
                                self.session.events.len()
                            ))
                            .monospace()
                            .size(9.5)
                            .color(T.faint),
                        );
                        ui.add_space(14.0);
                        let file = self.session.file_label();
                        let dirty = if self.session.dirty { " •" } else { "" };
                        ui.label(
                            RichText::new(format!("{file}{dirty}"))
                                .monospace()
                                .size(9.5)
                                .color(if self.session.dirty {
                                    T.warning
                                } else {
                                    T.muted
                                }),
                        );
                    });
                });
            });
    }

    // ── left panel ─────────────────────────────────────────────────────────────────────────────

    fn left_panel(&mut self, ui: &mut Ui) {
        egui::Panel::left("nc_devices")
            .resizable(true)
            .default_size(216.0)
            .size_range(egui::Rangef::new(198.0, 280.0))
            .frame(theme::panel_frame(T.panel))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("nc_left_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                section(ui, "DEVICES", "click to place");
                for kind in DeviceKind::ALL {
                    if palette_card(ui, kind).clicked() {
                        self.add_from_palette(kind);
                    }
                }
                ui.add_space(8.0);
                section(ui, "TOOLS", "canvas mode");
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 12.0) / 3.0;
                    for (tool, icon, shortcut) in [
                        (Tool::Select, Icon::Select, "V"),
                        (Tool::Cable, Icon::Cable, "C"),
                        (Tool::Delete, Icon::Trash, "X"),
                    ] {
                        if tool_chip(ui, width, icon, tool.label(), shortcut, self.tool == tool)
                            .clicked()
                        {
                            self.tool = tool;
                            self.link_start = None;
                        }
                    }
                });
                ui.add_space(8.0);
                section(ui, "LABS", "one-click layouts");
                if lab_button(
                    ui,
                    "Starter lab",
                    "router · switch · two hosts",
                    Icon::Sample,
                )
                .clicked()
                {
                    let _ = self.run("sample", json!({}));
                }
                if lab_button(ui, "Campus lab", "two switches · four hosts", Icon::Campus).clicked()
                {
                    let _ = self.run("campus", json!({}));
                }
                if lab_button(ui, "Empty canvas", "start from scratch", Icon::New).clicked() {
                    let _ = self.run("newlab", json!({}));
                }
                ui.add_space(8.0);
                section(ui, "QUICK START", "the 30-second lesson");
                tip_card(
                    ui,
                    "Select a host, then run a ping. Watch ARP, MAC learning and ICMP appear in the packet log.",
                    "ping 10.0.0.2",
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Tip: V select · C cable · X delete · scroll to zoom")
                        .size(10.0)
                        .color(T.faint),
                );
                    });
            });
    }

    fn add_from_palette(&mut self, kind: DeviceKind) {
        let world = self.screen_to_canvas(self.canvas_rect.center());
        let count = self.session.topology.nodes.len();
        let jitter = (count % 5) as f32 * 24.0;
        let x = (world.x + jitter).clamp(60.0, 1540.0);
        let y = (world.y + ((count % 3) as f32 - 1.0) * 46.0).clamp(60.0, 940.0);
        let _ = self.run(
            "adddevice",
            json!({ "kind": kind.as_str(), "x": x, "y": y }),
        );
        self.tool = Tool::Select;
        self.link_start = None;
    }

    // ── right panel ────────────────────────────────────────────────────────────────────────────

    fn right_panel(&mut self, ui: &mut Ui) {
        egui::Panel::right("nc_inspector")
            .resizable(true)
            .default_size(292.0)
            .size_range(egui::Rangef::new(256.0, 360.0))
            .frame(theme::panel_frame(T.panel))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("nc_inspector_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        section(ui, "INSPECTOR", "lab and selection");
                        self.lab_card(ui);
                        ui.add_space(8.0);
                        self.selected_card(ui);
                        ui.add_space(8.0);
                        self.goal_card(ui);
                        ui.add_space(8.0);
                        self.simulation_card(ui);
                    });
            });
    }

    fn lab_card(&mut self, ui: &mut Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("LAB").monospace().size(9.5).color(T.faint));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (label, color) = if self.session.dirty {
                        ("UNSAVED", T.warning)
                    } else {
                        ("SAVED", T.success)
                    };
                    ui.label(RichText::new(label).monospace().size(9.0).color(color));
                });
            });
            ui.add_space(2.0);
            let title_resp = ui.add(
                egui::TextEdit::singleline(&mut self.title_buf)
                    .font(TextStyle::Heading)
                    .desired_width(f32::INFINITY)
                    .text_color(T.text)
                    .margin(Margin::symmetric(0, 2)),
            );
            if title_resp.lost_focus() && !self.title_buf.trim().is_empty() {
                let value = self.title_buf.trim().to_string();
                if value != self.session.topology.title {
                    let _ = self.run("settitle", json!({ "title": value }));
                }
            }
            if title_resp.lost_focus() {
                self.title_buf = self.session.topology.title.clone();
            }
            ui.label(
                RichText::new(format!(
                    "{} devices · {} cables",
                    self.session.topology.nodes.len(),
                    self.session.topology.links.len()
                ))
                .size(11.5)
                .color(T.muted),
            );
            let file_line = self
                .session
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| "not saved to a file".to_string());
            ui.label(
                RichText::new(file_line)
                    .monospace()
                    .size(10.0)
                    .color(T.faint),
            );
        });
    }

    fn selected_card(&mut self, ui: &mut Ui) {
        let selected = self.session.selected;
        if selected.is_none()
            && let Some(link_id) = self.session.selected_link
            && let Some(link) = self
                .session
                .topology
                .links
                .iter()
                .find(|link| link.id == link_id)
                .cloned()
        {
            let a = self
                .session
                .topology
                .node(link.a)
                .map(|node| node.name.clone())
                .unwrap_or_else(|| "?".to_string());
            let b = self
                .session
                .topology
                .node(link.b)
                .map(|node| node.name.clone())
                .unwrap_or_else(|| "?".to_string());
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter().circle_filled(
                        icon_rect.center(),
                        12.0,
                        color_with_alpha(T.accent, 20),
                    );
                    draw_icon(
                        ui.painter(),
                        Icon::Cable,
                        icon_rect.center(),
                        15.0,
                        T.accent,
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(
                            RichText::new(format!("Cable L{}", link.id.0))
                                .size(14.0)
                                .strong()
                                .color(T.text),
                        );
                        ui.label(RichText::new("selected cable").size(10.0).color(T.muted));
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{a}  ↔  {b}"))
                        .monospace()
                        .size(11.0)
                        .color(T.muted),
                );
                ui.add_space(6.0);
                if ui
                    .add_sized(
                        vec2(ui.available_width(), 28.0),
                        egui::Button::new(RichText::new("Delete cable").size(11.5).color(T.danger))
                            .fill(T.card)
                            .stroke(Stroke::new(1.0, T.danger)),
                    )
                    .clicked()
                {
                    let _ = self.run("deletelink", json!({ "linkId": link.id.0 }));
                }
            });
            return;
        }
        let node = selected.and_then(|id| self.session.topology.node(id).cloned());
        if let Some(node) = node {
            let color = theme::kind_color(node.kind);
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (icon_rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter().circle_filled(
                        icon_rect.center(),
                        12.0,
                        color_with_alpha(color, 22),
                    );
                    draw_device_glyph(ui.painter(), node.kind, icon_rect.center(), 15.0, color);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(&node.name).size(14.0).strong().color(T.text));
                        ui.label(
                            RichText::new(format!("{} · id {}", node.kind.label(), node.id.0))
                                .size(10.0)
                                .color(color),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} cable{}",
                                self.session.topology.ports(node.id).len(),
                                if self.session.topology.ports(node.id).len() == 1 {
                                    ""
                                } else {
                                    "s"
                                }
                            ))
                            .monospace()
                            .size(9.5)
                            .color(T.faint),
                        );
                    });
                });
                ui.add_space(6.0);
                field_label(ui, "NAME");
                let name_resp = ui.add(
                    egui::TextEdit::singleline(&mut self.name_buf)
                        .desired_width(f32::INFINITY)
                        .text_color(T.text)
                        .margin(Margin::symmetric(8, 5)),
                );
                if name_resp.lost_focus() {
                    let value = self.name_buf.trim().to_string();
                    if !value.is_empty() && value != node.name {
                        let _ = self.run("rename", json!({ "nodeId": node.id.0, "name": value }));
                    }
                    if value.is_empty() {
                        self.name_buf = node.name.clone();
                    }
                }
                ui.add_space(2.0);
                field_label(ui, "IPv4 ADDRESS");
                if node.kind == DeviceKind::Host {
                    let ip_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.ip_buf)
                            .hint_text("10.0.0.10")
                            .desired_width(f32::INFINITY)
                            .text_color(T.text)
                            .margin(Margin::symmetric(8, 5)),
                    );
                    if ip_resp.lost_focus() {
                        let value = self.ip_buf.trim().to_string();
                        let old = node.ipv4.clone().unwrap_or_default();
                        if !value.is_empty() && value != old {
                            let _ = self
                                .run("setipv4", json!({ "nodeId": node.id.0, "address": value }));
                        }
                    }
                } else {
                    ui.label(
                        RichText::new("Interface addressing arrives with routing.")
                            .size(10.5)
                            .color(T.faint),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if node.kind == DeviceKind::Host
                        && ui
                            .add_sized(
                                vec2((ui.available_width() - 8.0) * 0.62, 28.0),
                                egui::Button::new(
                                    RichText::new("Ping from host")
                                        .size(11.5)
                                        .color(T.on_accent),
                                )
                                .fill(T.accent)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                    {
                        self.demo_ping();
                    }
                    if ui
                        .add_sized(
                            vec2((ui.available_width() - 8.0).max(70.0), 28.0),
                            egui::Button::new(RichText::new("Delete").size(11.5).color(T.danger))
                                .fill(T.card)
                                .stroke(Stroke::new(1.0, T.danger)),
                        )
                        .clicked()
                    {
                        let _ = self.run("deletedevice", json!({ "nodeId": node.id.0 }));
                    }
                });
                ui.add_space(6.0);
                field_label(ui, "CONNECTED TO");
                let neighbours = self.session.topology.neighbours(node.id);
                if neighbours.is_empty() {
                    ui.label(RichText::new("No cables yet").size(10.5).color(T.faint));
                } else {
                    for neighbour in neighbours.iter().take(5) {
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                            ui.painter().circle_filled(
                                dot.center(),
                                3.0,
                                theme::kind_color(neighbour.kind),
                            );
                            ui.label(RichText::new(&neighbour.name).size(11.0).color(T.muted));
                            ui.label(
                                RichText::new(
                                    self.session
                                        .topology
                                        .find_link(node.id, neighbour.id)
                                        .map(|link| format!("L{}", link.id.0))
                                        .unwrap_or_default(),
                                )
                                .monospace()
                                .size(9.0)
                                .color(T.faint),
                            );
                        });
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("MAC {}", mac_for(node.id, 0)))
                        .monospace()
                        .size(9.5)
                        .color(T.faint),
                );
            });
        } else {
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("Nothing selected").size(13.0).color(T.muted));
                ui.label(
                    RichText::new("Click a device on the canvas to inspect it.")
                        .size(10.5)
                        .color(T.faint),
                );
            });
        }
    }

    fn goal_card(&mut self, ui: &mut Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("LAB GOAL")
                        .monospace()
                        .size(9.5)
                        .color(T.faint),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.session.goal_done {
                        ui.label(
                            RichText::new("COMPLETE")
                                .monospace()
                                .size(9.0)
                                .color(T.success),
                        );
                    } else {
                        ui.label(
                            RichText::new("WAITING")
                                .monospace()
                                .size(9.0)
                                .color(T.warning),
                        );
                    }
                });
            });
            ui.label(
                RichText::new("Reach another host")
                    .size(13.5)
                    .strong()
                    .color(T.text),
            );
            ui.label(
                RichText::new("Select a host, then send a successful ping.")
                    .size(10.5)
                    .color(T.muted),
            );
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 5.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(3), T.input);
            let fill_w = if self.session.goal_done {
                rect.width()
            } else {
                rect.width() * 0.08
            };
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, vec2(fill_w, rect.height())),
                CornerRadius::same(3),
                if self.session.goal_done {
                    T.success
                } else {
                    T.stroke_bright
                },
            );
            if let Some(outcome) = &self.session.last_outcome {
                let color = if outcome.is_success() {
                    T.success
                } else {
                    T.danger
                };
                let text = match outcome.rtt_ms() {
                    Some(rtt) => format!("last ping: {} · {rtt} ms", outcome.label()),
                    None => format!("last ping: {}", outcome.label()),
                };
                ui.label(RichText::new(text).size(10.0).color(color));
            }
        });
    }

    fn simulation_card(&mut self, ui: &mut Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("SIMULATION")
                        .monospace()
                        .size(9.5)
                        .color(T.faint),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (label, color) = if self.session.running {
                        ("RUNNING", T.success)
                    } else {
                        ("PAUSED", T.warning)
                    };
                    ui.label(RichText::new(label).monospace().size(9.0).color(color));
                });
            });
            ui.horizontal(|ui| {
                stat(ui, "events", &self.session.events.len().to_string());
                stat(ui, "trace", &self.session.trace_links.len().to_string());
                stat(
                    ui,
                    "ms",
                    &self
                        .session
                        .last_outcome
                        .as_ref()
                        .and_then(|o| o.rtt_ms())
                        .map_or_else(|| "—".to_string(), |v| v.to_string()),
                );
            });
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        vec2((ui.available_width() - 8.0) / 3.0, 26.0),
                        egui::Button::new(RichText::new("Run").size(11.0)),
                    )
                    .clicked()
                {
                    let _ = self.run("sim.run", json!({}));
                }
                if ui
                    .add_sized(
                        vec2((ui.available_width() - 8.0) / 2.0, 26.0),
                        egui::Button::new(RichText::new("Pause").size(11.0)),
                    )
                    .clicked()
                {
                    let _ = self.run("sim.pause", json!({}));
                }
                if ui
                    .add_sized(
                        vec2((ui.available_width() - 4.0).max(60.0), 26.0),
                        egui::Button::new(RichText::new("Reset").size(11.0)),
                    )
                    .clicked()
                {
                    let _ = self.run("sim.reset", json!({}));
                }
            });
        });
    }

    // ── bottom panel ───────────────────────────────────────────────────────────────────────────

    fn bottom_panel(&mut self, ui: &mut Ui) {
        egui::Panel::bottom("nc_console")
            .resizable(true)
            .default_size(296.0)
            .size_range(egui::Rangef::new(150.0, 330.0))
            .frame(theme::panel_frame(T.panel))
            .show(ui, |ui| {
                let width = ui.available_width();
                let height = ui.available_height();
                let term_width = (width * 0.55).max(340.0);
                let log_width = (width - term_width - 12.0).max(260.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        vec2(term_width, height),
                        Layout::top_down(Align::Min),
                        |ui| self.terminal(ui),
                    );
                    ui.separator();
                    ui.allocate_ui_with_layout(
                        vec2(log_width, height),
                        Layout::top_down(Align::Min),
                        |ui| self.packet_log(ui),
                    );
                });
            });
    }

    fn terminal(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("TERMINAL")
                    .monospace()
                    .size(9.0)
                    .color(T.faint),
            );
            ui.label(RichText::new("·").size(9.0).color(T.faint));
            ui.label(
                RichText::new(self.prompt_label())
                    .monospace()
                    .size(9.0)
                    .color(T.accent),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_link(ui, "clear").clicked() {
                    let _ = self.run("clearlog", json!({}));
                }
            });
        });
        ui.add_space(2.0);

        let input_height = 30.0;
        let log_height = (ui.available_height() - input_height - 8.0).max(60.0);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("nc_terminal_log")
            .max_height(log_height)
            .auto_shrink([false, false]);
        scroll = scroll.stick_to_bottom(true);
        scroll.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            if self.session.log.is_empty() {
                ui.label(
                    RichText::new("Console ready. Type help for commands.")
                        .monospace()
                        .size(10.0)
                        .color(T.faint),
                );
            }
            for line in &self.session.log {
                let color = theme::log_color(line.kind);
                ui.add(
                    egui::Label::new(
                        RichText::new(&line.text)
                            .monospace()
                            .size(10.8)
                            .color(color),
                    )
                    .wrap(),
                );
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(">").monospace().size(11.5).color(T.accent));
            let width = (ui.available_width() - 4.0).max(120.0);
            let response = ui.add_sized(
                vec2(width, 28.0),
                egui::TextEdit::singleline(&mut self.term_input)
                    .hint_text("type help or ping 10.0.0.2")
                    .font(TextStyle::Monospace)
                    .text_color(T.text)
                    .margin(Margin::symmetric(8, 5)),
            );
            let submitted =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if submitted {
                let text = std::mem::take(&mut self.term_input);
                if !text.trim().is_empty() {
                    let _ = self.cmdline(&text);
                }
                response.request_focus();
                self.focus_terminal = false;
            }
            if self.focus_terminal {
                response.request_focus();
                self.focus_terminal = false;
            }
        });
    }

    fn prompt_label(&self) -> String {
        self.session
            .selected
            .and_then(|id| self.session.topology.node(id))
            .map(|node| node.name.clone())
            .unwrap_or_else(|| "lab".to_string())
    }

    fn packet_log(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("PACKET LOG")
                    .monospace()
                    .size(9.0)
                    .color(T.faint),
            );
            ui.label(
                RichText::new(format!("{} events", self.session.events.len()))
                    .monospace()
                    .size(9.0)
                    .color(T.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if small_link(ui, "clear").clicked() {
                    let _ = self.run("sim.reset", json!({}));
                }
            });
        });
        ui.add_space(2.0);
        let button_h = 30.0;
        let log_h = (ui.available_height() - button_h - 10.0).max(50.0);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("nc_packet_log")
            .max_height(log_h)
            .auto_shrink([false, false]);
        scroll = scroll.stick_to_bottom(true);
        scroll.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            if self.session.events.is_empty() {
                ui.add_space(10.0);
                ui.label(
                    RichText::new("No packets captured yet.")
                        .size(11.5)
                        .color(T.muted),
                );
                ui.label(
                    RichText::new("Select a host and press Ping, or type ping 10.0.0.2.")
                        .size(10.0)
                        .color(T.faint),
                );
            } else {
                for (index, event) in self.session.events.iter().enumerate() {
                    event_row(ui, event, index % 2 == 1);
                }
            }
        });
        ui.add_space(4.0);
        if ui
            .add_sized(
                vec2(ui.available_width(), button_h),
                egui::Button::new(
                    RichText::new("Ping selected host   F5")
                        .size(11.5)
                        .color(T.on_accent),
                )
                .fill(T.accent)
                .stroke(Stroke::NONE),
            )
            .clicked()
        {
            self.demo_ping();
        }
    }

    // ── canvas ─────────────────────────────────────────────────────────────────────────────────

    fn canvas_panel(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(T.canvas))
            .show(ui, |ui| {
                let available = ui.available_size();
                let (response, painter) = ui.allocate_painter(available, Sense::click_and_drag());
                let rect = response.rect;
                self.canvas_rect = rect;
                painter.rect_filled(rect, CornerRadius::ZERO, T.canvas);
                if self.fit_requested {
                    self.fit_canvas(rect);
                    self.fit_requested = false;
                    self.fit_settle = 3;
                } else if self.fit_settle > 0 {
                    self.fit_canvas(rect);
                    self.fit_settle = self.fit_settle.saturating_sub(1);
                }
                if self.show_grid {
                    draw_grid(&painter, rect, self.camera);
                }

                let hover = if self.screenshot_path.is_some() {
                    None
                } else {
                    ui.input(|input| input.pointer.hover_pos())
                };
                let hovered_node = hover.and_then(|pointer| {
                    hit_node(&self.session.topology, rect, self.camera, pointer)
                });
                let mut hovered_link = None;

                // Links first, then nodes on top.
                for link in &self.session.topology.links {
                    let (Some(a), Some(b)) = (
                        self.session.topology.node(link.a),
                        self.session.topology.node(link.b),
                    ) else {
                        continue;
                    };
                    let a_pos = self.node_screen(rect, a.x, a.y);
                    let b_pos = self.node_screen(rect, b.x, b.y);
                    let selected = self.session.selected_link == Some(link.id);
                    let in_trace = self.session.trace_links.contains(&link.id.0);
                    draw_link(
                        &painter,
                        a_pos,
                        b_pos,
                        link.id,
                        selected,
                        in_trace,
                        self.camera.zoom,
                    );
                    if let Some(pointer) = hover
                        && distance_to_segment(pointer, a_pos, b_pos) < 7.0
                    {
                        hovered_link = Some(link.id);
                    }
                }

                let hit_radius = (26.0 * self.camera.zoom).clamp(18.0, 36.0) + 6.0;
                for node in &self.session.topology.nodes {
                    let center = self.node_screen(rect, node.x, node.y);
                    let selected = self.session.selected == Some(node.id);
                    let hovered = hover
                        .map(|pointer| pointer.distance(center) <= hit_radius)
                        .unwrap_or(false);
                    draw_node(&painter, node, center, self.camera.zoom, selected, hovered);
                    let _ = hovered;
                }
                if self.session.running && !self.session.trace_links.is_empty() {
                    self.draw_pulse(&painter, rect);
                }

                if let Some(start) = self.link_start
                    && let Some(node) = self.session.topology.node(start)
                    && let Some(pointer) = hover
                {
                    let from = self.node_screen(rect, node.x, node.y);
                    painter.extend(egui::Shape::dashed_line(
                        &[from, pointer],
                        Stroke::new(1.5, T.accent),
                        7.0,
                        5.0,
                    ));
                    painter.circle_filled(pointer, 4.0, T.accent);
                }

                self.canvas_overlays(&painter, rect, hovered_node, hovered_link);

                if hover.is_some() {
                    if hovered_node.is_some() {
                        ui.ctx().set_cursor_icon(if self.tool == Tool::Cable {
                            CursorIcon::Crosshair
                        } else {
                            CursorIcon::PointingHand
                        });
                    } else if hovered_link.is_some() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                }

                self.canvas_input(ui, &response, rect, hovered_node, hovered_link);
            });
    }

    fn canvas_input(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        rect: Rect,
        hovered_node: Option<NodeId>,
        hovered_link: Option<LinkId>,
    ) {
        if self.screenshot_path.is_some() {
            return;
        }
        // Middle-drag pans the canvas.
        if response.hovered() && ui.input(|i| i.pointer.button_down(PointerButton::Middle)) {
            let delta = ui.input(|i| i.pointer.delta());
            self.camera.pan += delta;
        }
        // Scroll zoom around the pointer.
        if response.hovered()
            && let Some(pointer) = ui.input(|i| i.pointer.hover_pos())
        {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                let old = self.camera.zoom;
                let new = (old * (1.0 + scroll * 0.0015)).clamp(0.45, 1.8);
                let world = self.screen_to_canvas(pointer);
                self.camera.zoom = new;
                self.camera.pan = pointer - rect.min - vec2(world.x * new, world.y * new);
            }
        }

        if self.tool == Tool::Select {
            if response.drag_started()
                && let Some(id) = hovered_node
                && let Some(node) = self.session.topology.node(id)
                && let Some(pointer) = response.interact_pointer_pos()
            {
                self.drag_node = Some((id, (node.x, node.y), pointer));
            }
            if response.dragged()
                && let Some((id, (start_x, start_y), origin)) = self.drag_node
                && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
            {
                let delta = (pointer - origin) / self.camera.zoom.max(0.01);
                let x = (start_x + delta.x).clamp(40.0, 1560.0);
                let y = (start_y + delta.y).clamp(40.0, 960.0);
                let _ = self.run("movedevice", json!({ "nodeId": id.0, "x": x, "y": y }));
            }
            if response.drag_stopped() {
                self.drag_node = None;
            }
        }

        if !response.clicked() {
            return;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return;
        };
        match self.tool {
            Tool::Select => {
                if let Some(id) = hovered_node {
                    let _ = self.run("select", json!({ "nodeId": id.0 }));
                    self.link_start = None;
                } else if let Some(link) = hovered_link {
                    self.session.selected = None;
                    self.session.selected_link = Some(link);
                } else {
                    self.session.selected = None;
                    self.session.selected_link = None;
                    let _ = self.run("select", json!({ "nodeId": null }));
                }
            }
            Tool::Cable => {
                if let Some(id) = hovered_node {
                    if let Some(first) = self.link_start {
                        if first != id {
                            let _ = self.run("link", json!({ "a": first.0, "b": id.0 }));
                            self.link_start = None;
                        }
                    } else {
                        self.link_start = Some(id);
                        self.session.selected = Some(id);
                        self.show_toast(
                            "Cable tool: click a second device",
                            crate::engine::MessageKind::Info,
                        );
                    }
                } else {
                    self.link_start = None;
                }
            }
            Tool::Delete => {
                if let Some(id) = hovered_node {
                    let _ = self.run("deletedevice", json!({ "nodeId": id.0 }));
                } else if let Some(link) = hovered_link {
                    let _ = self.run("deletelink", json!({ "linkId": link.0 }));
                }
            }
        }
        let _ = pointer;
    }

    fn draw_pulse(&self, painter: &egui::Painter, rect: Rect) {
        let links = &self.session.trace_links;
        if links.is_empty() {
            return;
        }
        let progress = self.pulse_t.clamp(0.0, 0.999);
        let segment = ((progress * links.len() as f32) as usize).min(links.len() - 1);
        let local = (progress * links.len() as f32) - segment as f32;
        let Some(link) = self
            .session
            .topology
            .links
            .iter()
            .find(|link| link.id.0 == links[segment])
        else {
            return;
        };
        let (Some(a), Some(b)) = (
            self.session.topology.node(link.a),
            self.session.topology.node(link.b),
        ) else {
            return;
        };
        let start = self.node_screen(rect, a.x, a.y);
        let end = self.node_screen(rect, b.x, b.y);
        let pos = start + (end - start) * local;
        painter.circle_filled(pos, 5.0, color_with_alpha(T.accent, 26));
        painter.circle_filled(pos, 3.0, T.accent);
    }

    fn canvas_overlays(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        hovered_node: Option<NodeId>,
        hovered_link: Option<LinkId>,
    ) {
        let pill = Rect::from_min_size(rect.left_top() + vec2(12.0, 12.0), vec2(176.0, 38.0));
        painter.rect_filled(pill, CornerRadius::same(RADIUS), T.card);
        painter.rect_stroke(
            pill,
            CornerRadius::same(RADIUS),
            Stroke::new(1.0, T.stroke),
            StrokeKind::Inside,
        );
        painter.text(
            pill.left_top() + vec2(10.0, 6.0),
            Align2::LEFT_TOP,
            "TOPOLOGY",
            FontId::monospace(8.0),
            T.faint,
        );
        painter.text(
            pill.left_top() + vec2(10.0, 19.0),
            Align2::LEFT_TOP,
            truncate(&self.session.topology.title, 24),
            FontId::proportional(11.5),
            T.text,
        );

        let right = Rect::from_min_size(
            pos2(rect.right() - 98.0, rect.top() + 12.0),
            vec2(86.0, 22.0),
        );
        painter.rect_filled(right, CornerRadius::same(RADIUS), T.card);
        painter.rect_stroke(
            right,
            CornerRadius::same(RADIUS),
            Stroke::new(1.0, T.stroke),
            StrokeKind::Inside,
        );
        painter.text(
            right.center(),
            Align2::CENTER_CENTER,
            format!("{:.0}%  scroll", self.camera.zoom * 100.0),
            FontId::monospace(8.5),
            T.muted,
        );

        let foot = pos2(rect.left() + 16.0, rect.bottom() - 18.0);
        painter.text(
            foot,
            Align2::LEFT_CENTER,
            format!(
                "{} devices · {} links · grid 32",
                self.session.topology.nodes.len(),
                self.session.topology.links.len()
            ),
            FontId::monospace(9.0),
            T.faint,
        );

        if self.tool == Tool::Cable {
            let chip =
                Rect::from_center_size(pos2(rect.center().x, rect.top() + 28.0), vec2(248.0, 24.0));
            painter.rect_filled(chip, CornerRadius::same(RADIUS), T.card);
            painter.rect_stroke(
                chip,
                CornerRadius::same(RADIUS),
                Stroke::new(1.0, T.accent),
                StrokeKind::Inside,
            );
            painter.text(
                chip.center(),
                Align2::CENTER_CENTER,
                if self.link_start.is_some() {
                    "CABLE — click the second device"
                } else {
                    "CABLE — click the first device"
                },
                FontId::proportional(10.0),
                T.accent,
            );
        }

        if let Some(id) = hovered_node
            && let Some(node) = self.session.topology.node(id)
        {
            let center = self.node_screen(rect, node.x, node.y);
            let tip =
                Rect::from_min_size(pos2(center.x - 84.0, center.y + 46.0), vec2(168.0, 21.0));
            painter.rect_filled(tip, CornerRadius::same(RADIUS), T.card);
            painter.rect_stroke(
                tip,
                CornerRadius::same(RADIUS),
                Stroke::new(1.0, T.stroke),
                StrokeKind::Inside,
            );
            painter.text(
                tip.center(),
                Align2::CENTER_CENTER,
                format!(
                    "{} · {} · {}",
                    node.kind.label(),
                    node.ipv4.as_deref().unwrap_or("no address"),
                    self.session.topology.ports(node.id).len()
                ),
                FontId::monospace(8.5),
                T.muted,
            );
        }
        if let Some(id) = hovered_link
            && let Some(link) = self.session.topology.links.iter().find(|l| l.id == id)
        {
            let (Some(a), Some(b)) = (
                self.session.topology.node(link.a),
                self.session.topology.node(link.b),
            ) else {
                return;
            };
            let pa = self.node_screen(rect, a.x, a.y);
            let pb = self.node_screen(rect, b.x, b.y);
            let mid = pa + (pb - pa) * 0.5;
            painter.text(
                mid + vec2(0.0, 24.0),
                Align2::CENTER_CENTER,
                format!("L{} · click to select", link.id.0),
                FontId::monospace(9.0),
                T.accent,
            );
        }

        if self.session.topology.nodes.is_empty() {
            let center = rect.center();
            let card = Rect::from_center_size(center, vec2(320.0, 96.0));
            painter.rect_filled(card, CornerRadius::same(RADIUS), T.card);
            painter.rect_stroke(
                card,
                CornerRadius::same(RADIUS),
                Stroke::new(1.0, T.stroke),
                StrokeKind::Inside,
            );
            painter.circle_stroke(
                center + vec2(0.0, -14.0),
                16.0,
                Stroke::new(1.5, T.stroke_bright),
            );
            painter.line_segment(
                [center + vec2(-6.0, -14.0), center + vec2(6.0, -14.0)],
                Stroke::new(1.5, T.accent),
            );
            painter.line_segment(
                [center + vec2(0.0, -20.0), center + vec2(0.0, -8.0)],
                Stroke::new(1.5, T.accent),
            );
            painter.text(
                center + vec2(0.0, 18.0),
                Align2::CENTER_CENTER,
                "Add a device from the palette",
                FontId::proportional(13.0),
                T.text,
            );
            painter.text(
                center + vec2(0.0, 36.0),
                Align2::CENTER_CENTER,
                "or type  adddevice host PC1",
                FontId::monospace(9.5),
                T.faint,
            );
        }
    }

    fn node_screen(&self, rect: Rect, x: f32, y: f32) -> Pos2 {
        rect.min + self.camera.pan + vec2(x * self.camera.zoom, y * self.camera.zoom)
    }

    fn screen_to_canvas(&self, screen: Pos2) -> Vec2 {
        (screen - self.canvas_rect.min - self.camera.pan) / self.camera.zoom.max(0.01)
    }

    fn fit_canvas(&mut self, rect: Rect) {
        if self.session.topology.nodes.is_empty() {
            self.camera.zoom = 0.9;
            self.camera.pan = vec2(60.0, 80.0);
            return;
        }
        let mut min = vec2(f32::MAX, f32::MAX);
        let mut max = vec2(f32::MIN, f32::MIN);
        for node in &self.session.topology.nodes {
            min.x = min.x.min(node.x);
            min.y = min.y.min(node.y);
            max.x = max.x.max(node.x);
            max.y = max.y.max(node.y);
        }
        let span = (max - min).max(vec2(200.0, 160.0));
        let zoom = ((rect.width() - 260.0) / span.x)
            .min((rect.height() - 220.0) / span.y)
            .clamp(0.5, 1.15);
        self.camera.zoom = zoom;
        let center = (min + max) * 0.5;
        self.camera.pan = rect.center() - rect.min - vec2(center.x * zoom, center.y * zoom);
    }

    // ── dialogs ────────────────────────────────────────────────────────────────────────────────

    fn toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let color = theme::message_color(toast.kind);
        let alpha = (toast.ttl / 1.2).clamp(0.0, 1.0) * 235.0;
        egui::Area::new(egui::Id::new("nc_toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -42.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(
                        T.chrome.r(),
                        T.chrome.g(),
                        T.chrome.b(),
                        alpha as u8,
                    ))
                    .corner_radius(CornerRadius::same(RADIUS))
                    .inner_margin(Margin::symmetric(12, 7))
                    .stroke(Stroke::new(1.0, color))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&toast.text).size(11.5).color(color));
                    });
            });
    }

    fn file_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let mode = dialog.mode;
        let title = match mode {
            FileMode::Open => "Open lab…",
            FileMode::SaveAs => "Save lab as…",
        };
        let mut close = false;
        let mut confirm: Option<String> = None;
        egui::Window::new(title)
            .id(egui::Id::new("nc_file_dialog"))
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -18.0))
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .frame(theme::panel_frame(T.panel))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(match mode {
                        FileMode::Open => "Choose a .nclab file. Files are plain JSON.",
                        FileMode::SaveAs => "Write the current lab as a .nclab file.",
                    })
                    .size(11.0)
                    .color(T.muted),
                );
                ui.add_space(4.0);
                let response = ui.add_sized(
                    vec2(ui.available_width(), 30.0),
                    egui::TextEdit::singleline(&mut dialog.path)
                        .font(TextStyle::Monospace)
                        .margin(Margin::symmetric(8, 5)),
                );
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    confirm = Some(dialog.path.clone());
                }
                let folder = Path::new(&dialog.path)
                    .parent()
                    .map(Path::to_path_buf)
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                ui.label(
                    RichText::new(format!("Folder: {}", folder.display()))
                        .size(9.5)
                        .monospace()
                        .color(T.faint),
                );
                ui.add_space(4.0);
                Frame::new()
                    .fill(T.input)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(6))
                    .show(ui, |ui| {
                        let mut listed = 0;
                        if let Ok(entries) = std::fs::read_dir(&folder) {
                            let mut paths: Vec<PathBuf> = entries
                                .filter_map(Result::ok)
                                .map(|entry| entry.path())
                                .filter(|path| {
                                    path.extension()
                                        .and_then(|ext| ext.to_str())
                                        .is_some_and(|ext| ext.eq_ignore_ascii_case("nclab"))
                                })
                                .collect();
                            paths.sort();
                            for path in paths.iter().take(8) {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(
                                                path.file_name()
                                                    .and_then(|name| name.to_str())
                                                    .unwrap_or("lab"),
                                            )
                                            .monospace()
                                            .size(10.5)
                                            .color(T.muted),
                                        )
                                        .fill(Color32::TRANSPARENT),
                                    )
                                    .clicked()
                                {
                                    dialog.path = path.display().to_string();
                                }
                                listed += 1;
                            }
                        }
                        if listed == 0 {
                            ui.label(
                                RichText::new("No .nclab files in this folder yet.")
                                    .size(10.0)
                                    .color(T.faint),
                            );
                        }
                    });
                if let Some(error) = &dialog.error {
                    ui.label(RichText::new(error).size(10.5).color(T.danger));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(RichText::new(match mode {
                                    FileMode::Open => "Open",
                                    FileMode::SaveAs => "Save",
                                }))
                                .fill(T.accent_soft)
                                .stroke(Stroke::new(1.0, T.accent)),
                            )
                            .clicked()
                        {
                            confirm = Some(dialog.path.clone());
                        }
                    });
                });
            });
        if let Some(mut path) = confirm {
            if mode == FileMode::SaveAs {
                path = force_lab_extension(Path::new(&path)).display().to_string();
            }
            let command = match mode {
                FileMode::Open => "openlab",
                FileMode::SaveAs => "savelab",
            };
            let result = self.run(command, json!({ "path": path }));
            match result {
                Ok(_) => self.dialog = None,
                Err(error) => {
                    if let Some(dialog) = &mut self.dialog {
                        dialog.error = Some(error.to_string());
                    }
                }
            }
        }
        if close {
            self.dialog = None;
        }
    }

    fn help_window(&mut self, ctx: &egui::Context) {
        if !self.help_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Command reference")
            .id(egui::Id::new("nc_help"))
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .frame(theme::panel_frame(T.panel))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Every action is a command. Type the id or an alias in the terminal, or use the menus. Params are JSON for scripts.")
                        .size(11.0)
                        .color(T.muted),
                );
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(430.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for spec in crate::engine::COMMANDS {
                            Frame::new()
                                .fill(T.card)
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::same(7))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(spec.id)
                                                .monospace()
                                                .size(11.0)
                                                .color(T.accent),
                                        );
                                        ui.label(
                                            RichText::new(spec.label).size(11.0).color(T.text),
                                        );
                                        if !spec.aliases.is_empty() {
                                            ui.label(
                                                RichText::new(format!(
                                                    "aliases: {}",
                                                    spec.aliases.join(", ")
                                                ))
                                                .monospace()
                                                .size(9.0)
                                                .color(T.faint),
                                            );
                                        }
                                    });
                                    ui.label(
                                        RichText::new(spec.params)
                                            .monospace()
                                            .size(9.5)
                                            .color(T.muted),
                                    );
                                });
                            ui.add_space(3.0);
                        }
                    });
            });
        self.help_open = open;
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new("About NetworksCraft")
            .id(egui::Id::new("nc_about"))
            .open(&mut open)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .frame(theme::panel_frame(T.panel))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("NETWORKSCRAFT")
                        .size(20.0)
                        .strong()
                        .color(T.text),
                );
                ui.label(
                    RichText::new("M1 alpha · v0.1.0")
                        .monospace()
                        .size(10.0)
                        .color(T.accent),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("An open, education-first network lab simulator. Every icon and pattern is drawn in code; no vendor artwork, fonts, help text or saved labs are used.")
                        .size(11.0)
                        .color(T.muted),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("File format: networkscraft-lab v1 · plain JSON · .nclab")
                        .monospace()
                        .size(9.5)
                        .color(T.faint),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Command reference").clicked() {
                        self.help_open = true;
                    }
                    if ui.button("Close").clicked() {
                        ui.close();
                    }
                });
            });
        self.about_open = open;
    }

    // ── screenshot mode ────────────────────────────────────────────────────────────────────────

    pub fn tick_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot_path.clone() else {
            return;
        };
        self.frames = self.frames.saturating_add(1);
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            if let egui::Event::Screenshot { image, .. } = event {
                match save_screenshot(&image, &path) {
                    Ok(()) => {
                        SCREENSHOT_STATE.store(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        eprintln!("screenshot save failed: {error}");
                        SCREENSHOT_STATE.store(2, Ordering::Relaxed);
                    }
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }
        if self.frames >= SCREENSHOT_FRAME && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if self.frames >= SCREENSHOT_TIMEOUT {
            eprintln!("screenshot timed out after {SCREENSHOT_TIMEOUT} frames");
            SCREENSHOT_STATE.store(2, Ordering::Relaxed);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn force_lab_extension(path: &Path) -> PathBuf {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("nclab"))
    {
        path.to_path_buf()
    } else {
        let mut out = path.to_path_buf();
        out.set_extension("nclab");
        out
    }
}

fn save_screenshot(image: &egui::ColorImage, path: &str) -> Result<(), String> {
    let width = image.width() as u32;
    let height = image.height() as u32;
    let mut bytes = Vec::with_capacity(image.pixels.len().saturating_mul(4));
    for pixel in &image.pixels {
        bytes.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
    }
    let Some(buffer) = image::RgbaImage::from_raw(width, height, bytes) else {
        return Err("screenshot buffer size did not match its dimensions".to_string());
    };
    let path = Path::new(path);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create screenshot folder: {error}"))?;
    }
    buffer
        .save(path)
        .map_err(|error| format!("could not write screenshot: {error}"))
}

// ── drawing helpers ─────────────────────────────────────────────────────────────────────────────

fn draw_grid(painter: &egui::Painter, rect: Rect, camera: Camera) {
    let step = (32.0 * camera.zoom).max(8.0);
    let start_x = rect.left() + (camera.pan.x.rem_euclid(step));
    let start_y = rect.top() + (camera.pan.y.rem_euclid(step));
    let mut index_x = 0i32;
    let mut x = start_x;
    while x < rect.right() {
        let color = if index_x % 4 == 0 {
            T.grid_major
        } else {
            T.grid
        };
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, color),
        );
        x += step;
        index_x += 1;
    }
    let mut index_y = 0i32;
    let mut y = start_y;
    while y < rect.bottom() {
        let color = if index_y % 4 == 0 {
            T.grid_major
        } else {
            T.grid
        };
        painter.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, color),
        );
        y += step;
        index_y += 1;
    }
}

fn draw_link(
    painter: &egui::Painter,
    a: Pos2,
    b: Pos2,
    id: LinkId,
    selected: bool,
    in_trace: bool,
    zoom: f32,
) {
    let color = if selected || in_trace {
        T.link_hot
    } else {
        T.link
    };
    if in_trace {
        painter.line_segment([a, b], Stroke::new(6.0, color_with_alpha(T.accent, 18)));
    }
    painter.line_segment([a, b], Stroke::new(if selected { 2.4 } else { 1.6 }, color));
    if zoom > 0.55 {
        let mid = a + (b - a) * 0.5;
        let chip = Rect::from_center_size(mid + vec2(0.0, -10.0), vec2(26.0, 13.0));
        painter.rect_filled(chip, CornerRadius::same(2), T.card);
        painter.rect_stroke(
            chip,
            CornerRadius::same(2),
            Stroke::new(1.0, if selected { T.accent } else { T.stroke }),
            StrokeKind::Inside,
        );
        painter.text(
            chip.center(),
            Align2::CENTER_CENTER,
            format!("L{}", id.0),
            FontId::monospace(8.0),
            if selected { T.accent } else { T.faint },
        );
    }
}

fn draw_node(
    painter: &egui::Painter,
    node: &crate::model::Node,
    center: Pos2,
    zoom: f32,
    selected: bool,
    hovered: bool,
) {
    let scale = zoom.clamp(0.55, 1.25);
    let color = theme::kind_color(node.kind);
    let size = match node.kind {
        DeviceKind::Router => vec2(46.0, 34.0),
        DeviceKind::Switch => vec2(48.0, 28.0),
        DeviceKind::Host => vec2(38.0, 32.0),
    } * scale;
    let body = Rect::from_center_size(center, size);

    if selected {
        painter.rect_filled(
            body.expand(4.0),
            CornerRadius::same(RADIUS),
            color_with_alpha(T.accent_soft, 220),
        );
        painter.rect_stroke(
            body.expand(4.0),
            CornerRadius::same(RADIUS),
            Stroke::new(1.5, T.accent),
            StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_filled(body.expand(3.0), CornerRadius::same(RADIUS), T.card_hover);
    }

    painter.rect_filled(body, CornerRadius::same(RADIUS), T.card);
    painter.rect_stroke(
        body,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, if hovered { T.stroke_bright } else { T.stroke }),
        StrokeKind::Inside,
    );
    draw_device_glyph(painter, node.kind, center, 20.0 * scale, color);

    if node.kind == DeviceKind::Switch {
        let ports = 6;
        let spacing = (body.width() - 16.0) / (ports as f32 - 1.0);
        for index in 0..ports {
            let px = body.left() + 8.0 + spacing * index as f32;
            painter.rect_filled(
                Rect::from_center_size(pos2(px, body.bottom() - 4.0), vec2(3.0, 2.0)),
                CornerRadius::same(1),
                color_with_alpha(color, 170),
            );
        }
    }

    let name_y = body.bottom() + 5.0;
    painter.text(
        pos2(center.x, name_y),
        Align2::CENTER_TOP,
        &node.name,
        FontId::proportional((11.5 * zoom).clamp(9.0, 12.5)),
        T.text,
    );
    let ip = node.ipv4.as_deref().unwrap_or("no address");
    painter.text(
        pos2(center.x, name_y + (13.0 * zoom).clamp(11.0, 15.0)),
        Align2::CENTER_TOP,
        ip,
        FontId::monospace((9.0 * zoom).clamp(7.5, 10.0)),
        if node.ipv4.is_some() {
            T.muted
        } else {
            T.faint
        },
    );
}

fn draw_device_glyph(
    painter: &egui::Painter,
    kind: DeviceKind,
    center: Pos2,
    size: f32,
    color: Color32,
) {
    let stroke = Stroke::new(1.6, color);
    match kind {
        DeviceKind::Router => {
            let body = Rect::from_center_size(center, vec2(size * 1.06, size * 0.72));
            painter.rect_stroke(body, CornerRadius::same(5), stroke, StrokeKind::Inside);
            let y = center.y;
            let left = center.x - size * 0.36;
            let right = center.x + size * 0.36;
            painter.line_segment([pos2(left, y), pos2(right, y)], stroke);
            painter.line_segment([pos2(right, y), pos2(right - 4.0, y - 3.0)], stroke);
            painter.line_segment([pos2(right, y), pos2(right - 4.0, y + 3.0)], stroke);
            painter.line_segment(
                [pos2(right, y - size * 0.2), pos2(left, y - size * 0.2)],
                Stroke::new(1.2, color_with_alpha(color, 180)),
            );
            painter.line_segment(
                [
                    pos2(left, y - size * 0.2),
                    pos2(left + 4.0, y - size * 0.2 - 3.0),
                ],
                Stroke::new(1.2, color_with_alpha(color, 180)),
            );
        }
        DeviceKind::Switch => {
            let body = Rect::from_center_size(center, vec2(size * 1.1, size * 0.64));
            painter.rect_stroke(body, CornerRadius::same(4), stroke, StrokeKind::Inside);
            let y = center.y + size * 0.16;
            let count = 5;
            let spacing = (body.width() - 12.0) / (count as f32 - 1.0);
            for index in 0..count {
                let x = body.left() + 6.0 + spacing * index as f32;
                painter.line_segment([pos2(x, y), pos2(x, y + 4.0)], Stroke::new(1.3, color));
            }
            painter.line_segment(
                [
                    pos2(center.x - size * 0.3, center.y - size * 0.12),
                    pos2(center.x + size * 0.3, center.y - size * 0.12),
                ],
                Stroke::new(1.2, color_with_alpha(color, 170)),
            );
        }
        DeviceKind::Host => {
            let body = Rect::from_center_size(
                center + vec2(0.0, -size * 0.08),
                vec2(size * 0.92, size * 0.64),
            );
            painter.rect_stroke(body, CornerRadius::same(3), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(center.x, body.bottom()),
                    pos2(center.x, body.bottom() + size * 0.14),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(center.x - size * 0.22, body.bottom() + size * 0.14),
                    pos2(center.x + size * 0.22, body.bottom() + size * 0.14),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(body.left() + size * 0.14, body.top() + size * 0.18),
                    pos2(body.left() + size * 0.26, body.top() + size * 0.18),
                ],
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [
                    pos2(body.left() + size * 0.31, body.top() + size * 0.18),
                    pos2(body.left() + size * 0.31, body.top() + size * 0.32),
                ],
                Stroke::new(1.3, color),
            );
            painter.line_segment(
                [
                    pos2(body.left() + size * 0.37, body.top() + size * 0.32),
                    pos2(body.left() + size * 0.5, body.top() + size * 0.18),
                ],
                Stroke::new(1.3, color),
            );
        }
    }
}

fn draw_logo(painter: &egui::Painter, center: Pos2, size: f32) {
    let top = center + vec2(0.0, -size * 0.32);
    let left = center + vec2(-size * 0.32, size * 0.26);
    let right = center + vec2(size * 0.32, size * 0.26);
    let line = Stroke::new(1.3, T.stroke_bright);
    painter.line_segment([top, left], line);
    painter.line_segment([top, right], line);
    painter.line_segment([left, right], line);
    painter.circle_filled(top, size * 0.1, T.accent);
    painter.circle_filled(left, size * 0.1, T.muted);
    painter.circle_filled(right, size * 0.1, T.muted);
}

fn draw_icon(painter: &egui::Painter, icon: Icon, center: Pos2, size: f32, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    let half = size * 0.5;
    match icon {
        Icon::Logo => draw_logo(painter, center, size),
        Icon::New | Icon::Plus => {
            painter.line_segment(
                [
                    center + vec2(-half * 0.6, 0.0),
                    center + vec2(half * 0.6, 0.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + vec2(0.0, -half * 0.6),
                    center + vec2(0.0, half * 0.6),
                ],
                stroke,
            );
        }
        Icon::Sample | Icon::Campus => {
            painter.circle_filled(center + vec2(-half * 0.55, 0.0), 2.2, color);
            painter.circle_filled(center + vec2(half * 0.55, -half * 0.45), 2.2, color);
            painter.circle_filled(center + vec2(half * 0.55, half * 0.45), 2.2, color);
            painter.line_segment(
                [
                    center + vec2(-half * 0.55, 0.0),
                    center + vec2(half * 0.55, -half * 0.45),
                ],
                Stroke::new(1.2, color),
            );
            painter.line_segment(
                [
                    center + vec2(-half * 0.55, 0.0),
                    center + vec2(half * 0.55, half * 0.45),
                ],
                Stroke::new(1.2, color),
            );
        }
        Icon::Open => {
            let rect =
                Rect::from_center_size(center + vec2(0.0, 1.0), vec2(size * 0.8, size * 0.55));
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [rect.left_top(), rect.left_top() + vec2(size * 0.28, 0.0)],
                stroke,
            );
            painter.line_segment(
                [
                    rect.left_top() + vec2(size * 0.28, 0.0),
                    rect.left_top() + vec2(size * 0.36, -size * 0.16),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    rect.left_top() + vec2(size * 0.36, -size * 0.16),
                    rect.right_top() + vec2(0.0, -size * 0.16),
                ],
                stroke,
            );
        }
        Icon::Save | Icon::SaveAs => {
            let rect = Rect::from_center_size(center, vec2(size * 0.72, size * 0.72));
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.rect_filled(
                Rect::from_min_size(
                    rect.left_top() + vec2(size * 0.2, 0.0),
                    vec2(size * 0.32, size * 0.22),
                ),
                CornerRadius::same(1),
                color_with_alpha(color, 120),
            );
            if icon == Icon::SaveAs {
                painter.line_segment(
                    [
                        center + vec2(size * 0.2, size * 0.24),
                        center + vec2(size * 0.48, size * 0.24),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        center + vec2(size * 0.34, size * 0.1),
                        center + vec2(size * 0.34, size * 0.38),
                    ],
                    stroke,
                );
            }
        }
        Icon::Run => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(-size * 0.24, -size * 0.3),
                    center + vec2(-size * 0.24, size * 0.3),
                    center + vec2(size * 0.32, 0.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            painter.rect_filled(
                Rect::from_center_size(
                    center + vec2(-size * 0.18, 0.0),
                    vec2(size * 0.12, size * 0.55),
                ),
                CornerRadius::same(1),
                color,
            );
            painter.rect_filled(
                Rect::from_center_size(
                    center + vec2(size * 0.18, 0.0),
                    vec2(size * 0.12, size * 0.55),
                ),
                CornerRadius::same(1),
                color,
            );
        }
        Icon::Reset => {
            painter.circle_stroke(center, size * 0.38, stroke);
            painter.line_segment(
                [
                    center + vec2(size * 0.38, 0.0),
                    center + vec2(size * 0.5, -size * 0.12),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + vec2(size * 0.38, 0.0),
                    center + vec2(size * 0.5, size * 0.12),
                ],
                stroke,
            );
        }
        Icon::Ping => {
            painter.circle_stroke(center, size * 0.42, stroke);
            painter.circle_stroke(center, size * 0.2, stroke);
            painter.circle_filled(center, 2.0, color);
        }
        Icon::Select => {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(-size * 0.25, -size * 0.35),
                    center + vec2(-size * 0.25, size * 0.3),
                    center + vec2(-size * 0.02, size * 0.12),
                    center + vec2(size * 0.18, size * 0.36),
                    center + vec2(size * 0.3, size * 0.24),
                    center + vec2(size * 0.1, size * 0.0),
                    center + vec2(size * 0.36, 0.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Cable => {
            painter.circle_stroke(center + vec2(-size * 0.3, 0.0), size * 0.14, stroke);
            painter.circle_stroke(center + vec2(size * 0.3, 0.0), size * 0.14, stroke);
            painter.line_segment(
                [
                    center + vec2(-size * 0.16, 0.0),
                    center + vec2(size * 0.16, 0.0),
                ],
                stroke,
            );
        }
        Icon::Trash => {
            let body = Rect::from_center_size(
                center + vec2(0.0, size * 0.08),
                vec2(size * 0.5, size * 0.54),
            );
            painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [
                    center + vec2(-size * 0.35, -size * 0.24),
                    center + vec2(size * 0.35, -size * 0.24),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + vec2(-size * 0.14, -size * 0.24),
                    center + vec2(-size * 0.14, -size * 0.32),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    center + vec2(size * 0.14, -size * 0.24),
                    center + vec2(size * 0.14, -size * 0.32),
                ],
                stroke,
            );
        }
        Icon::Router => draw_device_glyph(painter, DeviceKind::Router, center, size, color),
        Icon::Switch => draw_device_glyph(painter, DeviceKind::Switch, center, size, color),
        Icon::Host => draw_device_glyph(painter, DeviceKind::Host, center, size, color),
        Icon::Help => {
            painter.circle_stroke(center, size * 0.4, stroke);
            painter.text(
                center,
                Align2::CENTER_CENTER,
                "?",
                FontId::proportional(size * 0.55),
                color,
            );
        }
        Icon::Fit => {
            let d = size * 0.3;
            for (corner, dx, dy) in [
                (center + vec2(-size * 0.38, -size * 0.38), d, d),
                (center + vec2(size * 0.38, -size * 0.38), -d, d),
                (center + vec2(-size * 0.38, size * 0.38), d, -d),
                (center + vec2(size * 0.38, size * 0.38), -d, -d),
            ] {
                painter.line_segment([corner, corner + vec2(dx, 0.0)], stroke);
                painter.line_segment([corner, corner + vec2(0.0, dy)], stroke);
            }
        }
    }
}

fn toolbar_button(ui: &mut Ui, icon: Icon, label: &str, primary: bool) -> egui::Response {
    let font = FontId::proportional(11.5);
    let text_color = if primary { T.on_accent } else { T.text };
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), text_color)
        .size()
        .x;
    let width = 11.0 + 14.0 + 5.0 + text_width + 10.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let hovered = response.hovered();
    let fill = if primary {
        if hovered { T.accent_hover } else { T.accent }
    } else if hovered {
        T.card_hover
    } else {
        T.card
    };
    let stroke = if primary {
        Stroke::NONE
    } else if hovered {
        Stroke::new(1.0, T.stroke_bright)
    } else {
        Stroke::new(1.0, T.stroke)
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), fill);
    ui.painter()
        .rect_stroke(rect, CornerRadius::same(RADIUS), stroke, StrokeKind::Inside);
    let icon_color = if primary { T.on_accent } else { T.muted };
    draw_icon(
        ui.painter(),
        icon,
        pos2(rect.left() + 12.0, rect.center().y),
        14.0,
        icon_color,
    );
    ui.painter().text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn palette_card(ui: &mut Ui, kind: DeviceKind) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 46.0), Sense::click());
    let hovered = response.hovered();
    let color = theme::kind_color(kind);
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(RADIUS),
        if hovered { T.card_hover } else { T.card },
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, if hovered { T.stroke_bright } else { T.stroke }),
        StrokeKind::Inside,
    );
    let icon_center = pos2(rect.left() + 21.0, rect.center().y);
    draw_icon(
        ui.painter(),
        match kind {
            DeviceKind::Router => Icon::Router,
            DeviceKind::Switch => Icon::Switch,
            DeviceKind::Host => Icon::Host,
        },
        icon_center,
        19.0,
        color,
    );
    ui.painter().text(
        pos2(rect.left() + 38.0, rect.center().y - 7.0),
        Align2::LEFT_CENTER,
        kind.label(),
        FontId::proportional(12.0),
        T.text,
    );
    ui.painter().text(
        pos2(rect.left() + 38.0, rect.center().y + 7.0),
        Align2::LEFT_CENTER,
        kind.description(),
        FontId::proportional(8.8),
        T.faint,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn tool_chip(
    ui: &mut Ui,
    width: f32,
    icon: Icon,
    label: &str,
    shortcut: &str,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let hovered = response.hovered();
    let fill = if active {
        T.accent_soft
    } else if hovered {
        T.card_hover
    } else {
        T.card
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, if active { T.accent } else { T.stroke }),
        StrokeKind::Inside,
    );
    let color = if active { T.accent } else { T.muted };
    draw_icon(
        ui.painter(),
        icon,
        pos2(rect.left() + 13.0, rect.center().y),
        13.0,
        color,
    );
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(9.5),
        color,
    );
    response
        .on_hover_text(format!("{label} ({shortcut})"))
        .on_hover_cursor(CursorIcon::PointingHand)
}

fn lab_button(ui: &mut Ui, title: &str, subtitle: &str, icon: Icon) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    let hovered = response.hovered();
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(RADIUS),
        if hovered { T.card_hover } else { T.card },
    );
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0, if hovered { T.stroke_bright } else { T.stroke }),
        StrokeKind::Inside,
    );
    draw_icon(
        ui.painter(),
        icon,
        pos2(rect.left() + 16.0, rect.center().y),
        14.0,
        if hovered { T.accent } else { T.muted },
    );
    ui.painter().text(
        pos2(rect.left() + 30.0, rect.center().y - 5.5),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(11.0),
        T.text,
    );
    ui.painter().text(
        pos2(rect.left() + 30.0, rect.center().y + 7.0),
        Align2::LEFT_CENTER,
        subtitle,
        FontId::proportional(8.3),
        T.faint,
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn tip_card(ui: &mut Ui, text: &str, command: &str) {
    Frame::new()
        .fill(T.panel_alt)
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(8))
        .stroke(Stroke::new(1.0, T.stroke))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(9.8).color(T.muted));
            ui.add_space(3.0);
            ui.label(
                RichText::new(format!("> {command}"))
                    .monospace()
                    .size(10.0)
                    .color(T.accent),
            );
        });
}

fn section(ui: &mut Ui, title: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .monospace()
                .size(9.0)
                .strong()
                .color(T.muted),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new(hint).size(8.3).color(T.faint));
        });
    });
    ui.add_space(3.0);
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).monospace().size(8.0).color(T.faint));
}

fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(value).monospace().size(12.0).color(T.text));
        ui.label(RichText::new(label).monospace().size(7.5).color(T.faint));
    });
    ui.add_space(10.0);
}

fn small_link(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(9.0).color(T.faint))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    )
}

fn event_row(ui: &mut Ui, event: &crate::sim::SimEvent, alternate: bool) {
    let color = theme::event_color(event.kind);
    Frame::new()
        .fill(if alternate {
            color_with_alpha(T.card, 90)
        } else {
            Color32::TRANSPARENT
        })
        .inner_margin(Margin::symmetric(5, 1))
        .corner_radius(CornerRadius::same(4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{:02}", event.seq))
                        .monospace()
                        .size(8.5)
                        .color(T.faint),
                );
                let (dot_rect, _) = ui.allocate_exact_size(vec2(7.0, 14.0), Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 2.8, color);
                ui.label(
                    RichText::new(theme::event_label(event.kind))
                        .monospace()
                        .size(8.0)
                        .color(color),
                );
                ui.add(
                    egui::Label::new(RichText::new(&event.summary).size(10.0).color(
                        if event.kind == crate::sim::EventKind::Error {
                            T.danger
                        } else {
                            T.text
                        },
                    ))
                    .wrap(),
                );
                if let Some(link) = event.link_id {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (chip, _) = ui.allocate_exact_size(vec2(30.0, 15.0), Sense::hover());
                        ui.painter()
                            .rect_filled(chip, CornerRadius::same(4), T.panel_alt);
                        ui.painter().text(
                            chip.center(),
                            Align2::CENTER_CENTER,
                            format!("L{link}"),
                            FontId::monospace(8.0),
                            T.muted,
                        );
                    });
                }
            });
        });
}

fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let length_squared = ab.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(a);
    }
    let t = ((point - a).dot(ab) / length_squared).clamp(0.0, 1.0);
    point.distance(a + ab * t)
}

fn color_with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Convenience used by tests and the MCP/CLI later: the public command list with id and label.
pub fn command_catalog() -> Vec<(String, String)> {
    crate::engine::COMMANDS
        .iter()
        .map(|spec| (spec.id.to_string(), spec.label.to_string()))
        .collect()
}

/// Pure helper: the node at a screen position (also used by tests).
fn hit_node(topology: &Topology, rect: Rect, camera: Camera, pointer: Pos2) -> Option<NodeId> {
    let radius = (30.0 * camera.zoom).clamp(21.0, 44.0) + 8.0;
    topology
        .nodes
        .iter()
        .find(|node| {
            let center = rect.min + camera.pan + vec2(node.x * camera.zoom, node.y * camera.zoom);
            pointer.distance(center) <= radius
        })
        .map(|node| node.id)
}

/// Pure helper used by tests: choose the next available name for a kind.
pub fn next_name(topology: &Topology, kind: DeviceKind) -> String {
    for index in 1..=9999 {
        let candidate = format!("{}{}", kind.name_prefix(), index);
        if !topology.nodes.iter().any(|node| node.name == candidate) {
            return candidate;
        }
    }
    kind.name_prefix().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::find_command;

    #[test]
    fn truncate_respects_unicode() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 3), "he…");
        assert_eq!(truncate("aé日", 2), "a…");
    }

    #[test]
    fn with_alpha_sets_alpha() {
        let color = Color32::from_rgb(10, 20, 30);
        let faint = color_with_alpha(color, 40);
        assert_eq!(faint.a(), 40);
        assert_eq!(color_with_alpha(color, 255), color);
    }

    #[test]
    fn distance_to_segment_is_bounded() {
        let a = pos2(0.0, 0.0);
        let b = pos2(10.0, 0.0);
        assert!((distance_to_segment(pos2(5.0, 3.0), a, b) - 3.0).abs() < 0.001);
        assert!((distance_to_segment(pos2(-5.0, 0.0), a, b) - 5.0).abs() < 0.001);
        assert!((distance_to_segment(pos2(15.0, 0.0), a, b) - 5.0).abs() < 0.001);
    }

    #[test]
    fn next_name_is_stable() {
        let sample = crate::model::sample_lab();
        assert_eq!(next_name(&sample, DeviceKind::Host), "PC3");
        assert_eq!(next_name(&sample, DeviceKind::Router), "R2");
        assert_eq!(next_name(&sample, DeviceKind::Switch), "SW2");
    }

    #[test]
    fn hit_node_finds_the_nearest_icon() {
        let sample = crate::model::sample_lab();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0));
        let camera = Camera::default();
        let node = sample
            .nodes
            .iter()
            .find(|node| node.name == "PC1")
            .map(|node| (node.id, node.x, node.y));
        if let Some((id, x, y)) = node {
            let screen = rect.min + camera.pan + vec2(x * camera.zoom, y * camera.zoom);
            assert_eq!(hit_node(&sample, rect, camera, screen), Some(id));
        }
    }

    #[test]
    fn command_catalog_is_populated() {
        let catalog = command_catalog();
        assert!(catalog.len() > 10);
        assert!(catalog.iter().any(|(id, _)| id == "ping"));
        assert!(find_command("ping").is_some());
    }
}
