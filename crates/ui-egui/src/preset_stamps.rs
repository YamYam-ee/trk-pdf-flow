//! Sign & Stamp ▸ Stamps: large ready-made stamps ("DONE", "INVOICED", "BLOCKED" …), worded in
//! Swedish when the interface is Swedish and in English otherwise. Each is drawn in the colour and
//! shape of a standard stamp kind, with its own words (`Shape::Stamp { text }`).

use egui::{Align2, Color32, CornerRadius, Sense, Stroke, vec2};
use pdfcraft_engine::StampKind;

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, QuickTool, widgets};

/// One ready-made stamp: its kind (colour and shape) and its words.
pub struct PresetStamp {
    pub kind: StampKind,
    pub en: &'static str,
    pub sv: &'static str,
}

impl PresetStamp {
    /// The words in the interface language.
    pub fn text(&self) -> &'static str {
        if crate::i18n::current().code() == "sv" { self.sv } else { self.en }
    }
}

const fn p(kind: StampKind, en: &'static str, sv: &'static str) -> PresetStamp {
    PresetStamp { kind, en, sv }
}

/// Green for done and approved, blue for information, red for stops and refusals.
pub const PRESETS: [PresetStamp; 18] = [
    p(StampKind::Completed, "DONE", "KLAR"),
    p(StampKind::InformationOnly, "INVOICED", "FAKTURERAD"),
    p(StampKind::Void, "BLOCKED", "SPÄRRAD"),
    p(StampKind::NotApproved, "REJECTED", "UNDERKÄND"),
    p(StampKind::Approved, "APPROVED", "GODKÄND"),
    p(StampKind::Completed, "PAID", "BETALD"),
    p(StampKind::Final, "RECEIVED", "MOTTAGEN"),
    p(StampKind::Approved, "CHECKED", "KONTROLLERAD"),
    p(StampKind::ForComment, "SENT", "SKICKAD"),
    p(StampKind::InformationOnly, "REGISTERED", "REGISTRERAD"),
    p(StampKind::InformationOnly, "ARCHIVED", "ARKIVERAD"),
    p(StampKind::PreliminaryResults, "PENDING", "AVVAKTAR"),
    p(StampKind::Draft, "COPY", "KOPIA"),
    p(StampKind::Final, "ORIGINAL", "ORIGINAL"),
    p(StampKind::Confidential, "URGENT", "BRÅDSKANDE"),
    p(StampKind::Void, "CANCELLED", "MAKULERAD"),
    p(StampKind::NotApproved, "NOT PAID", "OBETALD"),
    p(StampKind::Confidential, "CONFIDENTIAL", "KONFIDENTIELL"),
];

pub fn get(i: usize) -> Option<&'static PresetStamp> {
    PRESETS.get(i)
}

/// The line under a ready-made stamp: today's date (YYYY-MM-DD) and the author, if any.
pub fn date_line(session: &pdfcraft_engine::Session, author: &str) -> String {
    let (y, m, d) = session.today();
    let date = format!("{y}-{m:02}-{d:02}");
    match author.trim() {
        "" => date,
        who => format!("{date}  {who}"),
    }
}

fn color(kind: StampKind) -> Color32 {
    let [r, g, b] = kind.color().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
    Color32::from_rgb(r, g, b)
}

/// The ready-made stamps as chips, two to a row; a click picks one to place with the next click
/// on the page.
pub fn section(app: &mut PdfCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    widgets::section_title(ui, tl!("Stamps"));
    ui.label(egui::RichText::new(tl!("Choose a stamp, then click on the page to place it.")).small().color(t.text_faint));
    ui.checkbox(&mut app.preset_stamp_date, tl!("Add today's date and my name"));
    let gap = 6.0;
    let w = ((ui.available_width() - gap) / 2.0).max(60.0);
    for row in PRESETS.chunks(2).enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (j, stamp) in row.1.iter().enumerate() {
                let i = row.0 * 2 + j;
                let active = app.quick_tool == QuickTool::PresetStamp(i);
                let text = stamp.text();
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, text));
                if active {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), t.accent_soft);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(6), t.hover);
                }
                let col = color(stamp.kind);
                let chip = rect.shrink2(vec2(4.0, 4.0));
                ui.painter().rect(chip, CornerRadius::same(5), col.gamma_multiply(0.1), Stroke::new(1.8, col), egui::StrokeKind::Inside);
                ui.painter().text(chip.center(), Align2::CENTER_CENTER, text, theme::semibold(12.0), col);
                if resp.clicked() {
                    app.quick_tool = if active { QuickTool::Select } else { QuickTool::PresetStamp(i) };
                }
            }
        });
        ui.add_space(2.0);
    }
}
