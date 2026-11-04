//! Weld joint markers (TRK): mark where a joint is and how it was tested, with a colour-coded
//! badge (`RT-3`, `RT+MT-7`, free text), as TRK ForceShot's weld tool does.
//!
//! - Click where the joint is: a point marker with its badge above. Click and drag: the point
//!   stays where the button went down and the drag places the badge (the leader follows).
//! - The joint number is asked for when a marker is placed, suggesting the next one; ticking
//!   "Continue with running numbers from here" numbers the rest of the document by itself.
//!   Cancel removes the marker just placed.
//! - The method is chosen with coloured buttons; Ctrl+click adds more (the badge is split into
//!   one colour band per method). The choice never changes a marker already placed; a marker's
//!   own methods are changed by double-clicking it.
//! - Free text ("Text") is typed straight into the badge and gets no number unless asked for.
//! - A selected marker has handles: its point (Shift drags out a seam), the seam's ends and
//!   the badge.

use std::collections::HashMap;

use egui::{Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Stroke, pos2, vec2};
use pdfcraft_engine::{Document, Edit, NewAnnotation, Shape, Style, Weld, WeldMethod, WeldShape};
use pdfcraft_render::DocInfo;

use crate::canvas::{DocView, PageXform};
use crate::theme::Tokens;

/// How joints are numbered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Numbering {
    /// Each method its own series: RT-1, RT-2, UT-1 …
    #[default]
    PerMethod,
    /// One series for all: 1, 2, 3 …
    Shared,
}

/// The weld tool's settings (kept between sessions).
#[derive(Clone, Debug, PartialEq)]
pub struct WeldPrefs {
    /// The methods the next marker gets.
    pub methods: Vec<WeldMethod>,
    pub shape: WeldShape,
    pub scale: f64,
    pub point_scale: f64,
    pub show_method: bool,
    /// Free-text markers get a running number too.
    pub text_number: bool,
    pub numbering: Numbering,
    pub start: u32,
}

impl Default for WeldPrefs {
    fn default() -> Self {
        WeldPrefs {
            methods: vec![WeldMethod::Rt],
            shape: WeldShape::Pill,
            scale: 1.0,
            point_scale: 1.0,
            show_method: true,
            text_number: false,
            numbering: Numbering::PerMethod,
            start: 1,
        }
    }
}

impl WeldPrefs {
    pub fn primary(&self) -> WeldMethod {
        self.methods.first().copied().unwrap_or(WeldMethod::Rt)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "methods": self.methods.iter().map(|m| m.key()).collect::<Vec<_>>(),
            "shape": if self.shape == WeldShape::Rect { "rect" } else { "pill" },
            "scale": self.scale,
            "point_scale": self.point_scale,
            "show_method": self.show_method,
            "text_number": self.text_number,
            "numbering": if self.numbering == Numbering::Shared { "shared" } else { "per-method" },
            "start": self.start,
        })
    }

    pub fn from_json(v: &serde_json::Value) -> WeldPrefs {
        let d = WeldPrefs::default();
        let methods: Vec<WeldMethod> =
            v["methods"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().and_then(WeldMethod::from_key)).take(8).collect()).unwrap_or_default();
        let num = |k: &str, lo: f64, hi: f64, def: f64| v[k].as_f64().filter(|x| x.is_finite()).map_or(def, |x| x.clamp(lo, hi));
        WeldPrefs {
            methods: if methods.is_empty() { d.methods } else { methods },
            shape: if v["shape"].as_str() == Some("rect") { WeldShape::Rect } else { WeldShape::Pill },
            scale: num("scale", 0.6, 2.4, 1.0),
            point_scale: num("point_scale", 0.4, 3.0, 1.0),
            show_method: v["show_method"].as_bool().unwrap_or(true),
            text_number: v["text_number"].as_bool().unwrap_or(false),
            numbering: if v["numbering"].as_str() == Some("shared") { Numbering::Shared } else { Numbering::PerMethod },
            start: v["start"].as_u64().map_or(1, |s| s.min(99_999) as u32),
        }
    }
}

/// A document's joint numbering: whether it runs by itself, and the next numbers set or taken
/// so far (otherwise the next after the highest already in the document).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeldNumbers {
    pub running: bool,
    /// Per method, or (`None`) the shared series.
    next: HashMap<Option<WeldMethod>, u32>,
}

fn series(prefs: &WeldPrefs, m: WeldMethod) -> Option<WeldMethod> {
    (prefs.numbering == Numbering::PerMethod).then_some(m)
}

/// The highest number already on a marker of `m`'s series in the document.
fn highest(doc: &Document, prefs: &WeldPrefs, m: WeldMethod) -> Option<u32> {
    doc.info
        .annotations
        .iter()
        .filter(|a| a.weld.is_some())
        .filter_map(|a| doc.weld(a.page, a.index))
        .filter(|w| w.show_number && (prefs.numbering == Numbering::Shared || w.methods.contains(&m)))
        .filter_map(|w| w.label.trim().parse::<u32>().ok())
        .max()
}

impl WeldNumbers {
    /// The number the next joint tested with `m` gets.
    pub fn peek(&self, doc: &Document, prefs: &WeldPrefs, m: WeldMethod) -> u32 {
        match self.next.get(&series(prefs, m)) {
            Some(n) => *n,
            None => highest(doc, prefs, m).map_or(prefs.start, |n| n.saturating_add(1)),
        }
    }

    pub fn set_next(&mut self, prefs: &WeldPrefs, m: WeldMethod, n: u32) {
        self.next.insert(series(prefs, m), n);
    }

    /// `w` was placed with the next number: every series it belongs to moves on (a joint tested
    /// with RT and MT counts in both). Markers without a number don't count.
    fn take(&mut self, doc: &Document, prefs: &WeldPrefs, w: &Weld) {
        if !w.show_number {
            return;
        }
        if prefs.numbering == Numbering::Shared {
            let n = self.peek(doc, prefs, w.primary());
            self.set_next(prefs, w.primary(), n.saturating_add(1));
            return;
        }
        let mut seen = Vec::new();
        for m in &w.methods {
            if !seen.contains(m) {
                seen.push(*m);
                let n = self.peek(doc, prefs, *m);
                self.set_next(prefs, *m, n.saturating_add(1));
            }
        }
    }
}

/// A marker on its way onto the page.
#[derive(Clone, Debug, PartialEq)]
pub enum WeldStage {
    /// The button is down: the point is set, a drag places the badge.
    Placing { page: usize, weld: Weld, press: Pos2 },
    /// Asking for the joint number.
    Number { page: usize, weld: Weld, input: String, running: bool, focus: bool },
    /// Typing the free text into the badge.
    Text { page: usize, weld: Weld, input: String, focus: bool },
}

/// Which handle of a selected marker is dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Point,
    End,
    Badge,
}

/// A handle being dragged on the selected marker.
#[derive(Clone, Debug, PartialEq)]
pub struct HandleDrag {
    page: usize,
    index: usize,
    handle: Handle,
    /// Where the button went down (screen): less than a few pixels from it is a click.
    start: Pos2,
    from: Weld,
    now: Weld,
}

/// Double-click on a marker: change its methods, number and text.
#[derive(Clone, Debug, PartialEq)]
pub struct EditDialog {
    page: usize,
    index: usize,
    weld: Weld,
    focus: bool,
}

fn rgb(c: [f64; 3]) -> Color32 {
    Color32::from_rgb((c[0].clamp(0.0, 1.0) * 255.0) as u8, (c[1].clamp(0.0, 1.0) * 255.0) as u8, (c[2].clamp(0.0, 1.0) * 255.0) as u8)
}

pub(crate) fn method_color(m: WeldMethod) -> Color32 {
    rgb(m.color())
}

/// User space → screen.
fn u2s(xf: &PageXform, info: &DocInfo, page: usize, p: [f64; 2]) -> Pos2 {
    match info.pages.get(page) {
        Some(pi) => {
            let v = pi.user_to_view(p[0] as f32, p[1] as f32);
            xf.norm_to_screen(v[0] / xf.pw.max(1e-3), v[1] / xf.ph.max(1e-3))
        }
        None => Pos2::ZERO,
    }
}

/// Screen → user space.
fn s2u(xf: &PageXform, info: &DocInfo, page: usize, p: Pos2) -> [f64; 2] {
    let (vx, vy) = xf.screen_to_view(p);
    match info.pages.get(page) {
        Some(pi) => {
            let u = pi.view_to_user(vx, vy);
            [f64::from(u[0]), f64::from(u[1])]
        }
        None => [0.0; 2],
    }
}

/// Screen pixels per point.
fn zoom(xf: &PageXform) -> f32 {
    ((xf.rect.width() * xf.rect.height()) / (xf.pw * xf.ph).max(1e-3)).sqrt()
}

/// The badge's box on screen (upright).
fn badge_screen(w: &Weld, center: Pos2, z: f32) -> Rect {
    let l = w.layout();
    Rect::from_center_size(center, vec2(l.width as f32 * z, l.height as f32 * z))
}

/// Draw `w` as the page will show it (upright on screen), mapped by `map`, `z` pixels per point.
pub(crate) fn paint_weld(painter: &egui::Painter, w: &Weld, map: &dyn Fn([f64; 2]) -> Pos2, z: f32) {
    let col = method_color(w.primary());
    let k = w.scale.clamp(0.3, 6.0) as f32;
    let ms = k * w.point_scale.clamp(0.2, 6.0) as f32;
    let sw = (1.5 * ms).max(0.8) * z;
    let halo = Stroke::new(sw + 1.3 * ms * z, Color32::from_black_alpha(64));
    let (p, e) = (map(w.point), map(w.end));
    if w.has_seam() {
        let d = (e - p).normalized();
        let perp = vec2(-d.y, d.x) * ((1.5 * ms).max(0.8) * 3.2).max(4.2) * z / 2.0;
        for s in [halo, Stroke::new(sw, col)] {
            painter.line_segment([p, e], s);
            painter.line_segment([p - perp, p + perp], s);
            painter.line_segment([e - perp, e + perp], s);
        }
    } else {
        let r = (2.7 * ms).max(2.0) * z;
        painter.circle_stroke(p, r, halo);
        painter.circle(p, r, col.gamma_multiply(0.38), Stroke::new(sw, col));
    }
    let center = map(w.badge);
    let b = badge_screen(w, center, z);
    let anchor = map(w.anchor());
    // The leader, from the badge's edge.
    let v = anchor - b.center();
    if v.length() > 0.5 {
        let s = (b.width() / 2.0 / v.x.abs().max(1e-6)).min(b.height() / 2.0 / v.y.abs().max(1e-6));
        let edge = b.center() + v * s.min(1.0);
        if edge.distance(anchor) > 2.0 {
            let lw = (1.0 * k).max(0.9) * z;
            painter.line_segment([edge, anchor], Stroke::new(lw + 1.2 * k * z, Color32::from_black_alpha(58)));
            painter.line_segment([edge, anchor], Stroke::new(lw, col));
        }
    }
    let l = w.layout();
    let radius = (l.radius as f32 * z).min(b.height() / 2.0).min(255.0) as u8;
    for (i, a) in [(0.66, 38u8), (1.5, 18u8)] {
        painter.rect_filled(b.translate(vec2(0.0, i * k * z)), CornerRadius::same(radius), Color32::from_black_alpha(a));
    }
    let painted = w.painted();
    let band = b.width() / painted.len() as f32;
    for (i, m) in painted.iter().enumerate() {
        let r =
            Rect::from_min_size(pos2(b.left() + i as f32 * band, b.top()), vec2(band + if i + 1 < painted.len() { 0.5 } else { 0.0 }, b.height()));
        let (first, last) = (i == 0, i + 1 == painted.len());
        let cr = CornerRadius {
            nw: if first { radius } else { 0 },
            sw: if first { radius } else { 0 },
            ne: if last { radius } else { 0 },
            se: if last { radius } else { 0 },
        };
        painter.rect_filled(r, cr, method_color(*m));
    }
    painter.rect_stroke(b, CornerRadius::same(radius), Stroke::new((0.7 * k).max(0.6) * z, Color32::from_white_alpha(230)), egui::StrokeKind::Middle);
    if let Some(y) = l.divider {
        let y = b.center().y - y as f32 * z;
        let inset = 3.0 * k * z;
        painter.hline(b.left() + inset..=b.right() - inset, y, Stroke::new((0.6 * k).max(0.55) * z, Color32::from_white_alpha(140)));
    }
    for line in &l.lines {
        let font = FontId::new((line.size as f32 * z).max(4.0), FontFamily::Name("semibold".into()));
        let galley = painter.layout_no_wrap(line.text.clone(), font, Color32::WHITE);
        // The capitals' middle (0.36 em above the baseline) where the layout puts it.
        let mid = b.center().y - (line.y as f32 + 0.36 * line.size as f32) * z;
        painter.galley(pos2(b.center().x - galley.size().x / 2.0, mid - galley.size().y / 2.0), galley, Color32::WHITE);
    }
}

/// The selected comment, if it is a weld marker on `page`: its index.
fn selected_weld(view: &DocView, info: &DocInfo, page: usize) -> Option<usize> {
    let (p, i) = view.comments.selected?;
    (p == page && info.annotations.iter().any(|a| a.page == page && a.index == i && a.weld.is_some())).then_some(i)
}

fn handles(w: &Weld) -> Vec<(Handle, [f64; 2])> {
    if w.has_seam() {
        vec![(Handle::Point, w.point), (Handle::End, w.end), (Handle::Badge, w.badge)]
    } else {
        vec![(Handle::Point, w.point), (Handle::Badge, w.badge)]
    }
}

/// The marker to add for `w` on `page`.
fn add_edit(page: usize, w: Weld, author: &str) -> Edit {
    let contents = w.badge_text();
    let shape = Shape::Weld(w);
    Edit::AddAnnotation(NewAnnotation { page, style: Style::default_for(&shape), shape, contents, author: author.to_string() })
}

/// A marker just placed (the button let go): it gets the tool's methods and look, then its
/// number (asked for, unless numbering runs) or its text.
fn placed(view: &mut DocView, doc: &Document, prefs: &WeldPrefs, author: &str, page: usize, mut w: Weld) {
    w.methods = prefs.methods.clone();
    // Free text usually says it all, so it gets a number only when asked for.
    w.show_number = !w.is_text() || prefs.text_number;
    w.label = view.weld_numbers.peek(doc, prefs, w.primary()).to_string();
    if w.is_text() {
        view.weld_stage = Some(WeldStage::Text { page, weld: w, input: String::new(), focus: true });
    } else if view.weld_numbers.running || !w.show_number {
        view.weld_numbers.take(doc, prefs, &w);
        view.pending_edit = Some(add_edit(page, w, author));
    } else {
        let input = w.label.clone();
        view.weld_stage = Some(WeldStage::Number { page, weld: w, input, running: false, focus: true });
    }
}

/// Pointer input for weld markers on one page: placing new ones (`tool`), the selected one's
/// handles and double-click to edit. `true` when it used the input.
#[allow(clippy::too_many_arguments)]
pub(crate) fn page_input(
    ui: &egui::Ui,
    resp: &egui::Response,
    xf: &PageXform,
    page: usize,
    info: &DocInfo,
    doc: &Document,
    prefs: &WeldPrefs,
    tool: bool,
    author: &str,
    view: &mut DocView,
) -> bool {
    let (pointer, pressed, down) = ui.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
    let shift = ui.input(|i| i.modifiers.shift);
    // A handle on the move.
    if let Some(mut d) = view.weld_drag.clone().filter(|d| d.page == page) {
        if let Some(p) = pointer.filter(|p| p.distance(d.start) >= 3.0) {
            let u = s2u(xf, info, page, p);
            let mut w = d.from.clone();
            match d.handle {
                Handle::Badge => w.badge = u,
                // A point: moves (its badge stays); with Shift a seam is drawn out of it.
                Handle::Point if !d.from.has_seam() && shift => w.end = u,
                Handle::Point if !d.from.has_seam() => {
                    w.point = u;
                    w.end = u;
                }
                Handle::Point => w.point = u,
                Handle::End => w.end = u,
            }
            d.now = w;
        }
        if !down {
            view.weld_drag = None;
            if d.now != d.from {
                view.pending_edit = Some(Edit::SetWeld { page, index: d.index, weld: d.now });
            } else {
                // A second click on a handle (the badge, mostly) soon after the first: edit it.
                let now = ui.input(|i| i.time);
                let p = pointer.unwrap_or_default();
                if resp.double_clicked() || view.weld_tap.is_some_and(|(t, q)| now - t <= 0.6 && q.distance(p) < 6.0) {
                    view.weld_tap = None;
                    view.weld_edit = Some(EditDialog { page, index: d.index, weld: d.from, focus: true });
                } else {
                    view.weld_tap = Some((now, p));
                }
            }
        } else {
            view.weld_drag = Some(d);
        }
        return true;
    }
    match view.weld_stage.clone() {
        Some(WeldStage::Placing { page: sp, mut weld, press }) if sp == page => {
            if let Some(p) = pointer {
                // The point stays where the button went down; a drag places the badge.
                if p.distance(press) >= 6.0 {
                    weld.badge = s2u(xf, info, page, p);
                }
            }
            if down {
                view.weld_stage = Some(WeldStage::Placing { page, weld, press });
            } else {
                view.weld_stage = None;
                placed(view, doc, prefs, author, page, weld);
            }
            return true;
        }
        // Waiting for the number or the text: the page waits too.
        Some(WeldStage::Number { .. } | WeldStage::Text { .. }) => return tool,
        _ => {}
    }
    let Some(p) = pointer.filter(|p| xf.rect.contains(*p) && resp.contains_pointer()) else { return false };
    // The selected marker's handles.
    if let Some(index) = selected_weld(view, info, page)
        && let Some(w) = doc.weld(page, index)
        && let Some((h, _)) = handles(&w).into_iter().find(|(_, at)| u2s(xf, info, page, *at).distance(p) <= 8.0)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if pressed {
            view.weld_drag = Some(HandleDrag { page, index, handle: h, start: p, from: w.clone(), now: w });
            return true;
        }
        if !tool {
            return true;
        }
    }
    // Double-click a marker: change its method, number and text.
    if resp.double_clicked()
        && let Some(a) =
            info.annotations.iter().rev().filter(|a| a.page == page && a.weld.is_some()).find(|a| xf.user_rect(info, page, a.rect).contains(p))
        && let Some(w) = doc.weld(page, a.index)
    {
        view.weld_stage = None;
        view.weld_edit = Some(EditDialog { page, index: a.index, weld: w, focus: true });
        return true;
    }
    if !tool {
        return false;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if pressed {
        let at = s2u(xf, info, page, p);
        // The badge above the point (as the page is shown).
        let z = zoom(xf);
        let badge = s2u(xf, info, page, p - vec2(0.0, 28.0 * prefs.scale as f32 * z));
        let weld = Weld {
            point: at,
            end: at,
            badge,
            methods: prefs.methods.clone(),
            label: view.weld_numbers.peek(doc, prefs, prefs.primary()).to_string(),
            show_number: !prefs.methods.contains(&WeldMethod::Text) || prefs.text_number,
            show_method: prefs.show_method,
            shape: prefs.shape,
            scale: prefs.scale,
            point_scale: prefs.point_scale,
            ..Weld::default()
        };
        view.comments.selected = None;
        view.weld_stage = Some(WeldStage::Placing { page, weld, press: p });
    }
    true
}

/// The marker being placed or dragged, and the selected one's handles.
pub(crate) fn paint_page(painter: &egui::Painter, xf: &PageXform, page: usize, info: &DocInfo, doc: &Document, view: &DocView) {
    let map = |p: [f64; 2]| u2s(xf, info, page, p);
    let z = zoom(xf);
    match &view.weld_stage {
        Some(WeldStage::Placing { page: p, weld, .. } | WeldStage::Number { page: p, weld, .. }) if *p == page => paint_weld(painter, weld, &map, z),
        Some(WeldStage::Text { page: p, weld, input, .. }) if *p == page => {
            // The leader and point; the badge is the text field.
            let shown = Weld { text: if input.is_empty() { " ".into() } else { input.clone() }, ..weld.clone() };
            paint_weld(painter, &shown, &map, z);
        }
        _ => {}
    }
    let dragging = view.weld_drag.as_ref().filter(|d| d.page == page);
    if let Some(d) = dragging {
        // Where it goes: the old one dimmed under it.
        let old = xf.user_rect(info, page, d.from.bounds().map(|v| v as f32));
        painter.rect_filled(old, CornerRadius::ZERO, Color32::from_white_alpha(150));
        paint_weld(painter, &d.now, &map, z);
    }
    let shown = match dragging {
        Some(d) => Some(d.now.clone()),
        None => selected_weld(view, info, page).and_then(|i| doc.weld(page, i)),
    };
    if let Some(w) = shown {
        for (_, at) in handles(&w) {
            painter.circle(map(at), 4.5, Color32::WHITE, Stroke::new(1.5, Color32::from_rgb(0x14, 0x73, 0xE6)));
        }
    }
}

/// The number prompt, the free-text field and the edit dialog. Returns an edit when one of them
/// finishes a marker.
pub(crate) fn overlays(ctx: &egui::Context, view: &mut DocView, info: &DocInfo, doc: &Document, prefs: &WeldPrefs, author: &str) -> Option<Edit> {
    let t = Tokens::get(ctx);
    let mut out = None;
    match view.weld_stage.clone() {
        Some(WeldStage::Number { page, weld, mut input, mut running, focus }) => {
            let xf = view.page_xform(page)?;
            let z = zoom(&xf);
            let b = badge_screen(&weld, u2s(&xf, info, page, weld.badge), z);
            let (mut done, mut cancel) = (false, false);
            egui::Area::new(egui::Id::new(("weld-number", view.id.0))).order(egui::Order::Foreground).fixed_pos(b.right_top() + vec2(10.0, 0.0)).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(250.0);
                    ui.label(egui::RichText::new(tl!("Joint number")).font(crate::theme::semibold(12.0)).color(t.text_muted));
                    let edit = ui.add(egui::TextEdit::singleline(&mut input).char_limit(24).desired_width(f32::INFINITY).id_salt("weld-number-input"));
                    if focus {
                        edit.request_focus();
                        let mut state = egui::TextEdit::load_state(ui.ctx(), edit.id).unwrap_or_default();
                        let end = egui::text::CCursor::new(input.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                        state.store(ui.ctx(), edit.id);
                    }
                    ui.add_space(6.0);
                    ui.checkbox(&mut running, tl!("Continue with running numbers from here"))
                        .on_hover_text(tl!("The next joint gets the next number by itself, without this box"));
                    ui.label(
                        egui::RichText::new(tl!("Without running numbers you are asked for the number each time you place a joint. The choice applies to this document."))
                            .small()
                            .color(t.text_faint),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            done |= ui.button(tl!("Done")).clicked();
                            cancel |= ui.button(tl!("Cancel")).clicked();
                        });
                    });
                    done |= edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
                });
            });
            if cancel {
                // Cancelled: the marker just placed goes.
                view.weld_stage = None;
            } else if done {
                view.weld_stage = None;
                let mut w = weld;
                let typed = input.trim();
                if !typed.is_empty() {
                    w.label = typed.to_string();
                }
                // The next suggestion is the number after the one typed, whatever was chosen.
                if let Ok(n) = w.label.parse::<u32>() {
                    view.weld_numbers.set_next(prefs, w.primary(), n.saturating_add(1));
                }
                if running {
                    view.weld_numbers.running = true;
                }
                out = Some(add_edit(page, w, author));
            } else {
                view.weld_stage = Some(WeldStage::Number { page, weld, input, running, focus: false });
            }
        }
        Some(WeldStage::Text { page, weld, mut input, focus }) => {
            let xf = view.page_xform(page)?;
            let z = zoom(&xf);
            let center = u2s(&xf, info, page, weld.badge);
            let size = (9.0 * weld.scale as f32 * z).clamp(11.0, 40.0);
            let width = 180.0_f32.max(size * 10.0);
            let (mut done, mut cancel) = (false, false);
            egui::Area::new(egui::Id::new(("weld-text", view.id.0)))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(center.x - width / 2.0, center.y - size * 0.9))
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(0x15, 0x18, 0x20, 0xE6))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(egui::Margin::same(4))
                        .show(ui, |ui| {
                            ui.visuals_mut().text_cursor.stroke.color = Color32::WHITE;
                            let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut input)
                                    .char_limit(40)
                                    .font(FontId::new(size, FontFamily::Name("semibold".into())))
                                    .text_color(Color32::WHITE)
                                    .frame(egui::Frame::NONE)
                                    .desired_width(width)
                                    .hint_text(tl!("Type text"))
                                    .id_salt("weld-text-input"),
                            );
                            if focus {
                                edit.request_focus();
                            }
                            cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            done = enter || (edit.lost_focus() && !cancel);
                        });
                });
            if cancel || (done && input.trim().is_empty()) {
                view.weld_stage = None;
            } else if done {
                view.weld_stage = None;
                let mut w = weld;
                w.text = input.trim().to_string();
                if w.show_number {
                    w.label = view.weld_numbers.peek(doc, prefs, w.primary()).to_string();
                    view.weld_numbers.take(doc, prefs, &w);
                }
                out = Some(add_edit(page, w, author));
            } else {
                view.weld_stage = Some(WeldStage::Text { page, weld, input, focus: false });
            }
        }
        _ => {}
    }
    if let Some(edit) = edit_dialog(ctx, view, &t) {
        out = Some(edit);
    }
    out
}

/// The coloured method buttons; Ctrl+click adds or removes one. Returns whether they changed.
fn method_buttons(ui: &mut egui::Ui, methods: &mut Vec<WeldMethod>) -> bool {
    let mut changed = false;
    ui.spacing_mut().item_spacing.x = 3.0;
    for m in WeldMethod::ALL {
        let on = methods.contains(&m);
        let label = match m {
            WeldMethod::None => "—".to_string(),
            WeldMethod::Text => tl!("Text").to_string(),
            _ => m.code().to_string(),
        };
        let col = method_color(m);
        let text = egui::RichText::new(label).font(crate::theme::semibold(12.0)).color(if on { Color32::WHITE } else { col });
        let button = egui::Button::new(text)
            .fill(if on { col } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.5, col))
            .corner_radius(CornerRadius::same(10))
            .min_size(vec2(34.0, 22.0));
        let tip = match m {
            WeldMethod::Rt => tl!("RT — Radiographic testing"),
            WeldMethod::Ut => tl!("UT — Ultrasonic testing"),
            WeldMethod::Paut => tl!("PAUT — Phased array ultrasonic testing"),
            WeldMethod::Mt => tl!("MT — Magnetic particle testing"),
            WeldMethod::Pt => tl!("PT — Penetrant testing"),
            WeldMethod::Vt => tl!("VT — Visual testing"),
            WeldMethod::None => tl!("No method (number only)"),
            WeldMethod::Text => tl!("Free text instead of a method code"),
        };
        let resp = ui.add(button).on_hover_text(tip);
        if resp.clicked() {
            let add = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
            pdfcraft_engine::toggle_weld_method(methods, m, add);
            changed = true;
        }
    }
    changed
}

/// Double-click on a marker: its methods, free text, number and whether the method shows.
fn edit_dialog(ctx: &egui::Context, view: &mut DocView, t: &Tokens) -> Option<Edit> {
    let mut d = view.weld_edit.clone()?;
    let (mut save, mut cancel) = (false, false);
    let mut open = true;
    egui::Window::new(tl!("Weld joint"))
        .id(egui::Id::new(("weld-edit", view.id.0)))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(egui::RichText::new(tl!("Test method")).font(crate::theme::semibold(12.0)).color(t.text_muted));
            ui.horizontal(|ui| {
                method_buttons(ui, &mut d.weld.methods);
            });
            ui.label(
                egui::RichText::new(tl!(
                    "Click more if the joint is tested with several methods. The badge is then split into one colour band per method."
                ))
                .small()
                .color(t.text_faint),
            );
            if d.weld.is_text() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(tl!("Own text")).font(crate::theme::semibold(12.0)).color(t.text_muted));
                ui.add(egui::TextEdit::singleline(&mut d.weld.text).char_limit(40).desired_width(f32::INFINITY));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!("Joint number")).font(crate::theme::semibold(12.0)).color(t.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut d.weld.show_number, tl!("Show"));
                });
            });
            let edit = ui.add(egui::TextEdit::singleline(&mut d.weld.label).char_limit(24).desired_width(f32::INFINITY));
            if d.focus {
                edit.request_focus();
                d.focus = false;
            }
            ui.checkbox(&mut d.weld.show_method, tl!("Show the method on the marker"));
            ui.add_space(8.0);
            // The preview is drawn by the same code as on the page.
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), egui::Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(8), t.chrome);
            let mut sample = d.weld.clone();
            sample.scale = sample.scale.clamp(0.8, 1.4);
            let z = 1.8_f32;
            let origin = r.center() + vec2(0.0, 26.0);
            sample.point = [0.0, 0.0];
            sample.end = [0.0, 0.0];
            sample.badge = [0.0, 28.0];
            let map = |p: [f64; 2]| origin + vec2(p[0] as f32 * z, -(p[1] as f32) * z);
            paint_weld(&ui.painter().with_clip_rect(r), &sample, &map, z);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.button(tl!("Save")).clicked();
                    cancel = ui.button(tl!("Cancel")).clicked();
                });
            });
            save |= ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
            cancel |= ui.input(|i| i.key_pressed(egui::Key::Escape));
        });
    if cancel || !open {
        view.weld_edit = None;
        return None;
    }
    if save {
        view.weld_edit = None;
        let mut w = d.weld;
        w.label = w.label.trim().to_string();
        if w.label.is_empty() {
            w.label = "1".into();
        }
        w.text = w.text.trim().to_string();
        return Some(Edit::SetWeld { page: d.page, index: d.index, weld: w });
    }
    view.weld_edit = Some(d);
    None
}

/// The drawing tools the weld bar offers, drawn in the chosen method's colour.
pub(crate) const DRAW_TOOLS: [crate::comments::CommentTool; 5] = {
    use crate::comments::CommentTool as T;
    [T::TextBox, T::Arrow, T::Line, T::Rectangle, T::Oval]
};

/// Give `tool` the weld look: the method's colour, the leader's weight; a text box gets a white
/// box with a frame in that colour.
fn theme_tool(prefs: &mut crate::comments::CommentPrefs, tool: crate::comments::CommentTool, m: WeldMethod) {
    use crate::comments::CommentTool as T;
    prefs.set_color(tool, m.color());
    prefs.set_opacity(tool, 1.0);
    match tool {
        T::TextBox => {
            prefs.set_width(tool, 1.5);
            prefs.set_fill(tool, Some([1.0, 1.0, 1.0]));
        }
        T::Rectangle | T::Oval => {
            prefs.set_width(tool, 2.0);
            prefs.set_fill(tool, None);
        }
        _ => prefs.set_width(tool, 2.0),
    }
}

/// The weld tool's row of options, along the top of the document while the tool is on or a
/// marker is selected: method, numbering, shape and sizes. Shape, sizes and "show method" also
/// change the selected marker; the method never changes a marker already placed.
pub(crate) fn options_bar(app: &mut crate::PdfCraftApp, area: Rect, ui: &mut egui::Ui) {
    let Some((vi, id)) = app.active_ids() else { return };
    let tool = app.quick_tool == crate::QuickTool::Weld;
    // One of the bar's drawing tools, picked from it: the bar stays.
    let drawing = match app.quick_tool {
        crate::QuickTool::Comment(t) if app.weld_mode && DRAW_TOOLS.contains(&t) => Some(t),
        _ => None,
    };
    // Leaving the weld tools (another tool) ends the weld mode.
    if !tool && drawing.is_none() && !matches!(app.quick_tool, crate::QuickTool::Select) {
        app.weld_mode = false;
    }
    let tool = tool || drawing.is_some();
    let Some(doc) = app.session.get(id) else { return };
    let selected =
        app.views[vi].comments.selected.filter(|(p, i)| doc.info.annotations.iter().any(|a| a.page == *p && a.index == *i && a.weld.is_some()));
    if !tool && selected.is_none() {
        return;
    }
    let selected_weld = selected.and_then(|(p, i)| doc.weld(p, i).map(|w| (p, i, w)));
    let t = Tokens::get(ui.ctx());
    let mut prefs = app.weld_prefs.clone();
    let primary_before = prefs.primary();
    let mut numbers = app.views[vi].weld_numbers.clone();
    let mut next = numbers.peek(doc, &prefs, prefs.primary()).to_string();
    let next_before = next.clone();
    let mut restyle: Option<Weld> = None;
    let mut picked: Option<crate::comments::CommentTool> = None;
    let pos = area.left_top() + vec2(64.0, 14.0);
    egui::Area::new(egui::Id::new("weld-options")).order(egui::Order::Middle).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.card)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(if t.dark() { 80 } else { 22 }) })
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_max_width((area.width() - 90.0).max(300.0));
                ui.horizontal_wrapped(|ui| {
                    let label = |ui: &mut egui::Ui, s: &str| {
                        ui.label(egui::RichText::new(s).font(crate::theme::semibold(12.0)).color(t.text_muted));
                    };
                    label(ui, tl!("Method"));
                    method_buttons(ui, &mut prefs.methods);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.separator();
                    // Text box, arrow, line, rectangle and oval in the method's colour.
                    label(ui, tl!("Draw"));
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for t in DRAW_TOOLS {
                        if crate::icons::button(ui, t.icon(), 26.0, drawing == Some(t), tl!(t.label())).clicked() {
                            picked = Some(t);
                        }
                    }
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if prefs.methods.contains(&WeldMethod::Text) {
                        ui.checkbox(&mut prefs.text_number, tl!("With number")).on_hover_text(tl!("Give the free text a running number too"));
                    }
                    ui.separator();
                    label(ui, tl!("Next number"));
                    ui.add(egui::TextEdit::singleline(&mut next).desired_width(44.0).horizontal_align(egui::Align::Center))
                        .on_hover_text(tl!("The next running number; type another to continue from it"));
                    ui.checkbox(&mut numbers.running, tl!("Running numbers")).on_hover_text(tl!(
                        "On: the joints are numbered in order by themselves. Off: you are asked for each joint's number. Applies to this document."
                    ));
                    egui::ComboBox::from_id_salt("weld-numbering")
                        .selected_text(if prefs.numbering == Numbering::Shared { tl!("Shared series") } else { tl!("Series per method") })
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut prefs.numbering, Numbering::PerMethod, tl!("Series per method"));
                            ui.selectable_value(&mut prefs.numbering, Numbering::Shared, tl!("Shared series"));
                        });
                    ui.separator();
                    label(ui, tl!("Shape"));
                    let shape_before = prefs.shape;
                    egui::ComboBox::from_id_salt("weld-shape")
                        .selected_text(if prefs.shape == WeldShape::Rect { tl!("Rectangle") } else { tl!("Pill") })
                        .width(96.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut prefs.shape, WeldShape::Pill, tl!("Pill"));
                            ui.selectable_value(&mut prefs.shape, WeldShape::Rect, tl!("Rectangle"));
                        });
                    label(ui, tl!("Badge"));
                    let mut s = (prefs.scale * 100.0).round();
                    let badge = ui
                        .add(egui::Slider::new(&mut s, 60.0..=240.0).show_value(false))
                        .on_hover_text(tl!("Size of the badge with method and number"));
                    prefs.scale = s / 100.0;
                    label(ui, tl!("Point"));
                    let mut ps = (prefs.point_scale * 100.0).round();
                    let point = ui
                        .add(egui::Slider::new(&mut ps, 40.0..=300.0).show_value(false))
                        .on_hover_text(tl!("Size of the point or line that shows where the joint is"));
                    prefs.point_scale = ps / 100.0;
                    let show_before = prefs.show_method;
                    ui.checkbox(&mut prefs.show_method, tl!("Show method"));
                    ui.label(
                        egui::RichText::new(tl!("Ctrl+click adds a method · Shift-drag the point's handle for a seam")).small().color(t.text_faint),
                    );
                    // The look changes the selected marker too (sliders once they are let go).
                    if let Some((_, _, w)) = &selected_weld {
                        let mut nw = w.clone();
                        if prefs.shape != shape_before {
                            nw.shape = prefs.shape;
                        }
                        if prefs.show_method != show_before {
                            nw.show_method = prefs.show_method;
                        }
                        if badge.drag_stopped() || (badge.changed() && !badge.dragged()) {
                            nw.scale = prefs.scale;
                        }
                        if point.drag_stopped() || (point.changed() && !point.dragged()) {
                            nw.point_scale = prefs.point_scale;
                        }
                        if nw != *w {
                            restyle = Some(nw);
                        }
                    }
                });
            });
    });
    // A new method starts its own count again: the next joint asks for its number.
    if prefs.primary() != primary_before && prefs.numbering == Numbering::PerMethod {
        numbers.running = false;
    } else if next != next_before
        && let Ok(n) = next.trim().parse::<u32>()
    {
        numbers.set_next(&prefs, prefs.primary(), n);
    }
    app.views[vi].weld_numbers = numbers;
    // The drawing tools follow the method (a new method recolours the one in use).
    let method = prefs.primary();
    if let Some(t) = picked {
        theme_tool(&mut app.comment_prefs, t, method);
        app.execute(t.command());
        app.weld_mode = true;
    } else if let Some(t) = drawing
        && method != primary_before
    {
        theme_tool(&mut app.comment_prefs, t, method);
    }
    app.weld_prefs = prefs;
    if let (Some(w), Some((p, i, _))) = (restyle, selected_weld) {
        app.apply_edit(Edit::SetWeld { page: p, index: i, weld: w });
    }
}
