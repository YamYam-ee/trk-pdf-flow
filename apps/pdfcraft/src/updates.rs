//! Asks GitHub for the latest PdfCraft release (Help ▸ Check for updates, issue #28).

use std::time::Duration;

use pdfcraft_ui_egui::updates::{RELEASES_PAGE, Release};

const LATEST: &str = "https://api.github.com/repos/YamYam-ee/trk-pdf-flow/releases/latest";
/// Where release files are downloaded from: an installer from anywhere else is never fetched.
const DOWNLOADS: &str = "https://github.com/YamYam-ee/trk-pdf-flow/releases/download/";
/// The largest installer accepted (the 1.0 one is about 30 MB).
const MAX_INSTALLER: u64 = 400 << 20;

/// The latest release. The answer is untrusted: its size is capped, and only a page under
/// [`RELEASES_PAGE`] is ever offered for download (anything else falls back to that list).
pub fn latest_release() -> Result<Release, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        // Read error statuses ourselves: a 403 from GitHub usually means its anonymous
        // rate limit ran out, and the retry time rides along in a response header.
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .map_err(|e| format!("couldn't reach GitHub ({e})"))?;
    let status = response.status().as_u16();
    if status == 403 {
        let reset = response.headers().get("x-ratelimit-reset").and_then(|v| v.to_str().ok());
        return Err(rate_limited(reset, now_unix()));
    }
    if !(200..300).contains(&status) {
        return Err(format!("couldn't reach GitHub (http status: {status})"));
    }
    let body = response.body_mut().with_config().limit(1 << 20).read_to_string().map_err(|e| format!("unreadable answer ({e})"))?;
    parse(&body)
}

/// The certificate authorities the operating system trusts.
pub(crate) fn os_roots() -> Result<ureq::tls::RootCerts, String> {
    let found = rustls_native_certs::load_native_certs();
    let certs: Vec<ureq::tls::Certificate<'static>> = found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect();
    if certs.is_empty() {
        return Err("no trusted certificates found on this system".into());
    }
    Ok(ureq::tls::RootCerts::new_with_certs(&certs))
}

/// The current Unix time in seconds (0 when the clock gives nothing usable).
fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// What to tell the user when GitHub answers 403: its anonymous quota (60 checks an hour,
/// shared by everyone behind the same address) ran out. `reset` is the `x-ratelimit-reset`
/// header, the Unix time the quota refills; `now` is the current Unix time. Anything
/// surprising (no header, not a number, already past) falls back to asking for patience.
fn rate_limited(reset: Option<&str>, now: u64) -> String {
    const FALLBACK: &str = "GitHub is limiting update checks for now (shared hourly quota); try again later";
    let wait = reset.and_then(|r| r.trim().parse::<u64>().ok()).and_then(|reset| reset.checked_sub(now)).filter(|&wait| wait > 0);
    match wait {
        Some(wait) => {
            let minutes = wait.div_ceil(60).max(1);
            let plural = if minutes == 1 { "" } else { "s" };
            format!("GitHub is limiting update checks for now (shared hourly quota); try again in about {minutes} minute{plural}")
        }
        None => FALLBACK.into(),
    }
}

fn parse(body: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("unreadable answer ({e})"))?;
    let version = v["tag_name"].as_str().filter(|t| !t.is_empty() && t.len() <= 64).ok_or("no release found")?.to_string();
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.strip_prefix(RELEASES_PAGE).is_some_and(|rest| rest.starts_with('/') && !rest.contains(['?', '#', '\\'])))
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    // The Windows installer, if the release has one (from this repository's downloads only).
    let installer = v["assets"].as_array().and_then(|assets| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str()?;
            let url = a["browser_download_url"].as_str()?;
            (name.to_ascii_lowercase().ends_with(".msi") && safe_download(url)).then(|| url.to_string())
        })
    });
    Ok(Release { version, url, installer })
}

/// A file of one of this repository's releases (no other host, no tricks in the path).
fn safe_download(url: &str) -> bool {
    url.strip_prefix(DOWNLOADS).is_some_and(|rest| {
        let mut parts = rest.split('/');
        let (tag, file) = (parts.next(), parts.next());
        parts.next().is_none()
            && tag.is_some_and(|t| !t.is_empty() && !t.contains(".."))
            && file.is_some_and(|f| !f.is_empty() && !f.contains("..") && f.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
            && !rest.contains(['?', '#', '\\', '%'])
    })
}

/// Download the installer at `url` (see [`safe_download`]) into a folder of its own in the
/// temporary folder: where it was saved.
pub fn download_installer(url: &str) -> Result<std::path::PathBuf, String> {
    if !safe_download(url) {
        return Err("that installer isn't one of this app's releases".into());
    }
    let file = url.rsplit('/').next().filter(|f| !f.is_empty()).ok_or("no file name")?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(600)))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(os_roots()?).build())
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("User-Agent", concat!("PdfCraft/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("couldn't download the new version ({e})"))?;
    let bytes =
        response.body_mut().with_config().limit(MAX_INSTALLER).read_to_vec().map_err(|e| format!("couldn't download the new version ({e})"))?;
    // An MSI is an OLE compound file: anything else (an error page) is not run.
    if !bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Err("the download isn't an installer".into());
    }
    let dir = std::env::temp_dir().join("TRK PDF Flow update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(file);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Run the installer, and start this app again once it has finished. The app closes right after,
/// so the installer can replace its files.
#[cfg(windows)]
pub fn run_installer(msi: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let quoted = |p: &std::path::Path| format!(r#""{}""#, p.display());
    // `start /wait` waits for the installer's window; then the (new) app starts again.
    let line = format!(r#"start "" /wait msiexec.exe /i {} & start "" {}"#, quoted(msi), quoted(&exe));
    std::process::Command::new("cmd.exe")
        .arg("/C")
        .raw_arg(line)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
pub fn run_installer(_msi: &std::path::Path) -> Result<(), String> {
    Err("installing updates from the app works on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_read_and_only_our_release_pages_are_offered() {
        let r = parse(r#"{"tag_name":"v0.2.0","html_url":"https://github.com/YamYam-ee/trk-pdf-flow/releases/tag/v0.2.0"}"#).unwrap();
        assert_eq!(
            r,
            Release { version: "v0.2.0".into(), url: "https://github.com/YamYam-ee/trk-pdf-flow/releases/tag/v0.2.0".into(), installer: None }
        );
        for elsewhere in ["https://example.com/pdfcraft.exe", "https://github.com/YamYam-ee/trk-pdf-flow/releases.evil/x", "javascript:alert(1)"] {
            let r = parse(&format!(r#"{{"tag_name":"v9.9.9","html_url":"{elsewhere}"}}"#)).unwrap();
            assert_eq!(r.url, RELEASES_PAGE, "{elsewhere}");
        }
        assert!(parse(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn only_this_repositorys_installers_are_offered() {
        let body = |url: &str| {
            format!(
                r#"{{"tag_name":"v1.0.2","html_url":"https://github.com/YamYam-ee/trk-pdf-flow/releases/tag/v1.0.2","assets":[{{"name":"notes.txt","browser_download_url":"https://github.com/YamYam-ee/trk-pdf-flow/releases/download/v1.0.2/notes.txt"}},{{"name":"TRK-PDF-Flow-1.0.2-x64.msi","browser_download_url":"{url}"}}]}}"#
            )
        };
        let ok = "https://github.com/YamYam-ee/trk-pdf-flow/releases/download/v1.0.2/TRK-PDF-Flow-1.0.2-x64.msi";
        assert_eq!(parse(&body(ok)).unwrap().installer.as_deref(), Some(ok));
        for bad in [
            "https://example.com/TRK-PDF-Flow.msi",
            "https://github.com/someone/else/releases/download/v1/x.msi",
            "https://github.com/YamYam-ee/trk-pdf-flow/releases/download/v1.0.2/../../x.msi",
            "https://github.com/YamYam-ee/trk-pdf-flow/releases/download/v1.0.2/x.msi?redirect=evil",
            "http://github.com/YamYam-ee/trk-pdf-flow/releases/download/v1.0.2/x.msi",
        ] {
            assert_eq!(parse(&body(bad)).unwrap().installer, None, "{bad}");
        }
        assert!(download_installer("https://example.com/x.msi").is_err(), "never fetched");
    }

    #[test]
    fn rate_limit_messages_name_the_wait() {
        let limited = "GitHub is limiting update checks for now (shared hourly quota); try again in about 20 minutes";
        assert_eq!(rate_limited(Some("1800001200"), 1800000000), limited);
        // Rounds up: 61 seconds out reads as 2 minutes.
        assert_eq!(
            rate_limited(Some("1800000061"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 2 minutes"
        );
        assert_eq!(
            rate_limited(Some("1800000060"), 1800000000),
            "GitHub is limiting update checks for now (shared hourly quota); try again in about 1 minute"
        );
        // No header, not a number, or already past: ask for patience instead of a time.
        let later = "GitHub is limiting update checks for now (shared hourly quota); try again later";
        for bad in [None, Some(""), Some("soon"), Some("1799999999"), Some("1800000000")] {
            assert_eq!(rate_limited(bad, 1800000000), later);
        }
    }

    /// Live: asks GitHub over TLS with the OS's roots (`cargo test -p pdfcraft -- --ignored`).
    #[test]
    #[ignore = "needs network access"]
    fn github_answers_with_the_latest_release() {
        let r = latest_release().unwrap();
        assert!(pdfcraft_ui_egui::updates::is_newer(&r.version, "0.0.0"), "{r:?}");
        assert!(r.url.starts_with(RELEASES_PAGE), "{r:?}");
    }
}
