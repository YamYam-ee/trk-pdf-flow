//! Edit text & images, as you type: the paragraph being edited is shown as it will be written.
//! A scratch copy of the document gets the draft written in by the same rewrite the edit uses
//! ([`Document::text_block_preview`]); its page is rendered in the background and drawn over
//! the page, and the caret and selection are drawn on the draft's own glyphs. Nothing touches
//! the document, its undo history or dirty flag until the edit is applied.

use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Stroke, TextureHandle, TextureOptions};
use pdfcraft_engine::Document;
use pdfcraft_render::{PageText, RenderPool, RenderRequest, RequestKind};

use crate::canvas::PageXform;
use crate::edit_text_ui::LineEditor;

/// How long typing must pause before a new draft is rendered (seconds).
const SETTLE: f64 = 0.12;

/// What a preview shows: the document state, the paragraph, its draft text and formatting.
#[derive(Clone, Debug, PartialEq)]
struct Draft {
    generation: u64,
    page: usize,
    block: usize,
    text: String,
    style: String,
}

impl Draft {
    fn same_paragraph(&self, other: &Draft) -> bool {
        (self.generation, self.page, self.block) == (other.generation, other.page, other.block)
    }
}

/// A rendered preview: the page texture and where each character of the draft was drawn.
struct Ready {
    draft: Draft,
    image: TextureHandle,
    glyphs: Option<Glyphs>,
}

/// The draft's glyphs in the preview's text layer.
pub(crate) struct Glyphs {
    text: Arc<PageText>,
    /// For each character of the draft: its glyph (whitespace has none).
    of_char: Vec<Option<usize>>,
    chars: Vec<char>,
}

#[derive(Default)]
pub(crate) struct TextPreview {
    /// The draft last seen, and when it last changed (egui time).
    seen: Option<(Draft, f64)>,
    /// What was last sent to the renderer: the draft, the scale and the build number.
    built: Option<(Draft, f32, u64)>,
    renderer: Option<RenderPool>,
    /// Results of the current build as they arrive (pixels, text layer).
    image: Option<TextureHandle>,
    text: Option<Option<Arc<PageText>>>,
    ready: Option<Ready>,
    /// The draft can't be written (the rewrite refused it): the plain editor shows instead.
    failed: bool,
    builds: u64,
    /// After the edit was applied: until when the preview may stand in for the page (egui time).
    settle_until: f64,
}

impl TextPreview {
    /// Keep the preview in step with the paragraph being edited, drawn at `scale` (pixels per point).
    pub fn prepare(&mut self, ctx: &egui::Context, doc: &Document, ed: &LineEditor, scale: f32) {
        let Some(p) = doc.info.pages.get(ed.page) else { return };
        let style = ed.style();
        let draft = Draft { generation: doc.edit_generation(), page: ed.page, block: ed.block, text: ed.text.clone(), style: format!("{style:?}") };
        let now = ctx.input(|i| i.time);
        if self.seen.as_ref().is_none_or(|(d, _)| *d != draft) {
            if self.seen.as_ref().is_none_or(|(d, _)| !d.same_paragraph(&draft)) {
                // Another paragraph, or the document changed under it: start over.
                *self = Self::default();
            }
            self.seen = Some((draft.clone(), now));
        }
        let changed_at = self.seen.as_ref().map_or(now, |(_, t)| *t);
        // The page texture matches the view up to one whole-page texture (as the signature preview).
        let scale = scale.min(crate::canvas::BASE_SIDE / p.width.max(p.height).max(1.0)).max(0.05);
        let stale = self.built.as_ref().is_none_or(|(d, s, _)| *d != draft || (*s - scale).abs() > 1e-3);
        if stale {
            let new_text = self.built.as_ref().is_none_or(|(d, ..)| *d != draft);
            // The first preview of a paragraph is built at once; later ones once typing pauses.
            let wait = SETTLE - (now - changed_at);
            if new_text && self.ready.is_some() && wait > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            } else {
                if new_text {
                    match doc.text_block_preview(ed.page, ed.block, &ed.text, &style) {
                        Ok(preview) => {
                            self.renderer = Some(preview.renderer());
                            self.failed = false;
                        }
                        Err(_) => {
                            self.renderer = None;
                            self.ready = None;
                            self.failed = true;
                            self.built = Some((draft, scale, self.builds));
                            return;
                        }
                    }
                }
                self.builds += 1;
                let tag = self.builds * 2;
                if let Some(r) = &self.renderer {
                    r.set_queue(vec![
                        RenderRequest { page: ed.page, scale, tag, ..Default::default() },
                        RenderRequest { page: ed.page, kind: RequestKind::Text, scale: 1.0, tag: tag + 1, ..Default::default() },
                    ]);
                }
                self.built = Some((draft, scale, self.builds));
                self.image = None;
                self.text = None;
            }
        }
        let Some((built, _, build)) = self.built.clone() else { return };
        if let Some(r) = &self.renderer {
            while let Some(out) = r.try_recv() {
                if out.request.tag / 2 != build {
                    continue;
                }
                if out.error.is_some() {
                    self.failed = true;
                    self.ready = None;
                    return;
                }
                if out.request.tag % 2 == 0 {
                    self.image = Some(ctx.load_texture(
                        "edit-text-preview",
                        ColorImage::from_rgba_premultiplied([out.width as usize, out.height as usize], &out.rgba),
                        TextureOptions::LINEAR,
                    ));
                } else {
                    self.text = Some(out.text.clone());
                }
            }
        }
        if let (Some(image), Some(text)) = (&self.image, &self.text) {
            let anchor = p.user_to_view(ed.source_rect()[0], ed.source_rect()[3]);
            let glyphs = text.as_ref().and_then(|t| Glyphs::find(t.clone(), &built.text, anchor));
            self.ready = Some(Ready { draft: built, image: image.clone(), glyphs });
            self.image = None;
            self.text = None;
        } else if self.renderer.is_some() && (self.image.is_none() || self.text.is_none()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    /// The paragraph was applied: keep showing the preview until the page's own raster of the
    /// edited document arrives ([`Self::page_received`]), so the old text doesn't flash back.
    pub fn settle(&mut self, ctx: &egui::Context) {
        self.seen = None;
        self.built = None;
        self.renderer = None;
        self.settle_until = ctx.input(|i| i.time) + 2.0;
    }

    /// The page's raster arrived: the preview has done its job.
    pub fn page_received(&mut self, page: usize) {
        if self.seen.is_none() && self.ready.as_ref().is_some_and(|r| r.draft.page == page) {
            *self = Self::default();
        }
    }

    /// Let go when no paragraph is being edited (or settling after one).
    pub fn idle(&mut self, ctx: &egui::Context) {
        if (self.seen.is_some() || ctx.input(|i| i.time) > self.settle_until) && (self.ready.is_some() || self.renderer.is_some()) {
            *self = Self::default();
        }
    }

    /// The draft's look is showing (drawn over the page, with its glyphs for the caret).
    pub fn covers(&self, page: usize) -> bool {
        !self.failed && self.ready.as_ref().is_some_and(|r| r.draft.page == page && r.glyphs.is_some())
    }

    /// The draft the page currently shows, once its preview and glyphs have arrived.
    pub fn shown_text(&self) -> Option<&str> {
        self.ready.as_ref().filter(|r| !self.failed && r.glyphs.is_some()).map(|r| r.draft.text.as_str())
    }

    /// The caret's screen line before character `c` of the shown draft.
    pub fn caret(&self, xf: &PageXform, c: usize) -> Option<(Pos2, Pos2)> {
        self.glyphs()?.caret(xf, c)
    }

    /// The draft can't be previewed: edit in the plain box.
    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn glyphs(&self) -> Option<&Glyphs> {
        self.ready.as_ref().filter(|_| !self.failed).and_then(|r| r.glyphs.as_ref())
    }

    /// Draw the preview over page `page`.
    pub fn paint(&self, painter: &egui::Painter, xf: &PageXform, page: usize) {
        let Some(r) = self.ready.as_ref().filter(|r| r.draft.page == page && !self.failed) else { return };
        if self.seen.is_none() && painter.ctx().input(|i| i.time) > self.settle_until {
            return;
        }
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
        xf.paint_image(&painter, r.image.id(), 0.0, 0.0, 1.0, 1.0);
    }
}

impl Glyphs {
    /// The glyphs drawing `draft` in `text`: the match nearest `anchor` (view space).
    fn find(text: Arc<PageText>, draft: &str, anchor: [f32; 2]) -> Option<Glyphs> {
        let ranges = text.find_opts(draft, true, false);
        let distance =
            |r: &std::ops::Range<usize>| text.glyphs.get(r.start).map_or(f32::INFINITY, |g| (g.rect[0] - anchor[0]).hypot(g.rect[1] - anchor[1]));
        let range = ranges.into_iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))?;
        let chars: Vec<char> = draft.chars().collect();
        let mut of_char = vec![None; chars.len()];
        let (mut g, mut within) = (range.start, 0usize);
        for (i, c) in chars.iter().enumerate() {
            if c.is_whitespace() {
                continue;
            }
            let Some(glyph) = text.glyphs.get(g).filter(|_| g < range.end) else { break };
            of_char[i] = Some(g);
            within += 1;
            if within >= glyph.text.chars().count().max(1) {
                g += 1;
                within = 0;
            }
        }
        Some(Glyphs { text, of_char, chars })
    }

    /// The glyph before character `c` (on its line), if `c` follows a non-space character.
    fn before(&self, c: usize) -> Option<usize> {
        let prev = c.checked_sub(1)?;
        if self.chars.get(prev).is_none_or(|ch| ch.is_whitespace()) {
            return None;
        }
        self.of_char.get(prev).copied().flatten()
    }

    /// The first glyph at or after character `c`.
    fn after(&self, c: usize) -> Option<usize> {
        self.of_char.iter().skip(c).find_map(|g| *g)
    }

    /// The caret before character `c`, as a screen line (bottom, top).
    pub fn caret(&self, xf: &PageXform, c: usize) -> Option<(Pos2, Pos2)> {
        let q = |g: usize, at_end: bool| {
            let quad = self.text.glyphs.get(g)?.quad;
            let (bottom, top) = if at_end { (quad[1], quad[2]) } else { (quad[0], quad[3]) };
            Some((xf.view_to_screen(bottom[0], bottom[1]), xf.view_to_screen(top[0], top[1])))
        };
        if let Some(g) = self.before(c) {
            return q(g, true);
        }
        if let Some(g) = self.after(c) {
            return q(g, false);
        }
        // Past the last glyph, after trailing spaces: just beyond it.
        let last = self.of_char.iter().take(c).rev().find_map(|g| *g)?;
        let (b, t) = q(last, true)?;
        let nudge = (t - b).length() * 0.25;
        let along = self.text.glyphs.get(last).map(|gl| gl.direction).unwrap_or([1.0, 0.0]);
        let dir = (xf.view_to_screen(along[0], along[1]) - xf.view_to_screen(0.0, 0.0)).normalized();
        Some((b + dir * nudge, t + dir * nudge))
    }

    /// Paint the selection of characters `a..b` and the caret at `caret`.
    pub fn paint(&self, painter: &egui::Painter, xf: &PageXform, a: usize, b: usize, caret: usize, accent: Color32) {
        let (a, b) = (a.min(b), a.max(b));
        let mut last = None;
        for g in self.of_char.iter().take(b).skip(a).flatten().copied() {
            if last == Some(g) {
                continue;
            }
            last = Some(g);
            if let Some(glyph) = self.text.glyphs.get(g) {
                let pts: Vec<Pos2> = glyph.quad.iter().map(|p| xf.view_to_screen(p[0], p[1])).collect();
                painter.add(egui::Shape::convex_polygon(pts, accent.gamma_multiply(0.3), Stroke::NONE));
            }
        }
        if let Some((bottom, top)) = self.caret(xf, caret) {
            painter.line_segment([bottom, top], Stroke::new(1.5, Color32::BLACK));
        }
    }

    /// The character position nearest a screen point: before or after the glyph closest to it.
    pub fn char_at(&self, xf: &PageXform, p: Pos2) -> Option<usize> {
        let mut best: Option<(f32, usize, bool)> = None;
        for (i, g) in self.of_char.iter().enumerate() {
            let Some(glyph) = g.and_then(|g| self.text.glyphs.get(g)) else { continue };
            let q: Vec<Pos2> = glyph.quad.iter().map(|v| xf.view_to_screen(v[0], v[1])).collect();
            let start = q[0].lerp(q[3], 0.5);
            let end = q[1].lerp(q[2], 0.5);
            let mid = start.lerp(end, 0.5);
            let d = p.distance(mid);
            // Only on or right beside the text: further away is another paragraph, or the page.
            if d > q[0].distance(q[3]) * 1.2 + 6.0 {
                continue;
            }
            if best.is_none_or(|(bd, ..)| d < bd) {
                // Past the glyph's middle along the line: the position after it.
                let after = (p - start).dot(end - start) > (end - start).length_sq() * 0.5;
                best = Some((d, i, after));
            }
        }
        best.map(|(_, i, after)| if after { self.end_of_glyph_chars(i) } else { i })
    }

    /// The position after the last character sharing `i`'s glyph (a ligature's characters).
    fn end_of_glyph_chars(&self, i: usize) -> usize {
        let g = self.of_char.get(i).copied().flatten();
        let mut j = i + 1;
        while j < self.of_char.len() && g.is_some() && self.of_char.get(j).copied().flatten() == g {
            j += 1;
        }
        j
    }
}
