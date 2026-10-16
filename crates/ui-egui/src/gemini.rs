//! Recognize text with AI: a rendered page is sent to Google's Gemini API, which reads it (and
//! corrects what a plain recognizer gets wrong, keeping letters such as å, ä and ö) and answers
//! with each line of text and its box. The lines become the page's searchable text layer like
//! any other recognizer's. The HTTPS request itself is made by the app (`Transport`), which has
//! the TLS stack; this side builds the request and reads the answer.

use std::sync::Arc;

use base64::Engine as _;
use pdfcraft_engine::ocr::{Line, Word};

/// The model used unless Preferences names another.
pub const DEFAULT_MODEL: &str = "gemini-3.6-flash";

/// Send `body` (JSON) to `url` with the API key; the response body, or why not.
pub type Transport = Arc<dyn Fn(&str, &str, &str) -> Result<String, String> + Send + Sync>;

const PROMPT: &str = "This is a scanned document page. Transcribe every piece of text on it, line by line in reading order, \
exactly as printed: keep the original language and spelling, including letters such as å, ä, ö, é and ü, numbers, units and \
punctuation. Correct only what is plainly a misprint of the scan itself (broken or smudged letters). Do not translate, summarize \
or add anything. For each line give its bounding box as box_2d [ymin, xmin, ymax, xmax] normalized to 0-1000. Answer with JSON \
only: {\"lines\": [{\"text\": \"...\", \"box_2d\": [ymin, xmin, ymax, xmax]}]}.";

/// A model name as the API spells them (letters, digits, `-`, `.`, `_`).
pub fn valid_model(model: &str) -> bool {
    !model.is_empty() && model.len() <= 100 && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

/// The `generateContent` endpoint for `model`.
pub fn endpoint(model: &str) -> String {
    format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent")
}

/// The request: the page as a PNG and the instructions, asking for JSON back.
pub fn request_body(png: &[u8]) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    serde_json::json!({
        "contents": [{ "parts": [
            { "inline_data": { "mime_type": "image/png", "data": data } },
            { "text": PROMPT },
        ]}],
        "generationConfig": {
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "OBJECT",
                "properties": { "lines": { "type": "ARRAY", "items": {
                    "type": "OBJECT",
                    "properties": { "text": { "type": "STRING" }, "box_2d": { "type": "ARRAY", "items": { "type": "NUMBER" } } },
                    "required": ["text", "box_2d"],
                }}},
                "required": ["lines"],
            },
        },
    })
    .to_string()
}

/// The most lines read from one page (the answer is untrusted input).
const MAX_LINES: usize = 5000;

/// Read the answer: each line with its box in pixels of the `width` × `height` image sent.
pub fn parse_lines(response: &str, width: u32, height: u32) -> Result<Vec<Line>, String> {
    let v: serde_json::Value = serde_json::from_str(response).map_err(|e| format!("Gemini's answer isn't JSON: {e}"))?;
    if let Some(message) = v["error"]["message"].as_str() {
        return Err(message.to_string());
    }
    if let Some(reason) = v["promptFeedback"]["blockReason"].as_str() {
        return Err(format!("Gemini declined the page ({reason})"));
    }
    let text: String =
        v["candidates"][0]["content"]["parts"].as_array().map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect()).unwrap_or_default();
    // Asked for JSON, but tolerate it fenced as Markdown.
    let json = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    if json.is_empty() {
        return Err("Gemini gave no text for the page".into());
    }
    let answer: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("Gemini's lines aren't JSON: {e}"))?;
    let (w, h) = (width as f32, height as f32);
    let lines = answer["lines"]
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .take(MAX_LINES)
                .filter_map(|l| {
                    let text = l["text"].as_str()?.trim();
                    let b: Vec<f32> = l["box_2d"].as_array()?.iter().filter_map(|n| n.as_f64()).map(|n| (n as f32).clamp(0.0, 1000.0)).collect();
                    let [ymin, xmin, ymax, xmax] = <[f32; 4]>::try_from(b).ok()?;
                    let rect = [xmin.min(xmax) / 1000.0 * w, ymin.min(ymax) / 1000.0 * h, xmin.max(xmax) / 1000.0 * w, ymin.max(ymax) / 1000.0 * h];
                    (!text.is_empty() && rect[2] - rect[0] >= 1.0 && rect[3] - rect[1] >= 1.0)
                        .then(|| Line { words: vec![Word { text: text.to_string(), rect }] })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(lines)
}

/// Read a rendered page (premultiplied RGBA) with Gemini.
pub fn recognize(transport: &Transport, api_key: &str, model: &str, rgba: &[u8], width: u32, height: u32) -> Result<Vec<Line>, String> {
    if api_key.trim().is_empty() {
        return Err("no Gemini API key".into());
    }
    if !valid_model(model) {
        return Err(format!("\"{model}\" isn't a Gemini model name"));
    }
    let png = pdfcraft_engine::export::encode_png(width, height, rgba)?;
    let response = transport(&endpoint(model), api_key.trim(), &request_body(&png))?;
    parse_lines(&response, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_lines_land_in_image_pixels() {
        let answer = serde_json::json!({ "candidates": [{ "content": { "parts": [{ "text":
            "```json\n{\"lines\": [{\"text\": \"Föremål, dimension\", \"box_2d\": [100, 50, 120, 400]}, {\"text\": \"\", \"box_2d\": [1,1,2,2]}, {\"text\": \"bad\", \"box_2d\": [1]}, {\"text\": \"Övrigt\", \"box_2d\": [500, 900, 480, 1200]}]}\n```"
        }]}}]});
        let lines = parse_lines(&answer.to_string(), 2000, 1000).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text(), "Föremål, dimension");
        assert_eq!(lines[0].words[0].rect, [100.0, 100.0, 800.0, 120.0]);
        // Swapped and out-of-range coordinates are put in order and kept on the image.
        assert_eq!(lines[1].words[0].rect, [1800.0, 480.0, 2000.0, 500.0]);
    }

    #[test]
    fn gemini_errors_say_why() {
        let e = parse_lines(r#"{"error": {"code": 400, "message": "API key not valid"}}"#, 10, 10).unwrap_err();
        assert_eq!(e, "API key not valid");
        assert!(parse_lines(r#"{"promptFeedback": {"blockReason": "OTHER"}}"#, 10, 10).unwrap_err().contains("OTHER"));
        assert!(parse_lines("not json", 10, 10).is_err());
        assert!(!valid_model("models/../x") && valid_model(DEFAULT_MODEL));
        let body: serde_json::Value = serde_json::from_str(&request_body(b"png")).unwrap();
        assert_eq!(body["contents"][0]["parts"][0]["inline_data"]["data"], "cG5n");
    }
}
