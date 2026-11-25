// Central design tokens and small shared widgets for Lilt's egui UI.
//
// Everything visual that repeats across pages lives here so the chrome, cards,
// menus and lists stay consistent. Colors are semantic rather than literal, and
// text-on-accent colors are chosen for contrast instead of assuming white.

const SURFACE_BASE: Color32 = Color32::from_rgb(10, 11, 16);
const SURFACE_LOW: Color32 = Color32::from_rgb(15, 17, 24);
const SURFACE_PANEL: Color32 = Color32::from_rgb(20, 23, 31);
const SURFACE_RAISED: Color32 = Color32::from_rgb(27, 31, 41);
const SURFACE_HOVER: Color32 = Color32::from_rgb(36, 42, 55);
const BORDER: Color32 = Color32::from_rgb(39, 45, 58);
const BORDER_STRONG: Color32 = Color32::from_rgb(58, 66, 84);
const TEXT_PRIMARY: Color32 = Color32::from_rgb(242, 244, 249);
const TEXT_MUTED: Color32 = Color32::from_rgb(170, 179, 197);
const TEXT_SUBTLE: Color32 = Color32::from_rgb(126, 138, 158);
const DANGER: Color32 = Color32::from_rgb(255, 107, 118);
const RADIUS_SM: u8 = 9;
const RADIUS_MD: u8 = 13;
const RADIUS_LG: u8 = 18;

/// Relative luminance (WCAG) used to decide whether dark or light text sits on
/// an accent-colored fill.
fn relative_luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn accent_text_color(accent: Color32) -> Color32 {
    if relative_luminance(accent) > 0.20 {
        Color32::from_rgb(15, 17, 24)
    } else {
        Color32::WHITE
    }
}

/// Apply the complete egui theme. Call this only when a theme-affecting
/// preference changes; egui keeps the style between frames.
fn apply_app_theme(context: &egui::Context, accent: Color32, compact: bool, animations: bool) {
    context.set_visuals(egui::Visuals::dark());
    context.style_mut(|style| {
        style.animation_time = if animations { 0.16 } else { 0.0 };
        style.interaction.selectable_labels = false;

        let visuals = &mut style.visuals;
        visuals.panel_fill = SURFACE_BASE;
        visuals.window_fill = SURFACE_PANEL;
        visuals.extreme_bg_color = SURFACE_LOW;
        visuals.faint_bg_color = SURFACE_RAISED;
        visuals.window_stroke = egui::Stroke::new(1.0_f32, BORDER);
        visuals.override_text_color = Some(TEXT_PRIMARY);
        visuals.hyperlink_color = accent;
        visuals.selection.bg_fill = accent;
        visuals.selection.stroke = egui::Stroke::new(1.5_f32, accent_text_color(accent));

        visuals.widgets.noninteractive.bg_fill = SURFACE_LOW;
        visuals.widgets.noninteractive.weak_bg_fill = SURFACE_LOW;
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_MUTED);
        visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(RADIUS_SM);

        visuals.widgets.inactive.bg_fill = SURFACE_RAISED;
        visuals.widgets.inactive.weak_bg_fill = SURFACE_RAISED;
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_PRIMARY);
        visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(RADIUS_MD);

        visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
        visuals.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, BORDER_STRONG);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(RADIUS_MD);

        visuals.widgets.active.bg_fill = accent;
        visuals.widgets.active.weak_bg_fill = accent;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0_f32, accent_text_color(accent));
        // egui uses the active-widget foreground for every `RichText::strong()`
        // label, not only pressed buttons. Keep it light so headings and track
        // titles stay readable on the dark panels; text drawn on an accent fill
        // sets its own contrast color explicitly.
        visuals.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, TEXT_PRIMARY);
        visuals.widgets.active.corner_radius = egui::CornerRadius::same(RADIUS_MD);

        visuals.widgets.open.bg_fill = SURFACE_HOVER;
        visuals.widgets.open.weak_bg_fill = SURFACE_HOVER;
        visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0_f32, BORDER_STRONG);
        visuals.widgets.open.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.open.corner_radius = egui::CornerRadius::same(RADIUS_MD);

        style.spacing.item_spacing = if compact {
            Vec2::new(8.0, 6.0)
        } else {
            Vec2::new(12.0, 10.0)
        };
        style.spacing.button_padding = if compact {
            Vec2::new(9.0, 6.0)
        } else {
            Vec2::new(12.0, 8.0)
        };
        style.spacing.interact_size = if compact {
            Vec2::new(36.0, 34.0)
        } else {
            Vec2::new(42.0, 40.0)
        };
        style.spacing.slider_rail_height = 4.0;

        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.5));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.5));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.5));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(30.0));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
    });
}

fn page_heading(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.label(
        RichText::new(title)
            .size(30.0)
            .strong()
            .color(TEXT_PRIMARY),
    );
    if let Some(subtitle) = subtitle.filter(|subtitle| !subtitle.is_empty()) {
        ui.label(RichText::new(subtitle).size(13.5).color(TEXT_MUTED));
    }
}

/// Rounded filter/tab chip used by settings and compact toolbars.
fn chip_button(ui: &mut egui::Ui, label: &str, selected: bool, accent: Color32) -> egui::Response {
    let fill = if selected {
        accent.linear_multiply(0.22)
    } else {
        SURFACE_RAISED
    };
    let stroke = if selected {
        egui::Stroke::new(1.0_f32, accent.linear_multiply(0.55))
    } else {
        egui::Stroke::new(1.0_f32, BORDER)
    };
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(13.0)
                .color(if selected { TEXT_PRIMARY } else { TEXT_MUTED }),
        )
        .fill(fill)
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(RADIUS_LG))
        .min_size(Vec2::new(0.0, 30.0)),
    )
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .strong()
            .color(TEXT_SUBTLE),
    );
}

fn empty_state(
    ui: &mut egui::Ui,
    icon: Option<&TextureHandle>,
    title: &str,
    body: &str,
    action: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        if let Some(icon) = icon {
            ui.add(
                egui::Image::new(icon)
                    .fit_to_exact_size(Vec2::splat(34.0))
                    .tint(TEXT_SUBTLE),
            );
            ui.add_space(8.0);
        }
        ui.label(RichText::new(title).size(18.0).strong().color(TEXT_PRIMARY));
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(RichText::new(body).size(13.0).color(TEXT_MUTED))
                .wrap_mode(egui::TextWrapMode::Wrap),
        );
        if let Some(action) = action {
            ui.add_space(14.0);
            let fill = ui.visuals().selection.bg_fill;
            clicked = ui
                .add_sized(
                    [190.0, 40.0],
                    egui::Button::new(RichText::new(action).color(accent_text_color(fill)))
                        .fill(fill),
                )
                .clicked();
        }
    });
    clicked
}

/// Clean placeholder for artwork that is missing or still being extracted.
fn paint_cover_placeholder(ui: &mut egui::Ui, rect: egui::Rect, seed: usize) {
    let fill = if seed % 3 == 0 {
        Color32::from_rgb(30, 34, 46)
    } else if seed % 3 == 1 {
        Color32::from_rgb(27, 31, 42)
    } else {
        Color32::from_rgb(33, 36, 49)
    };
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(RADIUS_MD),
        fill,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "♫",
        egui::FontId::proportional((rect.width() * 0.3).clamp(18.0, 38.0)),
        TEXT_SUBTLE,
    );
}
