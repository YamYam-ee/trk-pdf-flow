//! The toolbar's quick buttons: the favourite stamp and the favourite signature. A click picks the
//! favourite to place with the next click on the page; the arrow beside each chooses which of the
//! stamps or saved signatures is the favourite.

use egui::{Align2, Color32, CornerRadius, Sense, Stroke, vec2};
use pdfcraft_engine::StampKind;

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, QuickTool, icons, preset_stamps};

/// Which stamp the favourite-stamp button places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FavoriteStamp {
    /// A ready-made stamp ([`preset_stamps::PRESETS`]).
    Preset(usize),
    /// A standard stamp, by its `/Name`.
    Standard(&'static str),
    /// A custom stamp from the library, by its index.
    Custom(usize),
}

impl Default for FavoriteStamp {
    fn default() -> Self {
        FavoriteStamp::Preset(0)
    }
}

impl FavoriteStamp {
    /// The tool that places it (`None` if it no longer exists).
    pub fn tool(self, app: &PdfCraftApp) -> Option<QuickTool> {
        match self {
            FavoriteStamp::Preset(i) => preset_stamps::get(i).map(|_| QuickTool::PresetStamp(i)),
            FavoriteStamp::Standard(name) => StampKind::from_name(name.as_bytes()).map(QuickTool::Stamp),
            FavoriteStamp::Custom(i) => (i < app.custom_stamps.len()).then_some(QuickTool::CustomStamp(i)),
        }
    }

    /// Its name on the button's tooltip.
    pub fn label(self, app: &PdfCraftApp) -> String {
        match self {
            FavoriteStamp::Preset(i) => preset_stamps::get(i).map_or_else(String::new, |p| p.text().to_string()),
            FavoriteStamp::Standard(name) => StampKind::from_name(name.as_bytes()).map_or_else(String::new, |k| k.label().to_string()),
            FavoriteStamp::Custom(i) => app.custom_stamps.get(i).map_or_else(String::new, |c| c.name.clone()),
        }
    }

    /// The saved form (`"preset:0"`, `"standard:Approved"`, `"custom:2"`).
    pub fn to_key(self) -> String {
        match self {
            FavoriteStamp::Preset(i) => format!("preset:{i}"),
            FavoriteStamp::Standard(name) => format!("standard:{name}"),
            FavoriteStamp::Custom(i) => format!("custom:{i}"),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let (kind, rest) = key.split_once(':')?;
        match kind {
            "preset" => rest.parse().ok().filter(|i| preset_stamps::get(*i).is_some()).map(FavoriteStamp::Preset),
            "standard" => StampKind::from_name(rest.as_bytes()).map(|k| FavoriteStamp::Standard(k.name())),
            "custom" => rest.parse().ok().map(FavoriteStamp::Custom),
            _ => None,
        }
    }
}

/// A button with an arrow beside it: the button's response, and whether the arrow was clicked.
fn split_button(ui: &mut egui::Ui, icon: &str, active: bool, tip: &str, arrow_tip: &str) -> (egui::Response, egui::Response) {
    // Right to left: the arrow is added first so it sits to the right of the button.
    let arrow = icons::button(ui, "chevron-down", 16.0, false, arrow_tip);
    let button = icons::button(ui, icon, 32.0, active, tip);
    (button, arrow)
}

/// The two quick buttons, laid out right to left (left of the search box).
pub(crate) fn toolbar(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let has_doc = app.active.is_some();
    ui.add_enabled_ui(has_doc, |ui| {
        // Favourite signature.
        let has_sig = app.signature.current().is_some();
        let on = app.quick_tool == QuickTool::Fill(crate::fill_sign::FillTool::Signature);
        let tip = if has_sig { tl!("Place my favourite signature") } else { tl!("Add a signature") };
        let (button, arrow) = split_button(ui, "signature", on, tip, tl!("Choose the favourite signature"));
        if button.clicked() {
            app.run_command("sign.fill.signature");
        }
        let mut run = None;
        egui::Popup::menu(&arrow).show(|ui| {
            ui.label(egui::RichText::new(tl!("Favourite signature")).font(theme::semibold(13.0)));
            ui.label(egui::RichText::new(tl!("The one outlined is placed by the button.")).small().color(t.text_muted));
            run = crate::fill_sign::signature_entries(ui, app, t);
        });
        if let Some(cmd) = run {
            app.run_command(cmd);
        }
        ui.add_space(6.0);
        // Favourite stamp.
        let fav = app.favorite_stamp;
        let tool = fav.tool(app);
        let on = tool.is_some_and(|tool| app.quick_tool == tool);
        let tip = crate::i18n::fmt(tl!("Place my favourite stamp: {name}"), &[("name", &fav.label(app))]);
        let (button, arrow) = split_button(ui, "stamp", on, &tip, tl!("Choose the favourite stamp"));
        if button.clicked() {
            app.quick_tool = match tool {
                Some(tool) if !on => tool,
                Some(_) => QuickTool::Select,
                None => QuickTool::Select,
            };
        }
        egui::Popup::menu(&arrow).show(|ui| {
            if let Some(chosen) = stamp_menu(app, ui, t) {
                app.favorite_stamp = chosen;
                if let Some(tool) = chosen.tool(app) {
                    app.quick_tool = tool;
                }
                ui.close();
            }
        });
    });
}

/// Every stamp to choose the favourite from: the ready-made ones, the standard ones and the
/// custom ones. The current favourite is marked.
fn stamp_menu(app: &PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) -> Option<FavoriteStamp> {
    ui.set_width(250.0);
    ui.label(egui::RichText::new(tl!("Favourite stamp")).font(theme::semibold(13.0)));
    let mut chosen = None;
    egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
        crate::widgets::section_title(ui, tl!("Stamps"));
        for (i, p) in preset_stamps::PRESETS.iter().enumerate() {
            if chip(ui, t, p.text(), p.kind, app.favorite_stamp == FavoriteStamp::Preset(i)) {
                chosen = Some(FavoriteStamp::Preset(i));
            }
        }
        crate::widgets::section_title(ui, tl!("Standard stamps"));
        for k in StampKind::ALL {
            if chip(ui, t, k.label(), k, app.favorite_stamp == FavoriteStamp::Standard(k.name())) {
                chosen = Some(FavoriteStamp::Standard(k.name()));
            }
        }
        if !app.custom_stamps.is_empty() {
            crate::widgets::section_title(ui, tl!("Custom stamps"));
            for (i, c) in app.custom_stamps.iter().enumerate() {
                let fav = app.favorite_stamp == FavoriteStamp::Custom(i);
                if ui.selectable_label(fav, &c.name).clicked() {
                    chosen = Some(FavoriteStamp::Custom(i));
                }
            }
        }
    });
    chosen
}

/// One stamp in the menu, in its colours; a star marks the favourite. `true` when clicked.
fn chip(ui: &mut egui::Ui, t: &Tokens, text: &str, kind: StampKind, favourite: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, favourite, text));
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(5), t.hover);
    }
    let [r, g, b] = kind.color().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
    let col = Color32::from_rgb(r, g, b);
    let chip = egui::Rect::from_min_size(rect.min + vec2(26.0, 4.0), vec2((rect.width() - 34.0).max(40.0), 22.0));
    ui.painter().rect(chip, CornerRadius::same(4), col.gamma_multiply(0.1), Stroke::new(1.4, col), egui::StrokeKind::Inside);
    ui.painter().text(chip.center(), Align2::CENTER_CENTER, text, theme::semibold(11.0), col);
    if favourite {
        icons::paint(ui, egui::Rect::from_center_size(rect.left_center() + vec2(12.0, 0.0), vec2(14.0, 14.0)), "star", 13.0, t.accent);
    }
    resp.clicked()
}
