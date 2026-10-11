//! Edit text & images, live: a paragraph's draft written into a scratch copy of the document by
//! the same rewrite the edit itself uses, for the page to be rendered as it will look. Preparing
//! it never changes the working bytes, dirty flag or undo history.

use std::sync::Arc;

use crate::{BlockStyle, Document, guard};

/// The document with one paragraph's draft written in, ready to render.
pub struct TextBlockPreview {
    bytes: Arc<Vec<u8>>,
    config: pdfcraft_render::RenderConfig,
}

impl TextBlockPreview {
    /// One background worker for the preview's page and text layer.
    pub fn renderer(&self) -> pdfcraft_render::RenderPool {
        pdfcraft_render::RenderPool::new(self.bytes.clone(), 1, self.config.clone())
    }
}

impl Document {
    /// Write `text` (with `style`) into paragraph `block` of `page` in a copy of the document.
    pub fn text_block_preview(&self, page: usize, block: usize, text: &str, style: &BlockStyle) -> Result<TextBlockPreview, String> {
        guard(|| {
            let editor = self.editor.as_ref().ok_or("the document can't be read")?;
            let mut cos = editor.cos.clone();
            pdfcraft_edit::rewrite_block(&mut cos, page, block, Some(text), style).map_err(|e| e.to_string())?;
            let bytes = pdfcraft_cos::write_incremental(&cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?;
            Ok(TextBlockPreview { bytes: Arc::new(bytes), config: self.config.clone() })
        })?
    }
}
