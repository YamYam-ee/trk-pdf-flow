//! Weld joint markers for NDT documentation (TRK): a point (or a seam with end ticks) where the
//! joint is, a leader line, and a colour-coded badge with the test method(s) and the joint
//! number, e.g. `RT-3` or `RT+MT-7`.
//!
//! The rules follow TRK ForceShot's weld tool, which was refined over several rounds:
//! - a joint can be tested with several methods; the badge is then split into one colour band
//!   per method and the codes are joined (`RT+MT-7`);
//! - "no method" (number only) and free text exclude every other method;
//! - free text carries its own message, so it gets no number unless asked for;
//! - the badge is a pill, or a rectangle split by a line with the number above and the method
//!   below (an undivided box when only one of them is shown);
//! - no heavy frame: a soft shadow lifts the badge off light drawings and a hairline light edge
//!   separates it from dark ones.
//!
//! The marker is a `/Stamp` named `/PCWeld` whose data lives in `/PCWeld`, so it can be edited
//! again; its appearance is drawn here from that data (other viewers show the appearance).

use pdfcraft_cos::{Dict, Object, PdfString, Stream};
use pdfcraft_fonts::{helvetica_width, literal, win_ansi};

use crate::{Rgb, n};

/// Non-destructive testing methods (SS-EN ISO 9712), plus "no method" and free text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeldMethod {
    Rt,
    Ut,
    Paut,
    Mt,
    Pt,
    Vt,
    /// No method: only the joint number.
    None,
    /// Own text instead of a method code.
    Text,
}

impl WeldMethod {
    /// In the order the tool bar shows them.
    pub const ALL: [WeldMethod; 8] =
        [WeldMethod::Rt, WeldMethod::Ut, WeldMethod::Paut, WeldMethod::Mt, WeldMethod::Pt, WeldMethod::Vt, WeldMethod::None, WeldMethod::Text];

    /// The code on the badge (empty for "no method" and free text).
    pub fn code(self) -> &'static str {
        match self {
            WeldMethod::Rt => "RT",
            WeldMethod::Ut => "UT",
            WeldMethod::Paut => "PAUT",
            WeldMethod::Mt => "MT",
            WeldMethod::Pt => "PT",
            WeldMethod::Vt => "VT",
            WeldMethod::None | WeldMethod::Text => "",
        }
    }

    /// The name stored in the file.
    pub fn key(self) -> &'static str {
        match self {
            WeldMethod::None => "None",
            WeldMethod::Text => "Text",
            m => m.code(),
        }
    }

    pub fn from_key(k: &str) -> Option<WeldMethod> {
        Self::ALL.into_iter().find(|m| m.key() == k)
    }

    /// The method's colour, chosen so that white text reads well, in print too.
    pub fn color(self) -> Rgb {
        let c = |r: u8, g: u8, b: u8| [f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0];
        match self {
            // A muted wine red rather than signal red: radiography without shouting.
            WeldMethod::Rt => c(0x8E, 0x44, 0x57),
            WeldMethod::Ut => c(0x15, 0x65, 0xC0),
            WeldMethod::Paut => c(0x00, 0x83, 0x8F),
            WeldMethod::Mt => c(0x7B, 0x1F, 0xA2),
            WeldMethod::Pt => c(0xE6, 0x51, 0x00),
            WeldMethod::Vt => c(0x2E, 0x7D, 0x32),
            WeldMethod::None => c(0x37, 0x47, 0x4F),
            // Brown: no other method is near it on the colour wheel.
            WeldMethod::Text => c(0x79, 0x55, 0x48),
        }
    }

    /// "No method" and free text can't be combined with anything else.
    pub fn exclusive(self) -> bool {
        matches!(self, WeldMethod::None | WeldMethod::Text)
    }
}

/// Choose `m` in `methods`: on its own, or (`add`, Ctrl+click) added to or removed from the others.
/// The last method is never removed; "no method" and free text stand alone.
pub fn toggle_method(methods: &mut Vec<WeldMethod>, m: WeldMethod, add: bool) {
    if !add || m.exclusive() {
        methods.clear();
        methods.push(m);
    } else if methods.contains(&m) {
        if methods.len() > 1 {
            methods.retain(|x| *x != m);
        }
    } else {
        methods.retain(|x| !x.exclusive());
        methods.push(m);
    }
}

/// The badge's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeldShape {
    /// A pill with the codes and number on one line.
    #[default]
    Pill,
    /// A rectangle split by a line: the number above, the method below.
    Rect,
}

/// A weld joint marker, in PDF user space.
#[derive(Clone, Debug, PartialEq)]
pub struct Weld {
    /// Where the joint is; a seam runs from `point` to `end` (equal for a point marker).
    pub point: [f64; 2],
    pub end: [f64; 2],
    /// The badge's centre.
    pub badge: [f64; 2],
    /// At least one.
    pub methods: Vec<WeldMethod>,
    /// The joint number (digits or e.g. `A12`).
    pub label: String,
    /// The free text, with [`WeldMethod::Text`].
    pub text: String,
    pub show_number: bool,
    pub show_method: bool,
    pub shape: WeldShape,
    /// The badge's size (1 = 9 pt text).
    pub scale: f64,
    /// The point's or seam's size, besides `scale`: the point has to point exactly, the badge
    /// has to be read, so they are set apart.
    pub point_scale: f64,
    /// The page turn the badge is drawn upright against (0, 90, 180, 270).
    pub turn: i64,
}

impl Default for Weld {
    fn default() -> Self {
        Weld {
            point: [0.0; 2],
            end: [0.0; 2],
            badge: [0.0, 28.0],
            methods: vec![WeldMethod::Rt],
            label: "1".into(),
            text: String::new(),
            show_number: true,
            show_method: true,
            shape: WeldShape::Pill,
            scale: 1.0,
            point_scale: 1.0,
            turn: 0,
        }
    }
}

/// One line of text on the badge, relative to its centre (upright, y up).
#[derive(Clone, Debug, PartialEq)]
pub struct BadgeLine {
    pub text: String,
    pub size: f64,
    /// Baseline, from the badge's centre.
    pub y: f64,
    pub width: f64,
}

/// The badge's size and text, laid out upright around its centre.
#[derive(Clone, Debug, PartialEq)]
pub struct BadgeLayout {
    pub width: f64,
    pub height: f64,
    pub radius: f64,
    pub lines: Vec<BadgeLine>,
    /// The dividing line of a split rectangle, from the centre.
    pub divider: Option<f64>,
}

/// Helvetica-Bold runs a little wider than the regular widths `helvetica_width` gives.
pub fn bold_width(s: &str, size: f64) -> f64 {
    helvetica_width(s, size) * 1.07
}

impl Weld {
    /// A seam rather than a point.
    pub fn has_seam(&self) -> bool {
        (self.end[0] - self.point[0]).hypot(self.end[1] - self.point[1]) > 4.0
    }

    /// Where the leader line ends: the point, or the seam's middle.
    pub fn anchor(&self) -> [f64; 2] {
        if self.has_seam() { [(self.point[0] + self.end[0]) / 2.0, (self.point[1] + self.end[1]) / 2.0] } else { self.point }
    }

    pub fn is_text(&self) -> bool {
        self.methods.contains(&WeldMethod::Text)
    }

    /// The method that colours the point, the seam and the leader.
    pub fn primary(&self) -> WeldMethod {
        self.methods.first().copied().unwrap_or(WeldMethod::None)
    }

    /// What stands in the method's place: the codes (`RT+MT`) or the free text; empty when the
    /// method is hidden.
    pub fn codes(&self) -> String {
        if !self.show_method {
            return String::new();
        }
        if self.is_text() {
            return self.text.trim().to_string();
        }
        let mut codes: Vec<&str> = Vec::new();
        for m in &self.methods {
            let c = m.code();
            if !c.is_empty() && !codes.contains(&c) {
                codes.push(c);
            }
        }
        codes.join("+")
    }

    /// The number as it stands on the badge, or empty.
    pub fn number(&self) -> String {
        if self.show_number { self.label.trim().to_string() } else { String::new() }
    }

    /// The badge's text on one line (`RT+MT-7`), as listed in the comments.
    pub fn badge_text(&self) -> String {
        let (codes, number) = (self.codes(), self.number());
        match (codes.is_empty(), number.is_empty()) {
            (false, false) => format!("{codes}-{number}"),
            (false, true) => codes,
            (true, false) => number,
            (true, true) => "?".into(),
        }
    }

    fn scale(&self) -> f64 {
        if self.scale.is_finite() { self.scale.clamp(0.3, 6.0) } else { 1.0 }
    }

    fn mark_scale(&self) -> f64 {
        self.scale() * if self.point_scale.is_finite() { self.point_scale.clamp(0.2, 6.0) } else { 1.0 }
    }

    /// The methods that get a colour band (at least one).
    pub fn painted(&self) -> Vec<WeldMethod> {
        let mut out: Vec<WeldMethod> = Vec::new();
        for m in &self.methods {
            if *m != WeldMethod::None && !out.contains(m) {
                out.push(*m);
            }
        }
        if out.is_empty() {
            out.push(WeldMethod::None);
        }
        out
    }

    pub fn layout(&self) -> BadgeLayout {
        let k = self.scale();
        let size = 9.0 * k;
        let (pad_x, pad_y) = (5.4 * k, 2.6 * k);
        let line_h = |s: f64| s * 1.2;
        let (codes, number) = (self.codes(), self.number());
        if self.shape == WeldShape::Rect && !codes.is_empty() && !number.is_empty() {
            // Two lines: the number above, the method under a line.
            let small = size * 0.78;
            let gap = 2.4 * k;
            let (nw, cw) = (bold_width(&number, size), bold_width(&codes, small));
            let width = nw.max(cw) + 2.0 * pad_x;
            let height = pad_y + line_h(size) + gap + line_h(small) + pad_y;
            let top = height / 2.0;
            let divider = top - pad_y - line_h(size) - gap / 2.0;
            return BadgeLayout {
                width,
                height,
                radius: 1.8 * k,
                lines: vec![
                    BadgeLine { text: number, size, y: top - pad_y - size * 0.95, width: nw },
                    BadgeLine { text: codes, size: small, y: divider - gap / 2.0 - small * 0.95, width: cw },
                ],
                divider: Some(divider),
            };
        }
        let text = self.badge_text();
        let w = bold_width(&text, size);
        let width = w + 2.0 * pad_x;
        let height = line_h(size) + 2.0 * pad_y;
        let radius = if self.shape == WeldShape::Pill { height / 2.0 } else { 1.8 * k };
        // Capitals and digits sit centred: their middle is about 0.36 em above the baseline.
        BadgeLayout { width, height, radius, lines: vec![BadgeLine { text, size, y: -size * 0.36, width: w }], divider: None }
    }

    /// The badge's box as the page is shown (turned with the page), in user space.
    pub fn badge_rect(&self) -> [f64; 4] {
        let l = self.layout();
        let (w, h) = if self.turn % 180 == 0 { (l.width, l.height) } else { (l.height, l.width) };
        [self.badge[0] - w / 2.0, self.badge[1] - h / 2.0, self.badge[0] + w / 2.0, self.badge[1] + h / 2.0]
    }

    fn seam_width(&self) -> f64 {
        (1.5 * self.mark_scale()).max(0.8)
    }

    fn tick(&self) -> f64 {
        (self.seam_width() * 3.2).max(4.2)
    }

    fn dot_radius(&self) -> f64 {
        (2.7 * self.mark_scale()).max(2.0)
    }

    /// Everything the marker covers, with room for its shadow and halo.
    pub fn bounds(&self) -> [f64; 4] {
        let b = self.badge_rect();
        let pad = 3.0 * self.scale();
        let reach = self.tick().max(self.dot_radius()) + self.seam_width() + 1.0;
        let xs = [b[0] - pad, b[2] + pad, self.point[0] - reach, self.point[0] + reach, self.end[0] - reach, self.end[0] + reach];
        let ys = [b[1] - pad, b[3] + pad, self.point[1] - reach, self.point[1] + reach, self.end[1] - reach, self.end[1] + reach];
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        [min(&xs), min(&ys), max(&xs), max(&ys)]
    }

    /// Every coordinate is a finite number and there is a method.
    pub fn valid(&self) -> bool {
        [self.point, self.end, self.badge].iter().flatten().all(|v| v.is_finite()) && !self.methods.is_empty()
    }

    /// Moved by `(dx, dy)`.
    pub fn moved(&self, dx: f64, dy: f64) -> Weld {
        let m = |p: [f64; 2]| [p[0] + dx, p[1] + dy];
        Weld { point: m(self.point), end: m(self.end), badge: m(self.badge), ..self.clone() }
    }

    /// The `/PCWeld` dictionary.
    pub fn to_dict(&self) -> Dict {
        let pt = |p: [f64; 2]| Object::Array(vec![Object::Real(p[0]), Object::Real(p[1])]);
        let mut d = Dict::new();
        d.set(b"P".to_vec(), pt(self.point));
        d.set(b"E".to_vec(), pt(self.end));
        d.set(b"B".to_vec(), pt(self.badge));
        d.set(b"M".to_vec(), Object::Array(self.methods.iter().map(|m| Object::name(m.key())).collect()));
        d.set(b"N".to_vec(), PdfString::text(&self.label));
        d.set(b"T".to_vec(), PdfString::text(&self.text));
        d.set(b"SN".to_vec(), Object::Bool(self.show_number));
        d.set(b"SM".to_vec(), Object::Bool(self.show_method));
        d.set(b"S".to_vec(), Object::name(if self.shape == WeldShape::Rect { "Rect" } else { "Pill" }));
        d.set(b"K".to_vec(), Object::Real(self.scale()));
        d.set(b"KP".to_vec(), Object::Real(self.point_scale));
        if self.turn != 0 {
            d.set(b"R".to_vec(), Object::Int(self.turn));
        }
        d
    }

    /// Read back a `/PCWeld` dictionary (`None` when it isn't one or is damaged).
    pub fn from_dict(d: &Dict) -> Option<Weld> {
        let pt = |k: &[u8]| -> Option<[f64; 2]> {
            let a = d.get(k)?.as_array()?;
            let (x, y) = (a.first()?.as_f64()?, a.get(1)?.as_f64()?);
            (x.is_finite() && y.is_finite()).then_some([x, y])
        };
        let flag = |k: &[u8], default: bool| match d.get(k) {
            Some(Object::Bool(b)) => *b,
            _ => default,
        };
        let text = |k: &[u8]| d.get(k).and_then(Object::as_string).map(PdfString::to_text).unwrap_or_default();
        let methods: Vec<WeldMethod> = d
            .get(b"M")?
            .as_array()?
            .iter()
            .filter_map(|o| o.as_name())
            .filter_map(|n| WeldMethod::from_key(&String::from_utf8_lossy(n)))
            .take(8)
            .collect();
        if methods.is_empty() {
            return None;
        }
        let point = pt(b"P")?;
        let w = Weld {
            point,
            end: pt(b"E").unwrap_or(point),
            badge: pt(b"B")?,
            methods,
            label: text(b"N").chars().take(24).collect(),
            text: text(b"T").chars().take(60).collect(),
            show_number: flag(b"SN", true),
            show_method: flag(b"SM", true),
            shape: if d.name(b"S") == Some(b"Rect") { WeldShape::Rect } else { WeldShape::Pill },
            scale: d.get(b"K").and_then(Object::as_f64).filter(|v| v.is_finite()).unwrap_or(1.0),
            point_scale: d.get(b"KP").and_then(Object::as_f64).filter(|v| v.is_finite()).unwrap_or(1.0),
            turn: d.int(b"R").unwrap_or(0).rem_euclid(360) / 90 * 90,
        };
        Some(w)
    }
}

fn rg(c: Rgb) -> String {
    format!("{} {} {} rg\n", n(c[0]), n(c[1]), n(c[2]))
}

fn rg_stroke(c: Rgb) -> String {
    format!("{} {} {} RG\n", n(c[0]), n(c[1]), n(c[2]))
}

/// A rounded rectangle path centred on the origin.
fn rounded(w: f64, h: f64, r: f64) -> String {
    let (x0, y0, x1, y1) = (-w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let k = 0.552_284_75 * r;
    format!(
        "{} {} m {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c h\n",
        n(x0 + r),
        n(y0),
        n(x1 - r),
        n(y0),
        n(x1 - r + k),
        n(y0),
        n(x1),
        n(y0 + r - k),
        n(x1),
        n(y0 + r),
        n(x1),
        n(y1 - r),
        n(x1),
        n(y1 - r + k),
        n(x1 - r + k),
        n(y1),
        n(x1 - r),
        n(y1),
        n(x0 + r),
        n(y1),
        n(x0 + r - k),
        n(y1),
        n(x0),
        n(y1 - r + k),
        n(x0),
        n(y1 - r),
        n(x0),
        n(y0 + r),
        n(x0),
        n(y0 + r - k),
        n(x0 + r - k),
        n(y0),
        n(x0 + r),
        n(y0)
    )
}

/// A circle path.
fn circle(cx: f64, cy: f64, r: f64) -> String {
    let k = 0.552_284_75 * r;
    format!(
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h\n",
        n(cx + r),
        n(cy),
        n(cx + r),
        n(cy + k),
        n(cx + k),
        n(cy + r),
        n(cx),
        n(cy + r),
        n(cx - k),
        n(cy + r),
        n(cx - r),
        n(cy + k),
        n(cx - r),
        n(cy),
        n(cx - r),
        n(cy - k),
        n(cx - k),
        n(cy - r),
        n(cx),
        n(cy - r),
        n(cx + k),
        n(cy - r),
        n(cx + r),
        n(cy - k),
        n(cx + r),
        n(cy)
    )
}

/// The matrix that turns the upright badge layout with the page (about its centre).
fn turn_matrix(turn: i64) -> [f64; 4] {
    match turn {
        90 => [0.0, 1.0, -1.0, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0],
        270 => [0.0, -1.0, 1.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0],
    }
}

/// The point on the badge's edge (user space) facing `target`.
pub fn edge_towards(w: &Weld, target: [f64; 2]) -> [f64; 2] {
    let b = w.badge_rect();
    let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    let (vx, vy) = (target[0] - cx, target[1] - cy);
    if vx.hypot(vy) < 0.01 {
        return [cx, cy];
    }
    let (hw, hh) = ((b[2] - b[0]) / 2.0, (b[3] - b[1]) / 2.0);
    let sx = if vx.abs() > 1e-9 { hw / vx.abs() } else { f64::MAX };
    let sy = if vy.abs() > 1e-9 { hh / vy.abs() } else { f64::MAX };
    let s = sx.min(sy);
    [cx + vx * s, cy + vy * s]
}

/// The marker's normal appearance (`/BBox` = its bounds, page space).
pub fn appearance(w: &Weld) -> Stream {
    let bounds = w.bounds();
    let k = w.scale();
    let ms = w.mark_scale();
    let col = w.primary().color();
    let mut c = String::new();
    // Graphics states: shadow (two layers), halo, dot fill, hairline edge, divider.
    let mut gs = Dict::new();
    for (name, fill, stroke) in [("GS1", 0.15, 1.0), ("GS2", 0.07, 1.0), ("GH", 0.25, 0.25), ("GF", 0.38, 1.0), ("GL", 1.0, 0.9), ("GD", 1.0, 0.55)] {
        let mut g = Dict::new();
        g.set(b"Type".to_vec(), Object::name("ExtGState"));
        g.set(b"ca".to_vec(), Object::Real(fill));
        g.set(b"CA".to_vec(), Object::Real(stroke));
        gs.set(name.as_bytes().to_vec(), Object::Dict(g));
    }
    // The point or the seam, with a thin dark halo for contrast (not a frame).
    let sw = w.seam_width();
    let halo = sw + 1.3 * ms;
    if w.has_seam() {
        let (p, e) = (w.point, w.end);
        let len = (e[0] - p[0]).hypot(e[1] - p[1]).max(0.01);
        let (dx, dy) = ((e[0] - p[0]) / len, (e[1] - p[1]) / len);
        let (px, py) = (-dy * w.tick() / 2.0, dx * w.tick() / 2.0);
        let path = format!(
            "{} {} m {} {} l\n{} {} m {} {} l\n{} {} m {} {} l\n",
            n(p[0]),
            n(p[1]),
            n(e[0]),
            n(e[1]),
            n(p[0] - px),
            n(p[1] - py),
            n(p[0] + px),
            n(p[1] + py),
            n(e[0] - px),
            n(e[1] - py),
            n(e[0] + px),
            n(e[1] + py)
        );
        c.push_str(&format!("q /GH gs 0 0 0 RG {} w 1 J\n{path}S Q\n", n(halo)));
        c.push_str(&format!("q {}{} w 1 J\n{path}S Q\n", rg_stroke(col), n(sw)));
    } else {
        let r = w.dot_radius();
        let ring = circle(w.point[0], w.point[1], r);
        c.push_str(&format!("q /GH gs 0 0 0 RG {} w\n{ring}S Q\n", n(halo)));
        c.push_str(&format!("q /GF gs {}{ring}f Q\n", rg(col)));
        c.push_str(&format!("q {}{} w\n{ring}S Q\n", rg_stroke(col), n(sw)));
    }
    // The leader, from the badge's edge to the joint.
    let anchor = w.anchor();
    let edge = edge_towards(w, anchor);
    if (edge[0] - anchor[0]).hypot(edge[1] - anchor[1]) > 2.0 {
        let lw = (1.0 * k).max(0.9);
        let line = format!("{} {} m {} {} l\n", n(edge[0]), n(edge[1]), n(anchor[0]), n(anchor[1]));
        c.push_str(&format!("q /GH gs 0 0 0 RG {} w 1 J\n{line}S Q\n", n(lw + 1.2 * k)));
        c.push_str(&format!("q {}{} w 1 J\n{line}S Q\n", rg_stroke(col), n(lw)));
    }
    // The badge, laid out upright around its centre and turned with the page.
    let l = w.layout();
    let m = turn_matrix(w.turn);
    c.push_str(&format!("q {} {} {} {} {} {} cm\n", n(m[0]), n(m[1]), n(m[2]), n(m[3]), n(w.badge[0]), n(w.badge[1])));
    let shape = rounded(l.width, l.height, l.radius);
    // 1. A soft shadow: two offset copies, faint.
    for (i, g) in ["GS1", "GS2"].iter().enumerate() {
        let off = (0.66 + i as f64 * 0.84) * k;
        c.push_str(&format!("q /{g} gs 0 0 0 rg 1 0 0 1 0 {} cm\n{shape}f Q\n", n(-off)));
    }
    // 2. The colour bands, one per method, clipped to the badge.
    let painted = w.painted();
    if painted.len() == 1 {
        c.push_str(&format!("{}{shape}f\n", rg(painted[0].color())));
    } else {
        c.push_str(&format!("q\n{shape}W n\n"));
        let band = l.width / painted.len() as f64;
        for (i, meth) in painted.iter().enumerate() {
            // A hair of overlap, or thin gaps show between the bands.
            let extra = if i + 1 < painted.len() { 0.3 } else { 0.0 };
            c.push_str(&format!(
                "{}{} {} {} {} re f\n",
                rg(meth.color()),
                n(-l.width / 2.0 + i as f64 * band),
                n(-l.height / 2.0),
                n(band + extra),
                n(l.height)
            ));
        }
        c.push_str("Q\n");
    }
    // 3. A hairline light edge.
    c.push_str(&format!("q /GL gs 1 1 1 RG {} w 1 j\n{shape}S Q\n", n((0.7 * k).max(0.6))));
    // 4. The divider of a split rectangle.
    if let Some(y) = l.divider {
        let inset = 3.0 * k;
        c.push_str(&format!(
            "q /GD gs 1 1 1 RG {} w\n{} {} m {} {} l S Q\n",
            n((0.6 * k).max(0.55)),
            n(-l.width / 2.0 + inset),
            n(y),
            n(l.width / 2.0 - inset),
            n(y)
        ));
    }
    let mut out = c.into_bytes();
    out.extend_from_slice(b"BT\n1 1 1 rg\n");
    for line in &l.lines {
        out.extend(format!("/HelvB {} Tf\n1 0 0 1 {} {} Tm ", n(line.size), n(-line.width / 2.0), n(line.y)).bytes());
        out.extend(literal(&win_ansi(&line.text)));
        out.extend_from_slice(b" Tj\n");
    }
    out.extend_from_slice(b"ET\nQ\n");
    let mut font = Dict::new();
    font.set(b"Type".to_vec(), Object::name("Font"));
    font.set(b"Subtype".to_vec(), Object::name("Type1"));
    font.set(b"BaseFont".to_vec(), Object::name("Helvetica-Bold"));
    font.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
    let mut fonts = Dict::new();
    fonts.set(b"HelvB".to_vec(), Object::Dict(font));
    let mut res = Dict::new();
    res.set(b"Font".to_vec(), Object::Dict(fonts));
    res.set(b"ExtGState".to_vec(), Object::Dict(gs));
    let mut d = Dict::new();
    d.set(b"Type".to_vec(), Object::name("XObject"));
    d.set(b"Subtype".to_vec(), Object::name("Form"));
    d.set(b"FormType".to_vec(), Object::Int(1));
    d.set(b"BBox".to_vec(), Object::Array(bounds.iter().map(|x| Object::Real(*x)).collect()));
    d.set(b"Resources".to_vec(), Object::Dict(res));
    Stream::flate(d, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_combine_and_exclusive_ones_stand_alone() {
        let mut m = vec![WeldMethod::Rt];
        toggle_method(&mut m, WeldMethod::Mt, true);
        assert_eq!(m, [WeldMethod::Rt, WeldMethod::Mt]);
        let w = Weld { methods: m.clone(), label: "7".into(), ..Weld::default() };
        assert_eq!(w.badge_text(), "RT+MT-7");
        assert_eq!(w.painted().len(), 2);
        // The last one stays.
        toggle_method(&mut m, WeldMethod::Mt, true);
        toggle_method(&mut m, WeldMethod::Rt, true);
        assert_eq!(m, [WeldMethod::Rt]);
        // Free text and "no method" replace the others; a method replaces them.
        toggle_method(&mut m, WeldMethod::Text, true);
        assert_eq!(m, [WeldMethod::Text]);
        toggle_method(&mut m, WeldMethod::Ut, true);
        assert_eq!(m, [WeldMethod::Ut]);
        toggle_method(&mut m, WeldMethod::Pt, false);
        assert_eq!(m, [WeldMethod::Pt]);
    }

    #[test]
    fn badge_text_follows_what_is_shown() {
        let free = Weld { methods: vec![WeldMethod::Text], text: " Reparerad ".into(), show_number: false, ..Weld::default() };
        assert_eq!(free.badge_text(), "Reparerad");
        let numbered = Weld { show_number: true, label: "4".into(), ..free.clone() };
        assert_eq!(numbered.badge_text(), "Reparerad-4");
        let only_number = Weld { methods: vec![WeldMethod::None], label: "12".into(), ..Weld::default() };
        assert_eq!(only_number.badge_text(), "12");
        let hidden = Weld { show_method: false, label: "3".into(), ..Weld::default() };
        assert_eq!(hidden.badge_text(), "3");
    }

    #[test]
    fn the_split_rectangle_has_two_lines_only_with_both_parts() {
        let rect = Weld { shape: WeldShape::Rect, label: "5".into(), ..Weld::default() };
        let l = rect.layout();
        assert_eq!(l.lines.len(), 2);
        assert!(l.divider.is_some());
        assert_eq!(l.lines[0].text, "5");
        assert_eq!(l.lines[1].text, "RT");
        let undivided = Weld { show_method: false, ..rect };
        assert_eq!(undivided.layout().lines.len(), 1);
    }

    #[test]
    fn the_dictionary_round_trips() {
        let w = Weld {
            point: [10.0, 20.0],
            end: [60.0, 20.0],
            badge: [35.0, 60.0],
            methods: vec![WeldMethod::Ut, WeldMethod::Vt],
            label: "A12".into(),
            text: String::new(),
            show_number: true,
            show_method: true,
            shape: WeldShape::Rect,
            scale: 1.4,
            point_scale: 0.8,
            turn: 90,
        };
        assert_eq!(Weld::from_dict(&w.to_dict()), Some(w.clone()));
        assert!(w.has_seam());
        let b = w.bounds();
        assert!(b[0] < 10.0 && b[2] > 60.0 && b[3] > 60.0);
        let s = appearance(&w);
        assert!(!s.raw.is_empty());
    }
}
