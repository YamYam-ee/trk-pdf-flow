//! Document tabs: renaming one in place, dragging it to another place in the row, and opening a
//! file that is already open.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

const PDF: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        for (name, path) in [("alpha.pdf", "C:\\docs\\alpha.pdf"), ("beta.pdf", "C:\\docs\\beta.pdf"), ("gamma.pdf", "C:\\docs\\gamma.pdf")] {
            app.open_bytes(name, Some(path.into()), PDF.to_vec()).unwrap();
        }
        app
    });
    h.run_steps(6);
    h
}

fn names(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    (0..h.state().views.len()).filter_map(|i| h.state().tab_label(i)).collect()
}

fn tab_center(h: &Harness<'static, PdfCraftApp>, name: &str) -> egui::Pos2 {
    // The tab is the first node with the name (the window title and others come later).
    h.query_all_by_label(name).next().expect("the tab").rect().center()
}

fn press(h: &mut Harness<'static, PdfCraftApp>, p: egui::Pos2, button: egui::PointerButton) {
    // Down and up in one frame, as a quick click arrives (two of them make a double-click).
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(1);
}

#[test]
fn a_tab_is_renamed_in_place_by_double_click_or_from_its_menu() {
    let mut h = harness();
    // Double-click: the name is typed on the tab (all of it selected), Enter keeps it.
    let p = tab_center(&h, "beta.pdf");
    h.hover_at(p);
    h.run_steps(1);
    press(&mut h, p, egui::PointerButton::Primary);
    press(&mut h, p, egui::PointerButton::Primary);
    h.run_steps(2);
    assert!(h.state().tab_rename.is_some(), "the tab is being renamed");
    h.event(egui::Event::Text("Protokoll".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(names(&h), ["alpha.pdf", "Protokoll", "gamma.pdf"]);
    // From the menu: Rename starts the same thing; Esc leaves the name alone.
    let p = tab_center(&h, "gamma.pdf");
    h.hover_at(p);
    h.run_steps(1);
    press(&mut h, p, egui::PointerButton::Secondary);
    h.run_steps(2);
    let item = h.get_by_label("Rename").rect().center();
    h.hover_at(item);
    h.run_steps(1);
    press(&mut h, item, egui::PointerButton::Primary);
    h.run_steps(2);
    assert!(h.state().tab_rename.is_some(), "Rename starts typing on the tab");
    h.event(egui::Event::Text("Ritning".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(names(&h), ["alpha.pdf", "Protokoll", "Ritning"]);
    assert!(h.state().tab_rename.is_none());
}

#[test]
fn a_tab_dragged_along_the_row_takes_its_new_place() {
    let mut h = harness();
    h.state_mut().active = Some(0);
    h.run_steps(2);
    let (from, to) = (tab_center(&h, "alpha.pdf"), tab_center(&h, "gamma.pdf"));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for k in 1..=12 {
        h.hover_at(from + (to + egui::vec2(20.0, 0.0) - from) * (k as f32 / 12.0));
        h.run_steps(1);
    }
    h.drop_at(to + egui::vec2(20.0, 0.0));
    h.run_steps(3);
    assert_eq!(names(&h), ["beta.pdf", "gamma.pdf", "alpha.pdf"], "dragged past both to the end");
    assert_eq!(h.state().tab_label(h.state().active.unwrap()).as_deref(), Some("alpha.pdf"), "the same document stays active");
    assert!(h.state().tab_drag.is_none());
    // And back to the left, one place.
    let (from, to) = (tab_center(&h, "alpha.pdf"), tab_center(&h, "gamma.pdf"));
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for k in 1..=8 {
        h.hover_at(from + (to - egui::vec2(15.0, 0.0) - from) * (k as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(to - egui::vec2(15.0, 0.0));
    h.run_steps(3);
    assert_eq!(names(&h), ["beta.pdf", "alpha.pdf", "gamma.pdf"]);
}

#[test]
fn a_file_that_is_already_open_is_shown_not_opened_again() {
    let mut h = harness();
    assert_eq!(h.state().active, Some(2));
    // The same file again (another spelling of its path): its tab comes forward.
    h.state_mut().open_bytes("alpha.pdf", Some("c:/docs/ALPHA.pdf".into()), PDF.to_vec()).unwrap();
    h.run_steps(2);
    assert_eq!(names(&h).len(), 3, "no fourth tab");
    assert_eq!(h.state().active, Some(0));
    // Another file with the same name somewhere else is another document.
    h.state_mut().open_bytes("alpha.pdf", Some("C:\\other\\alpha.pdf".into()), PDF.to_vec()).unwrap();
    h.run_steps(2);
    assert_eq!(names(&h).len(), 4);
}
