//! Help ▸ Check for updates (issue #28): ask for the latest release and offer its download page.
//!
//! The desktop app supplies how to ask ([`PdfCraftApp::update_source`]), so this crate has no
//! network code; without a source (the web build, tests) the command opens the releases page.
//! It never downloads or installs anything itself: the user downloads the new version's installer,
//! which replaces the installed one. This build asks once at start too (unless turned off in
//! Preferences) and speaks up only when a newer version is out.

use std::sync::Arc;

use egui::{Align, Layout};

use crate::{PdfCraftApp, theme, widgets};

/// Where every PdfCraft release is listed.
pub const RELEASES_PAGE: &str = "https://github.com/YamYam-ee/trk-pdf-flow/releases";

/// The latest published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version tag, such as `v0.2.0`.
    pub version: String,
    /// Its page on [`RELEASES_PAGE`], where the downloads are.
    pub url: String,
    /// Its installer (the `.msi`), when it has one.
    pub installer: Option<String>,
}

/// Asks for the latest release (blocking; it runs on its own thread).
pub type UpdateSource = Arc<dyn Fn() -> Result<Release, String> + Send + Sync>;
/// Downloads an installer (blocking; on its own thread): where it was saved.
pub type UpdateDownload = Arc<dyn Fn(&str) -> Result<std::path::PathBuf, String> + Send + Sync>;
/// Starts an installer that replaces this app (the app then closes so its files are free).
pub type UpdateInstall = Arc<dyn Fn(&std::path::Path) -> Result<(), String> + Send + Sync>;

/// Whether release `latest` (a tag such as `v0.2.0`) is newer than version `current` (`0.1.1`).
/// Pre-release and build suffixes are ignored; a version that doesn't parse is never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse(latest), parse(current)), (Some(l), Some(c)) if l > c)
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = |required: bool| match parts.next() {
        Some(p) => p.parse::<u64>().ok(),
        None if required => None,
        None => Some(0),
    };
    let version = (next(true)?, next(false)?, next(false)?);
    parts.next().is_none().then_some(version)
}

/// Where a check is.
#[derive(Default)]
pub(crate) enum Check {
    #[default]
    Idle,
    #[cfg(not(target_arch = "wasm32"))]
    Running(std::sync::mpsc::Receiver<Result<Release, String>>),
    Done(Result<Release, String>),
    /// The new version's installer on its way.
    #[cfg(not(target_arch = "wasm32"))]
    Downloading(String, std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>),
    /// The installer, downloaded: the version and where it is.
    Downloaded(String, std::path::PathBuf),
}

#[derive(Default)]
pub(crate) struct Updates {
    pub(crate) check: Check,
    /// The Updates dialog is showing.
    pub(crate) open: bool,
    /// The running check is the quiet one at start: its answer shows only if it is news.
    pub(crate) quiet: bool,
    /// The check at start has been made.
    pub(crate) started: bool,
    /// Install once every document is closed: the downloaded installer.
    pub(crate) install_when_closed: Option<std::path::PathBuf>,
}

impl PdfCraftApp {
    /// Help ▸ Check for updates: ask for the latest release and show the outcome.
    pub fn check_for_updates(&mut self) {
        let Some(source) = self.update_source.clone() else {
            self.open_url(RELEASES_PAGE);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.updates.open = true;
            if matches!(self.updates.check, Check::Running(_)) {
                return;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                // The receiver may be gone (the app quit): nothing to report to then.
                let _ = tx.send(source());
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.check = Check::Running(rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = source;
            self.open_url(RELEASES_PAGE);
        }
    }

    /// Once, at start: ask quietly, and show the Updates dialog only if a newer version is out.
    pub(crate) fn check_for_updates_at_start(&mut self) {
        if self.updates.started || !self.check_updates_at_start || self.update_source.is_none() {
            return;
        }
        self.updates.started = true;
        let was_open = self.updates.open;
        self.check_for_updates();
        self.updates.open = was_open;
        self.updates.quiet = true;
    }

    /// Pick up a finished check (each frame).
    pub(crate) fn poll_updates(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Check::Running(rx) = &self.updates.check {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the update check stopped unexpectedly".into()),
            };
            if std::mem::take(&mut self.updates.quiet) && matches!(&result, Ok(r) if is_newer(&r.version, env!("CARGO_PKG_VERSION"))) {
                self.updates.open = true;
            }
            self.updates.check = Check::Done(result);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Check::Downloading(version, rx) = &self.updates.check {
            let version = version.clone();
            match rx.try_recv() {
                Ok(Ok(path)) => self.updates.check = Check::Downloaded(version, path),
                Ok(Err(e)) => self.updates.check = Check::Done(Err(e)),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.updates.check = Check::Done(Err("the download stopped unexpectedly".into())),
            }
        }
        // Every document closed (or one with changes kept open): install, or give up.
        if self.close_request.is_none()
            && let Some(path) = self.updates.install_when_closed.take()
        {
            if self.views.is_empty() {
                self.run_installer(&path);
            } else {
                self.notify_tr("The update waits: close the open documents first.");
            }
        }
    }

    /// Fetch the new version's installer in the background.
    fn download_update(&mut self, version: String, url: String) {
        let Some(download) = self.update_download.clone() else {
            self.open_url(RELEASES_PAGE);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(download(&url));
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
            self.updates.check = Check::Downloading(version, rx);
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (download, version, url);
            self.open_url(RELEASES_PAGE);
        }
    }

    /// Close every document (asking about unsaved changes), then install.
    fn install_update(&mut self, path: std::path::PathBuf) {
        self.close_all();
        self.updates.install_when_closed = Some(path);
    }

    fn run_installer(&mut self, path: &std::path::Path) {
        let Some(install) = self.update_install.clone() else { return };
        match install(path) {
            Ok(()) => {
                self.updates.open = false;
                if let Some(ctx) = &self.ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(e) => self.notify_fmt("The installer didn't start: {e}", &[("e", &e)]),
        }
    }
}

/// What the Updates dialog's buttons ask for.
enum Choice {
    /// Open the release page.
    Page(String),
    /// Download the installer: the version and its address.
    Download(String, String),
    /// Close every document and run the downloaded installer.
    Install(std::path::PathBuf),
}

/// The Updates dialog.
pub(crate) fn dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    if !app.updates.open {
        return;
    }
    let t = theme::Tokens::get(ctx);
    let current = env!("CARGO_PKG_VERSION");
    let mut close = false;
    let mut choice: Option<Choice> = None;
    let can_install = app.update_download.is_some() && app.update_install.is_some();
    let open_docs = app.views.len();
    let modal = egui::Modal::new(egui::Id::new("updates")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.horizontal(|ui| {
            ui.add(crate::icons::image("cloud", 22.0, t.accent));
            ui.label(egui::RichText::new(tl!("Check for updates")).font(theme::semibold(16.0)));
        });
        ui.add_space(8.0);
        // The buttons this state offers: (label, primary, choice); "close" closes.
        let mut buttons: Vec<(&str, bool, Option<Choice>)> = Vec::new();
        match &app.updates.check {
            Check::Idle => {
                ui.label(tl!("No check has run yet."));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Check::Running(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(tl!("Checking for a newer version…"));
                });
            }
            Check::Done(Ok(r)) if is_newer(&r.version, current) => {
                let version = r.version.trim_start_matches(['v', 'V']);
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("PdfCraft {v} is available."), &[("v", version)])).strong());
                match (&r.installer, can_install) {
                    (Some(installer), true) => {
                        ui.label(
                            egui::RichText::new(crate::i18n::fmt(
                                tl!("You have version {c}. The new version downloads and installs from here; your settings stay."),
                                &[("c", current)],
                            ))
                            .color(t.text_muted),
                        );
                        buttons.push((tl!("Download and install"), true, Some(Choice::Download(version.to_string(), installer.clone()))));
                    }
                    _ => {
                        ui.label(
                            egui::RichText::new(crate::i18n::fmt(
                                tl!("You have version {c}. Download the new version from its release page."),
                                &[("c", current)],
                            ))
                            .color(t.text_muted),
                        );
                        buttons.push((tl!("Download"), true, Some(Choice::Page(r.url.clone()))));
                    }
                }
                buttons.push((tl!("Later"), false, None));
            }
            Check::Done(Ok(_)) => {
                ui.label(crate::i18n::fmt(tl!("PdfCraft {c} is up to date."), &[("c", current)]));
            }
            Check::Done(Err(e)) => {
                ui.label(crate::i18n::fmt(tl!("Couldn't check for updates: {e}"), &[("e", &e.to_string())]));
                ui.label(
                    egui::RichText::new(crate::i18n::fmt(
                        tl!("You have version {c}. All releases are listed at {page}."),
                        &[("c", current), ("page", RELEASES_PAGE)],
                    ))
                    .color(t.text_muted),
                );
            }
            #[cfg(not(target_arch = "wasm32"))]
            Check::Downloading(version, _) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(crate::i18n::fmt(tl!("Downloading version {v}…"), &[("v", version)]));
                });
            }
            Check::Downloaded(version, path) => {
                ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Version {v} is ready to install."), &[("v", version)])).strong());
                ui.label(
                    egui::RichText::new(if open_docs > 0 {
                        tl!("The open documents close first (you are asked about unsaved changes). The app closes while the new version installs and starts again afterwards.")
                    } else {
                        tl!("The app closes while the new version installs and starts again afterwards.")
                    })
                    .color(t.text_muted),
                );
                let label = if open_docs > 0 { tl!("Close all files and install") } else { tl!("Install now") };
                buttons.push((label, true, Some(Choice::Install(path.clone()))));
                buttons.push((tl!("Later"), false, None));
            }
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if buttons.is_empty() {
                buttons.push((tl!("Close"), true, None));
            }
            for (label, primary, action) in buttons {
                if widgets::pill_button(ui, label, primary).clicked() {
                    match action {
                        Some(a) => choice = Some(a),
                        None => close = true,
                    }
                }
            }
        });
    });
    if modal.should_close() {
        close = true;
    }
    match choice {
        Some(Choice::Page(url)) => {
            app.updates.open = false;
            app.open_url(&url);
        }
        Some(Choice::Download(version, url)) => app.download_update(version, url),
        Some(Choice::Install(path)) => app.install_update(path),
        None if close => app.updates.open = false,
        None => {}
    }
}
