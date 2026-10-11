//! Edit text & images, live: a paragraph's draft written into a scratch copy of the document by
//! the same rewrite the edit itself uses, and its page rendered as it will look. Preparing it
//! never changes the working bytes, dirty flag or undo history.

use std::sync::Arc;

use pdfcraft_render::{PageRenderer, PageText, RenderRequest, RenderedPage, RequestKind};

use crate::{BlockStyle, Document, guard};

/// A paragraph's draft to preview, holding its own copy of the document so it can be rendered
/// on another thread while the user keeps typing.
pub struct TextPreviewJob {
    cos: pdfcraft_cos::Document,
    page: usize,
    block: usize,
    text: String,
    style: BlockStyle,
    config: pdfcraft_render::RenderConfig,
}

impl Document {
    /// Prepare a preview of paragraph `block` of `page` with `text` (and `style`) written in.
    pub fn text_block_preview(&self, page: usize, block: usize, text: &str, style: &BlockStyle) -> Result<TextPreviewJob, String> {
        let editor = self.editor.as_ref().ok_or("the document can't be read")?;
        Ok(TextPreviewJob { cos: editor.cos.clone(), page, block, text: text.to_string(), style: style.clone(), config: self.config.clone() })
    }
}

impl TextPreviewJob {
    /// Write the draft and render its page at `scale`, with the page's text layer (for the caret).
    pub fn render(mut self, scale: f32) -> Result<(RenderedPage, Option<Arc<PageText>>), String> {
        guard(move || {
            pdfcraft_edit::rewrite_block(&mut self.cos, self.page, self.block, Some(&self.text), &self.style).map_err(|e| e.to_string())?;
            let bytes = pdfcraft_cos::write_incremental(&self.cos, &pdfcraft_cos::SaveOptions::default()).map_err(|e| e.to_string())?;
            let mut renderer = PageRenderer::new(Arc::new(bytes), self.config);
            let pixels = renderer.render(RenderRequest { page: self.page, scale, ..Default::default() });
            if let Some(e) = pixels.error {
                return Err(e);
            }
            let text = renderer.render(RenderRequest { page: self.page, kind: RequestKind::Text, scale: 1.0, ..Default::default() }).text;
            Ok((pixels, text))
        })?
    }
}
