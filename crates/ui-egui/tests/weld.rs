//! Weld joint markers (TRK): placing, numbering, methods, free text, handles and editing.

use egui::Pos2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{Weld, WeldMethod};
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

/// One 300 × 400 pt page.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness(setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("welds.pdf", None, FIXTURE.to_vec()).unwrap();
        app.set_option("left", "closed").unwrap();
        app.set_option("panel", "none").unwrap();
        app.set_option("zoom", "150").unwrap();
        app.set_option("quick", "weld").unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(6);
    h
}

/// User space → screen on the first page.
fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).expect("on screen");
    let k = r.width() / 300.0;
    egui::pos2(r.min.x + x * k, r.min.y + (400.0 - y) * k)
}

fn welds(h: &Harness<'static, PdfCraftApp>) -> Vec<Weld> {
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    doc.info.annotations.iter().filter(|a| a.weld.is_some()).filter_map(|a| doc.weld(a.page, a.index)).collect()
}

fn click(h: &mut Harness<'static, PdfCraftApp>, p: Pos2) {
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(3);
}

fn press_done(h: &mut Harness<'static, PdfCraftApp>) {
    let done = h.get_by_label("Done").rect().center();
    h.hover_at(done);
    h.run_steps(1);
    h.get_by_label("Done").click();
    h.run_steps(4);
}

#[test]
fn the_number_is_asked_for_until_running_numbers_are_ticked() {
    let mut h = harness(|_| {});
    // The first joint: the number is asked for, with the next one suggested.
    let p = at(&h, 100.0, 200.0);
    click(&mut h, p);
    assert!(welds(&h).is_empty(), "not placed before the number is given");
    h.get_by_label("Joint number");
    // Type the drawing's own number and continue with running numbers from it.
    h.event(egui::Event::Text("2".into()));
    h.run_steps(1);
    h.get_by_label("Continue with running numbers from here").click();
    h.run_steps(1);
    press_done(&mut h);
    let w = welds(&h);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].badge_text(), "RT-2", "the typed number replaces the suggested 1: {:?}", w[0].label);
    assert!((w[0].point[0] - 100.0).abs() < 1.0 && (w[0].point[1] - 200.0).abs() < 1.0, "where it was clicked: {:?}", w[0].point);
    assert!(w[0].badge[1] > w[0].point[1] + 10.0, "the badge above the point");
    // From now on the joints number themselves.
    let p = at(&h, 200.0, 200.0);
    click(&mut h, p);
    assert!(h.query_by_label("Joint number").is_none(), "no more questions");
    let labels: Vec<String> = welds(&h).iter().map(|w| w.label.clone()).collect();
    assert_eq!(labels, ["2", "3"]);
    assert_eq!(h.state().quick_tool, QuickTool::Weld, "the tool stays on");
}

#[test]
fn cancel_removes_the_joint_just_placed() {
    let mut h = harness(|_| {});
    let p = at(&h, 100.0, 200.0);
    click(&mut h, p);
    let cancel = h.get_by_label("Cancel").rect().center();
    h.hover_at(cancel);
    h.run_steps(1);
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(welds(&h).is_empty());
    assert!(h.state().session.get(h.state().views[0].id).unwrap().can_undo().is_none(), "nothing was added");
}

#[test]
fn several_methods_share_a_badge_and_free_text_is_typed_into_it() {
    let mut h = harness(|app| {
        app.weld_prefs.methods = vec![WeldMethod::Rt, WeldMethod::Mt];
        app.views[0].weld_numbers.running = true;
    });
    let p = at(&h, 100.0, 250.0);
    click(&mut h, p);
    let w = welds(&h);
    assert_eq!(w[0].badge_text(), "RT+MT-1");
    assert_eq!(w[0].painted(), [WeldMethod::Rt, WeldMethod::Mt], "a colour band each");
    // Per-method series: both moved on.
    h.state_mut().weld_prefs.methods = vec![WeldMethod::Mt];
    h.run_steps(1);
    h.state_mut().views[0].weld_numbers.running = true;
    let p = at(&h, 200.0, 250.0);
    click(&mut h, p);
    assert_eq!(welds(&h)[1].badge_text(), "MT-2");
    // Free text: typed straight into the badge, Enter finishes; no number.
    h.state_mut().weld_prefs.methods = vec![WeldMethod::Text];
    let p = at(&h, 150.0, 120.0);
    click(&mut h, p);
    h.event(egui::Event::Text("Reparerad".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    let w = welds(&h);
    assert_eq!(w.len(), 3);
    assert_eq!(w[2].badge_text(), "Reparerad");
    assert!(!w[2].show_number);
}

#[test]
fn a_drag_places_the_badge_and_the_point_stays() {
    let mut h = harness(|app| app.views[0].weld_numbers.running = true);
    let (from, to) = (at(&h, 100.0, 200.0), at(&h, 180.0, 300.0));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for k in 1..=5 {
        h.hover_at(from + (to - from) * (k as f32 / 5.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(4);
    let w = &welds(&h)[0];
    assert!((w.point[0] - 100.0).abs() < 1.0 && (w.point[1] - 200.0).abs() < 1.0, "{:?}", w.point);
    assert!((w.badge[0] - 180.0).abs() < 1.0 && (w.badge[1] - 300.0).abs() < 1.0, "{:?}", w.badge);
}

#[test]
fn a_selected_marker_has_handles_and_double_click_edits_it() {
    let mut h = harness(|app| app.views[0].weld_numbers.running = true);
    let p = at(&h, 100.0, 200.0);
    click(&mut h, p);
    // Select it with the Select tool.
    h.state_mut().quick_tool = QuickTool::Select;
    let index = {
        let s = h.state();
        s.session.get(s.views[0].id).unwrap().info.annotations.iter().find(|a| a.weld.is_some()).unwrap().index
    };
    h.state_mut().views[0].comments.selected = Some((0, index));
    h.run_steps(2);
    // Drag the badge's handle somewhere else: only the badge moves.
    let w = welds(&h)[0].clone();
    let (from, to) = (at(&h, w.badge[0] as f32, w.badge[1] as f32), at(&h, 220.0, 320.0));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(4);
    let moved = welds(&h)[0].clone();
    assert!((moved.badge[0] - 220.0).abs() < 1.5 && (moved.badge[1] - 320.0).abs() < 1.5, "{:?}", moved.badge);
    assert_eq!(moved.point, w.point, "the point stays");
    // Double-click it: change the methods and the number.
    let b = at(&h, moved.badge[0] as f32, moved.badge[1] as f32);
    h.hover_at(b);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: b, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(2);
    h.get_by_label("Test method");
    // (The tool bar has the same buttons: the dialog's come first, it is drawn before the bar.)
    h.query_all_by_label("UT").next().expect("the dialog's UT").click();
    h.run_steps(1);
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert_eq!(welds(&h)[0].badge_text(), "UT-1");
}

#[test]
fn the_tool_settings_survive_a_restart() {
    let h = harness(|app| {
        app.weld_prefs.methods = vec![WeldMethod::Paut, WeldMethod::Vt];
        app.weld_prefs.shape = pdfcraft_engine::WeldShape::Rect;
        app.weld_prefs.text_number = true;
    });
    let saved = h.state().persist();
    let mut again = PdfCraftApp::new();
    again.restore(&saved);
    assert_eq!(again.weld_prefs, h.state().weld_prefs);
}
