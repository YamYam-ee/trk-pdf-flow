//! Scrolling past the end of the document: the pages give a little and spring back, and are
//! never thrown back up.

use egui_kittest::Harness;
use pdfcraft_ui_egui::PdfCraftApp;

fn pdf(n: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i)).collect();
    let mut s = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [{}] /Count {n} >> endobj\n",
        kids.join(" ")
    );
    for i in 0..n {
        s += &format!("{} 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] >> endobj\n", 3 + i);
    }
    s += "trailer << /Root 1 0 R >>\n%%EOF";
    s.into_bytes()
}

/// The last page's top on screen, frame by frame, while the wheel scrolls down for 300 frames.
fn wheel_down(layout: &'static str, zoom: &'static str) -> Vec<f32> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("p.pdf", None, pdf(3)).unwrap();
        app.set_option("layout", layout).unwrap();
        app.set_option("zoom", zoom).unwrap();
        app
    });
    h.run_steps(6);
    h.hover_at(egui::pos2(600.0, 450.0));
    h.run_steps(1);
    let mut tops = Vec::new();
    for frame in 0..360 {
        if frame < 300 && frame % 3 == 0 {
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -3.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(1);
        let s = h.state();
        let v = &s.views[0];
        tops.push(if v.current == 2 { v.page_screen_rect(2).map_or(f32::NAN, |r| r.top()) } else { f32::NAN });
    }
    tops
}

#[test]
fn the_end_of_the_document_is_never_thrown_back_up() {
    for (layout, zoom) in [("continuous", "100"), ("single", "300")] {
        let tops = wheel_down(layout, zoom);
        // Once at the last page's bottom, no frame jumps up by more than the rubber band's give.
        let reached = tops.iter().position(|t| t.is_finite()).expect("reached the last page");
        let settled = *tops.last().unwrap();
        assert!(settled < -100.0, "{layout}: ends at the last page's bottom, not its top: {settled}");
        for w in tops[reached..].windows(2) {
            assert!(w[1] - w[0] < 120.0, "{layout}: thrown up from {} to {}", w[0], w[1]);
        }
    }
}
