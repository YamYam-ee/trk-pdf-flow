//! Unlock a locked PDF: type its password to lift the restrictions on the open document, or,
//! without the password, make an unlocked copy (a new, editable document with the same content;
//! the original file is not changed).

use egui::{Align, Layout};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// The Unlock dialog's password box and what went wrong last.
#[derive(Clone, Debug, Default)]
pub struct UnlockDraft {
    pub password: String,
    pub error: Option<String>,
    /// The password box has had the cursor put in it.
    focused: bool,
}

/// The dialog. Returns `true` to close it.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> bool {
    ui.set_width(460.0);
    ui.label(egui::RichText::new(tl!("Unlock document")).font(theme::semibold(18.0)));
    ui.add_space(6.0);
    ui.label(tl!("This document is locked for editing. Type its password to unlock it."));
    ui.add_space(8.0);
    let mut unlock = false;
    ui.horizontal(|ui| {
        let label = ui.label(tl!("Password"));
        let edit = ui
            .add(egui::TextEdit::singleline(&mut app.unlock_draft.password).password(true).desired_width(260.0).char_limit(256))
            .labelled_by(label.id);
        if !app.unlock_draft.focused {
            edit.request_focus();
            app.unlock_draft.focused = true;
        }
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            unlock = true;
        }
    });
    if let Some(e) = &app.unlock_draft.error {
        ui.label(egui::RichText::new(e).color(egui::Color32::from_rgb(0xC6, 0x28, 0x28)));
    }
    ui.add_space(8.0);
    let mut cancel = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled_ui(!app.unlock_draft.password.is_empty(), |ui| widgets::pill_button(ui, tl!("Unlock"), true)).inner.clicked() {
                unlock = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        })
    });
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(egui::RichText::new(tl!("No password?")).font(theme::semibold(13.0)));
    ui.label(
        egui::RichText::new(tl!("Make an unlocked copy: a new, editable document with the same content. The original file is not changed."))
            .small()
            .color(t.text_muted),
    );
    ui.add_space(4.0);
    let copy = widgets::pill_button(ui, tl!("Make an unlocked copy"), false).clicked();
    if unlock && !app.unlock_draft.password.is_empty() {
        let pw = std::mem::take(&mut app.unlock_draft.password);
        match app.unlock_active(&pw) {
            Ok(()) => {
                app.unlock_draft = UnlockDraft::default();
                app.notify_tr("Unlocked: the document can be edited");
                return true;
            }
            Err(e) => app.unlock_draft.error = Some(e),
        }
    }
    if copy {
        match app.unlocked_copy_of_active() {
            Ok(()) => {
                app.unlock_draft = UnlockDraft::default();
                return true;
            }
            Err(e) => app.unlock_draft.error = Some(e),
        }
    }
    if cancel {
        app.unlock_draft = UnlockDraft::default();
    }
    cancel
}

impl PdfCraftApp {
    /// Reopen the active document with `password`, which must be its owner password: the
    /// restrictions are lifted and the tab stays where it was. Unsaved changes would be lost, so a
    /// changed document must be saved first.
    pub fn unlock_active(&mut self, password: &str) -> Result<(), String> {
        let (index, id) = self.active_ids().ok_or_else(|| tl!("Open a document first").to_string())?;
        let doc = self.session.get(id).ok_or_else(|| tl!("Open a document first").to_string())?;
        if doc.dirty {
            return Err(tl!("Save your changes first, then unlock.").to_string());
        }
        let (name, path, bytes) = (doc.name.clone(), doc.path.clone(), doc.bytes.clone());
        // Try the password on a copy first: a wrong one leaves the open document alone.
        let mut probe = pdfcraft_engine::Session::new();
        let opened = probe.open(&name, None, bytes.clone(), Some(password)).ok();
        let owner = opened.and_then(|p| probe.get(p)).and_then(|d| d.permissions()).is_some_and(|p| p.owner);
        if !owner {
            return Err(tl!("That isn't the document's password.").to_string());
        }
        self.close_tab(index);
        self.try_open(&name, path, bytes, Some(password), true)?;
        // Back into the tab's old place.
        if let Some(view) = self.views.pop() {
            let at = index.min(self.views.len());
            self.views.insert(at, view);
            self.active = Some(at);
        }
        Ok(())
    }

    /// Open an unlocked copy of the active document as a new, unsaved document.
    pub fn unlocked_copy_of_active(&mut self) -> Result<(), String> {
        let (_, id) = self.active_ids().ok_or_else(|| tl!("Open a document first").to_string())?;
        let doc = self.session.get(id).ok_or_else(|| tl!("Open a document first").to_string())?;
        let stem = crate::files::strip_pdf(&doc.name).to_string();
        let bytes = self.session.unlocked_copy(id).map_err(|e| e.to_string())?;
        let name = crate::i18n::fmt(tl!("{name} (unlocked).pdf"), &[("name", &stem)]);
        self.open_created(&name, bytes, "An unlocked copy was opened. Save it to keep it.");
        Ok(())
    }
}
