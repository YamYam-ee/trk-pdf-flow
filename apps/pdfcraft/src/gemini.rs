//! The HTTPS request for Recognize text with AI (`pdfcraft_ui_egui::gemini`): one POST to the
//! Gemini API, over rustls with the operating system's roots, the key in a header (never in
//! the URL, where logs and proxies would keep it).

use std::time::Duration;

/// POST `body` to `url` with `api_key`; the answer's body. Error answers carry Gemini's own
/// message (`{"error": {"message": …}}`), which is passed on.
pub fn post(url: &str, api_key: &str, body: &str) -> Result<String, String> {
    if !url.starts_with("https://generativelanguage.googleapis.com/") {
        return Err("not a Gemini address".into());
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(crate::updates::os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", api_key)
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        .config()
        .http_status_as_error(false)
        .build()
        .send(body)
        .map_err(|e| format!("couldn't reach Gemini ({e})"))?;
    let status = response.status().as_u16();
    let text = response.body_mut().with_config().limit(16 << 20).read_to_string().map_err(|e| format!("unreadable answer ({e})"))?;
    if (200..300).contains(&status) {
        return Ok(text);
    }
    // The status rides along, so the caller can tell a used-up quota (429) or a model that
    // isn't offered (404) from a bad key, and try the next model.
    let message = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string));
    Err(format!("{} (http {status})", message.unwrap_or_else(|| "Gemini refused the request".into())))
}
