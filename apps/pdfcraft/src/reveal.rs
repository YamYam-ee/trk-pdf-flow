//! Show a saved file in its folder in the system's file manager (the "saved" notice's button).

use std::path::Path;
use std::process::Command;

/// Open the folder of `path` with the file selected where the file manager can (Explorer,
/// Finder); elsewhere the folder itself. Failures are only logged: nothing was lost.
pub fn reveal(path: &Path) {
    let result = spawn(path);
    if let Err(e) = result {
        log::warn!("couldn't show {} in its folder: {e}", path.display());
    }
}

#[cfg(windows)]
fn spawn(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // Explorer wants `/select,<path>` as it is, with its own quoting.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut c = Command::new(Path::new(&root).join("explorer.exe"));
    if path.exists() {
        c.raw_arg(format!("/select,\"{}\"", path.display()));
    } else if let Some(dir) = path.parent() {
        c.arg(dir);
    }
    c.spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn spawn(path: &Path) -> std::io::Result<()> {
    Command::new("open").arg("-R").arg(path).spawn().map(|_| ())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn spawn(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(path);
    Command::new("xdg-open").arg(dir).spawn().map(|_| ())
}
