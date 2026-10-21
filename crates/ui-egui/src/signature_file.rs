//! Saved signatures and initials in a file of their own (`signatures.json` beside the settings),
//! written the moment they change, with the previous version kept as `signatures.json.bak`.
//!
//! The settings file is written whole by whichever window closes last, so a window opened before
//! a signature was added (or one closed by an installer) could write the old list back over the
//! new one. This file is only ever written by a window whose own list changed, and a window that
//! finds it changed by another takes that version up instead.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::PdfCraftApp;

/// How often (seconds) the list and the file are compared.
const EVERY: f64 = 1.0;

pub(crate) struct SignatureFile {
    path: PathBuf,
    /// What this window last wrote or read (`persist_signatures`).
    known: String,
    /// The file's modification time when it was last written or read.
    modified: Option<SystemTime>,
    /// When the list and the file were last compared (egui time).
    checked: f64,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Write `json` over `path` without ever leaving half a file: the old one becomes `.bak`, the new
/// one is written beside it and renamed into place.
fn write(path: &Path, json: &str) -> std::io::Result<()> {
    if path.exists() {
        let _ = std::fs::copy(path, path.with_extension("json.bak"));
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

impl PdfCraftApp {
    /// Keep the signatures in `path` from now on. The file, if there is one, is the list (it is
    /// never older than the settings' copy); otherwise the list read from the settings starts it.
    pub fn attach_signature_file(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(json) => self.restore_signatures(&json),
            Err(_) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = write(&path, &self.persist_signatures());
            }
        }
        let known = self.persist_signatures();
        self.signature_file = Some(SignatureFile { modified: modified(&path), path, known, checked: f64::NEG_INFINITY });
    }

    /// Write the list when it changed here; take the file's when another window changed it.
    pub fn sync_signature_file(&mut self, now: f64) {
        let Some(f) = self.signature_file.as_mut() else { return };
        if now - f.checked < EVERY {
            return;
        }
        f.checked = now;
        let current = self.persist_signatures();
        let Some(f) = self.signature_file.as_mut() else { return };
        if current != f.known {
            match write(&f.path, &current) {
                Ok(()) => {
                    f.known = current;
                    f.modified = modified(&f.path);
                }
                Err(e) => log::warn!("signatures not saved to {}: {e}", f.path.display()),
            }
        } else if modified(&f.path) != f.modified {
            let path = f.path.clone();
            if let Ok(json) = std::fs::read_to_string(&path) {
                self.replace_signatures(&json);
                let known = self.persist_signatures();
                if let Some(f) = self.signature_file.as_mut() {
                    f.known = known;
                    f.modified = modified(&path);
                }
            }
        }
    }

    /// Take the lists in `json` as they are (another window's changes, removals included).
    fn replace_signatures(&mut self, json: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return };
        if let Some(s) = crate::fill_sign::SigLibrary::from_json(&v["signatures"]) {
            self.signature = s;
            self.saved_signature_previews[0].clear();
        }
        if let Some(s) = crate::fill_sign::SigLibrary::from_json(&v["initials"]) {
            self.initials = s;
            self.saved_signature_previews[1].clear();
        }
    }
}
