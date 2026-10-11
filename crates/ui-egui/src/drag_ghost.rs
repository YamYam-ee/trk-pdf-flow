//! While pages are dragged out of the window (to be saved as a new PDF), a small borderless
//! window follows the pointer over the desktop: the first page's thumbnail and the page count,
//! so the drag still shows what is being carried once it leaves the app.

use egui::{Align2, Color32, CornerRadius, Rect, Stroke, pos2, vec2};

use crate::theme;

/// The ghost's size, in points.
const SIZE: egui::Vec2 = vec2(120.0, 150.0);

impl crate::PdfCraftApp {
    /// Show the ghost while a page drag is outside the window.
    pub(crate) fn drag_ghost(&self, ctx: &egui::Context) {
        // Tests and the web draw every viewport inside the window: there is no desktop to show.
        if cfg!(target_arch = "wasm32") || ctx.embed_viewports() {
            return;
        }
        let Some(view) = self.active.and_then(|i| self.views.get(i)) else { return };
        let Some(pages) = view.panel_drag.as_ref().or(view.org_drag.as_ref()).filter(|p| !p.is_empty()) else { return };
        if !crate::canvas::dropped_outside_window(ctx) {
            return;
        }
        let (Some(inner), Some(pointer)) = (ctx.input(|i| i.viewport().inner_rect), ctx.input(|i| i.pointer.latest_pos())) else { return };
        let at = inner.min + pointer.to_vec2() + vec2(14.0, 14.0);
        let thumb = pages.iter().min().and_then(|&p| view.thumb(p)).map(|t| (t.id(), t.size_vec2()));
        let n = pages.len();
        let label = if n == 1 { tl!("1 page").to_string() } else { crate::i18n::fmt(tl!("{n} pages"), &[("n", &n.to_string())]) };
        let t = theme::Tokens::get(ctx);
        let builder = egui::ViewportBuilder::default()
            .with_title("PdfCraft")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_resizable(false)
            .with_active(false)
            .with_inner_size(SIZE)
            .with_position(at);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("pdfcraft-drag-ghost"), builder, |ui, _| {
            let painter = ui.painter();
            let card = Rect::from_min_size(pos2(0.0, 0.0), SIZE).shrink(2.0);
            painter.rect_filled(card, CornerRadius::same(8), t.panel);
            painter.rect_stroke(card, CornerRadius::same(8), Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            let page_area = Rect::from_min_max(card.min + vec2(10.0, 10.0), pos2(card.max.x - 10.0, card.max.y - 30.0));
            match thumb {
                Some((tex, size)) if size.x > 0.0 && size.y > 0.0 => {
                    let scale = (page_area.width() / size.x).min(page_area.height() / size.y);
                    let page = Rect::from_center_size(page_area.center(), size * scale);
                    painter.image(tex, page, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    painter.rect_stroke(page, CornerRadius::ZERO, Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
                }
                _ => crate::icons::paint(ui, page_area, if n == 1 { "file-text" } else { "files" }, 48.0, t.accent),
            }
            painter.text(pos2(card.center().x, card.max.y - 15.0), Align2::CENTER_CENTER, label.clone(), theme::medium(12.0), t.accent_text);
        });
        ctx.request_repaint();
    }
}
