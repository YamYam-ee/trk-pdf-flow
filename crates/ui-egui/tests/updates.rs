//! Help ▸ Check for updates (issue #28), with stand-in release sources (no network).

use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::updates::{Release, UpdateSource, is_newer};

fn source(answer: Result<&str, &str>) -> UpdateSource {
    let answer = answer.map(str::to_string).map_err(str::to_string);
    Arc::new(move || answer.clone().map(|v| Release { url: format!("https://github.com/YamYam-ee/trk-pdf-flow/releases/tag/{v}"), version: v }))
}

fn harness(answer: Result<&str, &str>) -> Harness<'static, PdfCraftApp> {
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.update_source = Some(source(answer));
        app
    })
}

/// Run frames until the background check has reported (or give up).
fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.query_by_label_contains("Checking for a newer version").is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(2);
}

#[test]
fn versions_compare_by_number() {
    assert!(is_newer("v0.2.0", "0.1.1"));
    assert!(is_newer("v0.1.10", "0.1.9"));
    assert!(is_newer("1", "0.9.9"));
    assert!(!is_newer("v0.1.1", "0.1.1"));
    assert!(!is_newer("v0.1.0", "0.1.1"));
    assert!(!is_newer("v0.1.1-beta.2", "0.1.1"), "suffixes are ignored");
    assert!(!is_newer("nightly", "0.1.1"), "a tag that isn't a version is never newer");
    assert!(!is_newer("v1.2.3.4", "0.1.1"));
    assert!(!is_newer("v99999999999999999999.0.0", "0.1.1"), "out of range");
}

#[test]
fn a_newer_release_is_offered_for_download() {
    let mut h = harness(Ok("v99.0.0"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("TRK PDF Flow 99.0.0 is available");
    h.get_by_label("Download");
    h.get_by_label("Later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none(), "Later closes the dialog");
}

#[test]
fn an_up_to_date_or_failed_check_says_so() {
    let mut h = harness(Ok(concat!("v", env!("CARGO_PKG_VERSION"))));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("is up to date");
    assert!(h.query_by_label("Download").is_none());

    let mut h = harness(Err("couldn't reach GitHub"));
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    h.get_by_label_contains("Couldn't check for updates: couldn't reach GitHub");
}

#[test]
fn a_newer_version_is_announced_at_start_unless_turned_off() {
    // This build asks once at start and speaks up only when a newer version is out.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = calls.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.restore(r#"{"check_updates_at_start": false}"#);
        let counted = counted.clone();
        app.update_source = Some(Arc::new(move || {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Release { version: "v99.0.0".into(), url: "https://github.com/YamYam-ee/trk-pdf-flow/releases/tag/v99.0.0".into() })
        }));
        app
    });
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().execute("help.check_updates");
    settle(&mut h);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    h.get_by_label_contains("TRK PDF Flow 99.0.0 is available");
}

#[test]
fn the_check_at_start_speaks_up_only_for_a_newer_version() {
    let mut h = harness(Ok("v99.0.0"));
    settle(&mut h);
    h.get_by_label_contains("TRK PDF Flow 99.0.0 is available");
    let mut h = harness(Ok(concat!("v", env!("CARGO_PKG_VERSION"))));
    settle(&mut h);
    assert!(h.query_by_label_contains("is up to date").is_none(), "no news, no dialog");
    let mut h = harness(Err("offline"));
    settle(&mut h);
    assert!(h.query_by_label_contains("Couldn't check").is_none(), "a failed check at start stays quiet");
}
