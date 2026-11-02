//! Visual tokens and the egui style. Every colour in the panels comes from here.

use crate::engine::{LogKind, MessageKind};
use crate::model::{CableKind, DeviceKind};
use crate::sim::EventKind;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, Stroke, TextStyle, Visuals,
};

pub struct Tokens {
    pub bg: Color32,
    pub chrome: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub input: Color32,
    pub canvas: Color32,
    pub grid: Color32,
    pub grid_major: Color32,
    pub stroke: Color32,
    pub stroke_bright: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
    pub router: Color32,
    pub switch: Color32,
    pub host: Color32,
    pub link: Color32,
    pub link_hot: Color32,
    pub shadow: Color32,
}

pub const T: Tokens = Tokens {
    bg: Color32::from_rgb(236, 237, 234),
    chrome: Color32::from_rgb(245, 246, 243),
    panel: Color32::from_rgb(250, 250, 248),
    panel_alt: Color32::from_rgb(241, 242, 239),
    card: Color32::from_rgb(255, 255, 255),
    card_hover: Color32::from_rgb(239, 245, 250),
    input: Color32::from_rgb(255, 255, 255),
    canvas: Color32::from_rgb(252, 252, 250),
    grid: Color32::from_rgb(236, 238, 234),
    grid_major: Color32::from_rgb(224, 227, 222),
    stroke: Color32::from_rgb(212, 215, 210),
    stroke_bright: Color32::from_rgb(180, 185, 179),
    text: Color32::from_rgb(32, 41, 51),
    muted: Color32::from_rgb(86, 97, 107),
    faint: Color32::from_rgb(125, 135, 144),
    accent: Color32::from_rgb(31, 92, 153),
    accent_hover: Color32::from_rgb(23, 74, 124),
    accent_soft: Color32::from_rgb(231, 239, 248),
    on_accent: Color32::from_rgb(255, 255, 255),
    success: Color32::from_rgb(46, 125, 79),
    warning: Color32::from_rgb(166, 106, 18),
    danger: Color32::from_rgb(178, 59, 59),
    info: Color32::from_rgb(31, 92, 153),
    router: Color32::from_rgb(166, 101, 37),
    switch: Color32::from_rgb(46, 109, 164),
    host: Color32::from_rgb(46, 125, 79),
    link: Color32::from_rgb(138, 148, 141),
    link_hot: Color32::from_rgb(31, 92, 153),
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 30),
};

pub const RADIUS: u8 = 3;
pub const RADIUS_SMALL: u8 = 2;

pub fn install(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    let mut visuals = Visuals::light();

    visuals.override_text_color = Some(T.text);
    visuals.panel_fill = T.panel;
    visuals.window_fill = T.panel;
    visuals.extreme_bg_color = T.input;
    visuals.text_edit_bg_color = Some(T.input);
    visuals.code_bg_color = T.panel_alt;
    visuals.faint_bg_color = T.panel_alt;
    visuals.hyperlink_color = T.accent;
    visuals.warn_fg_color = T.warning;
    visuals.error_fg_color = T.danger;
    visuals.selection.bg_fill = T.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, T.accent);
    visuals.window_corner_radius = CornerRadius::same(RADIUS);
    visuals.window_stroke = Stroke::new(1.0, T.stroke_bright);
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow::NONE;
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_SMALL);
    visuals.widgets.noninteractive.bg_fill = T.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, T.stroke);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, T.muted);
    visuals.widgets.inactive.bg_fill = T.card;
    visuals.widgets.inactive.weak_bg_fill = T.card;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, T.stroke);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, T.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_SMALL);
    visuals.widgets.hovered.bg_fill = T.card_hover;
    visuals.widgets.hovered.weak_bg_fill = T.card_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, T.stroke_bright);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, T.text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_SMALL);
    visuals.widgets.active.bg_fill = T.accent_soft;
    visuals.widgets.active.weak_bg_fill = T.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, T.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, T.text);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_SMALL);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(6.0, 5.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.menu_margin = Margin::same(4);
    style.spacing.indent = 14.0;
    style.spacing.interact_size.y = 24.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(12.5));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(10.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(11.5));
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Light);
}

pub fn panel_frame(fill: Color32) -> Frame {
    Frame::new()
        .fill(fill)
        .inner_margin(Margin::same(10))
        .stroke(Stroke::new(1.0, T.stroke))
}

pub fn card_frame() -> Frame {
    Frame::new()
        .fill(T.card)
        .inner_margin(Margin::same(9))
        .corner_radius(CornerRadius::same(RADIUS))
        .stroke(Stroke::new(1.0, T.stroke))
}

pub fn kind_color(kind: DeviceKind) -> Color32 {
    match kind {
        DeviceKind::Router => T.router,
        DeviceKind::Switch => T.switch,
        DeviceKind::Host => T.host,
    }
}

pub fn cable_color(cable: CableKind) -> Color32 {
    match cable {
        CableKind::StraightThrough => T.link,
        CableKind::Crossover => Color32::from_rgb(62, 70, 76),
        CableKind::Console => Color32::from_rgb(66, 142, 176),
        CableKind::Serial => Color32::from_rgb(185, 64, 56),
        CableKind::Fiber => Color32::from_rgb(214, 126, 38),
    }
}

pub fn event_color(kind: EventKind) -> Color32 {
    match kind {
        EventKind::Arp => T.warning,
        EventKind::Learn => T.info,
        EventKind::Icmp => T.accent,
        EventKind::Info => T.success,
        EventKind::Error => T.danger,
    }
}

pub fn event_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Arp => "ARP",
        EventKind::Learn => "MAC",
        EventKind::Icmp => "ICMP",
        EventKind::Info => "OK",
        EventKind::Error => "ERR",
    }
}

pub fn message_color(kind: MessageKind) -> Color32 {
    match kind {
        MessageKind::Info => T.muted,
        MessageKind::Success => T.success,
        MessageKind::Error => T.danger,
    }
}

pub fn log_color(kind: LogKind) -> Color32 {
    match kind {
        LogKind::Input => T.accent,
        LogKind::Output => T.text,
        LogKind::Error => T.danger,
        LogKind::Info => T.muted,
    }
}
