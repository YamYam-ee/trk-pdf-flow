//! Edit text & images, as you type: the paragraph being edited is shown as it will be written.
//! Each change is written into a copy of the document by the same rewrite the edit uses
//! ([`Document::text_block_preview`]) and its page rendered on a worker thread, the newest draft
//! first; the result is drawn over the page, and the caret and selection are drawn on the
//! draft's own glyphs. Until the first result arrives the page itself shows the paragraph, and
//! its own glyphs carry the caret. Nothing touches the document, its undo history or dirty flag
//! until the edit is applied.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, ColorImage, Pos2, Stroke, TextureHandle, TextureOptions};
use pdfcraft_engine::Document;
use pdfcraft_render::{PageText, RenderedPage};

use crate::canvas::PageXform;
use crate::edit_text_ui::LineEditor;

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

/// The draft's glyphs in a page's text layer.
pub(crate) struct Glyphs {
    text: Arc<PageText>,
    /// For each character of the draft: its glyph (whitespace has none).
    of_char: Vec<Option<usize>>,
    chars: Vec<char>,
}

/// A worker's answer: the build it was for, its draft, and the page or why not.
type Answer = (u64, Draft, Result<(RenderedPage, Option<Arc<PageText>>), String>);

pub(crate) struct TextPreview {
    /// The paragraph's glyphs on the page as it is: the caret before any preview arrives.
    seed: Option<Glyphs>,
    /// The newest rendered draft.
    ready: Option<Ready>,
    /// The draft and scale last sent to a worker, and whether one is still at it.
    requested: Option<(Draft, f32)>,
    running: bool,
    builds: u64,
    sender: Sender<Answer>,
    answers: Receiver<Answer>,
    /// The paragraph being edited (to start over when another one opens).
    paragraph: Option<Draft>,
    /// The current draft can't be written (the rewrite refused it): the plain editor shows.
    failed: bool,
    /// After the edit was applied: until when the preview may stand in for the page (egui time).
    settle_until: Option<f64>,
}

impl Default for TextPreview {
    fn default() -> Self {
        let (sender, answers) = channel();
        Self {
            seed: None,
            ready: None,
            requested: None,
            running: false,
            builds: 0,
            sender,
            answers,
            paragraph: None,
            failed: false,
            settle_until: None,
        }
    }
}

/// The paragraph's top-left corner in view space, to find its glyphs by.
fn anchor(doc: &Document, ed: &LineEditor) -> Option<[f32; 2]> {
    let r = ed.source_rect();
    Some(doc.info.pages.get(ed.page)?.user_to_view(r[0], r[3]))
}

impl TextPreview {
    /// Keep the preview in step with the paragraph being edited, drawn at `scale` (pixels per
    /// point). `page_text` is the page's own text layer, if it has one yet.
    pub fn prepare(&mut self, ctx: &egui::Context, doc: &Document, ed: &LineEditor, scale: f32, page_text: Option<&Arc<PageText>>) {
        let Some(p) = doc.info.pages.get(ed.page) else { return };
        let style = ed.style();
        let draft = Draft { generation: doc.edit_generation(), page: ed.page, block: ed.block, text: ed.text.clone(), style: format!("{style:?}") };
        if self.paragraph.as_ref().is_none_or(|d| !d.same_paragraph(&draft)) {
            *self = Self { paragraph: Some(draft.clone()), ..Self::default() };
        }
        self.settle_until = None;
        let anchor = anchor(doc, ed);
        if self.seed.is_none()
            && let (Some(text), Some(anchor)) = (page_text, anchor)
        {
            self.seed = Glyphs::find(text.clone(), ed.original_text(), anchor);
        }
        // Answers: the newest build wins.
        while let Ok((build, done, result)) = self.answers.try_recv() {
            if build == self.builds {
                self.running = false;
            }
            match result {
                Ok((pixels, text)) if pixels.error.is_none() => {
                    let glyphs = text.zip(anchor).and_then(|(t, a)| Glyphs::find(t, &done.text, a));
                    let image = ctx.load_texture(
                        "edit-text-preview",
                        ColorImage::from_rgba_premultiplied([pixels.width as usize, pixels.height as usize], &pixels.rgba),
                        TextureOptions::LINEAR,
                    );
                    self.ready = Some(Ready { draft: done, image, glyphs });
                    self.failed = false;
                }
                _ => {
                    if done == draft {
                        self.failed = true;
                    }
                }
            }
        }
        // The page texture matches the view up to one whole-page texture (as the signature preview).
        let scale = scale.min(crate::canvas::BASE_SIDE / p.width.max(p.height).max(1.0)).max(0.05);
        let shown = self.ready.as_ref().map(|r| &r.draft);
        // Nothing to render while the paragraph is as it is on the page.
        let unchanged = !ed.has_changes() && shown.is_none();
        let wanted = (draft.clone(), scale);
        if !unchanged && !self.running && self.requested.as_ref() != Some(&wanted) {
            match doc.text_block_preview(ed.page, ed.block, &ed.text, &style) {
                Ok(job) => {
                    self.builds += 1;
                    let (build, tx, done) = (self.builds, self.sender.clone(), draft.clone());
                    let repaint = ctx.clone();
                    let spawned = std::thread::Builder::new().name("edit text preview".into()).spawn(move || {
                        let _ = tx.send((build, done, job.render(scale)));
                        repaint.request_repaint();
                    });
                    self.running = spawned.is_ok();
                    self.failed = spawned.is_err();
                }
                Err(_) => self.failed = true,
            }
            self.requested = Some(wanted);
        }
        if self.running {
            ctx.request_repaint_after(std::time::Duration::from_millis(8));
        }
    }

    /// The paragraph was applied: keep showing the preview until the page's own raster of the
    /// edited document arrives ([`Self::page_received`]), so the old text doesn't flash back.
    pub fn settle(&mut self, ctx: &egui::Context) {
        self.paragraph = None;
        self.seed = None;
        self.settle_until = Some(ctx.input(|i| i.time) + 2.0);
    }

    /// The page's raster arrived: the preview has done its job.
    pub fn page_received(&mut self, page: usize) {
        if self.settle_until.is_some() && self.ready.as_ref().is_some_and(|r| r.draft.page == page) {
            *self = Self::default();
        }
    }

    /// Let go when no paragraph is being edited (or once settling after one is over).
    pub fn idle(&mut self, ctx: &egui::Context) {
        let settling = self.settle_until.is_some_and(|t| ctx.input(|i| i.time) <= t);
        if !settling && (self.paragraph.is_some() || self.ready.is_some() || self.seed.is_some()) {
            *self = Self::default();
        }
    }

    /// The caret and selection are drawn on glyphs (the preview's, or the page's own).
    pub fn covers(&self, page: usize) -> bool {
        self.glyphs().is_some() && self.paragraph.as_ref().is_some_and(|d| d.page == page)
    }

    /// The draft the page currently shows, once its preview and glyphs have arrived.
    pub fn shown_text(&self) -> Option<&str> {
        self.ready.as_ref().filter(|r| !self.failed && r.glyphs.is_some()).map(|r| r.draft.text.as_str())
    }

    /// The caret's screen line before character `c` of the shown draft.
    pub fn caret(&self, xf: &PageXform, c: usize) -> Option<(Pos2, Pos2)> {
        self.glyphs()?.caret(xf, c)
    }

    /// The screen box around the draft as the page shows it (the preview's glyphs).
    pub fn bounds(&self, xf: &PageXform, page: usize) -> Option<egui::Rect> {
        if self.paragraph.as_ref().is_none_or(|d| d.page != page) {
            return None;
        }
        self.glyphs()?.bounds(xf)
    }

    /// The draft can't be previewed: edit in the plain box.
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// The glyphs the caret goes on: the newest preview's, or before one, the page's own.
    pub fn glyphs(&self) -> Option<&Glyphs> {
        if self.failed {
            return None;
        }
        match &self.ready {
            Some(r) => r.glyphs.as_ref(),
            None => self.seed.as_ref(),
        }
    }

    /// Draw the preview over page `page`.
    pub fn paint(&self, painter: &egui::Painter, xf: &PageXform, page: usize) {
        let Some(r) = self.ready.as_ref().filter(|r| r.draft.page == page && !self.failed) else { return };
        if self.settle_until.is_some_and(|t| painter.ctx().input(|i| i.time) > t) {
            return;
        }
        let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
        xf.paint_image(&painter, r.image.id(), 0.0, 0.0, 1.0, 1.0);
    }
}

impl Glyphs {
    /// The glyphs drawing `draft` in `text`: the match nearest `anchor` (view space).
    pub(crate) fn find(text: Arc<PageText>, draft: &str, anchor: [f32; 2]) -> Option<Glyphs> {
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
            // Space glyphs (drawn spaces) belong to the draft's whitespace, which has none.
            while g < range.end && text.glyphs.get(g).is_some_and(|gl| gl.text.trim().is_empty()) {
                g += 1;
                within = 0;
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

    /// The screen box around the draft's glyphs.
    pub fn bounds(&self, xf: &PageXform) -> Option<egui::Rect> {
        let mut seen = None;
        let mut r = egui::Rect::NOTHING;
        for g in self.of_char.iter().flatten().copied() {
            if seen == Some(g) {
                continue;
            }
            seen = Some(g);
            if let Some(glyph) = self.text.glyphs.get(g) {
                for p in glyph.quad {
                    r.extend_with(xf.view_to_screen(p[0], p[1]));
                }
            }
        }
        r.is_positive().then_some(r)
    }

    /// The glyph before character `c` (on its line), if `c` follows a non-space character.
    fn before(&self, c: usize) -> Option<usize> {
        let prev = c.checked_sub(1)?;
        if self.chars.get(prev).is_none_or(|ch| ch.is_whitespace()) {
            return None;
        }
        self.of_char.get(prev).copied().flatten()
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
        let breaks = |range: std::ops::Range<usize>| self.chars.get(range).map_or(0, |s| s.iter().filter(|ch| **ch == '\n').count());
        let next = self.of_char.iter().enumerate().skip(c).find_map(|(i, g)| g.map(|g| (i, g)));
        match next {
            // On the next glyph's line: where it starts.
            Some((i, g)) if breaks(c..i) == 0 => return q(g, false),
            // Empty lines before it (Enter pressed twice): the start of those lines.
            Some((i, g)) => return self.line_start(xf, g, -(breaks(c..i) as f32)),
            None => {}
        }
        // After the last glyph: on a new line when Enter was pressed after it.
        let (i, last) = self.of_char.iter().enumerate().take(c).rev().find_map(|(i, g)| g.map(|g| (i, g)))?;
        let down = breaks(i + 1..c);
        if down > 0 {
            return self.line_start(xf, last, down as f32);
        }
        // Past the last glyph, after trailing spaces: just beyond it.
        let last = self.of_char.iter().take(c).rev().find_map(|g| *g)?;
        let (b, t) = q(last, true)?;
        let nudge = (t - b).length() * 0.25;
        let along = self.text.glyphs.get(last).map(|gl| gl.direction).unwrap_or([1.0, 0.0]);
        let dir = (xf.view_to_screen(along[0], along[1]) - xf.view_to_screen(0.0, 0.0)).normalized();
        Some((b + dir * nudge, t + dir * nudge))
    }

    /// The caret at the start of the line `lines` lines below glyph `g`'s (above, when negative):
    /// at the paragraph's left edge (its first glyph's start), a line being 1.2 × the glyph's height.
    fn line_start(&self, xf: &PageXform, g: usize, lines: f32) -> Option<(Pos2, Pos2)> {
        let glyph = self.text.glyphs.get(g)?;
        let first = self.of_char.iter().find_map(|g| *g).and_then(|f| self.text.glyphs.get(f))?;
        let v = |p: [f32; 2]| egui::vec2(p[0], p[1]);
        let (bottom, top) = (v(glyph.quad[0]), v(glyph.quad[3]));
        let dir = v(glyph.direction);
        let start = bottom + dir * (v(first.quad[0]) - bottom).dot(dir);
        let shift = (bottom - top) * 1.2 * lines;
        let (b, t) = (start + shift, start + shift + (top - bottom));
        Some((xf.view_to_screen(b.x, b.y), xf.view_to_screen(t.x, t.y)))
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
