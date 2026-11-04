//! Where pages dragged out of the window were let go: the folder of the Explorer window under the
//! pointer (Windows), so the save dialog starts there. The desktop (or anything else) gives
//! `None`, and the dialog starts on the Desktop as before.
//!
//! Windows PowerShell answers it (the pointer, the window under it, and the Shell's list of open
//! folder windows), so there is no Win32 call here and no `unsafe`.

#[cfg(windows)]
const SCRIPT: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class TrkDropPoint {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
}
"@
$p = New-Object TrkDropPoint+POINT
[void][TrkDropPoint]::GetCursorPos([ref]$p)
$root = [TrkDropPoint]::GetAncestor([TrkDropPoint]::WindowFromPoint($p), 2).ToInt64()
foreach ($w in (New-Object -ComObject Shell.Application).Windows()) {
  try { if ([int64]$w.HWND -eq $root) { $w.Document.Folder.Self.Path; break } } catch {}
}
"#;

/// The folder of the Explorer window under the pointer, if it is one.
#[cfg(windows)]
pub fn folder_under_pointer() -> Option<std::path::PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let bytes: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64_encode(&bytes);
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let exe = std::path::Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let out = std::process::Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let path = std::path::PathBuf::from(text.lines().map(str::trim).find(|l| !l.is_empty())?);
    path.is_dir().then_some(path)
}

#[cfg(not(windows))]
pub fn folder_under_pointer() -> Option<std::path::PathBuf> {
    None
}

/// Standard base64 (PowerShell's `-EncodedCommand` takes the script as base64 of UTF-16LE).
#[cfg(windows)]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { char::from(TABLE[((n >> shift) & 63) as usize]) } else { '=' });
        }
    }
    out
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(super::base64_encode(b"ab"), "YWI=");
        assert_eq!(super::base64_encode(b"abc"), "YWJj");
        assert_eq!(super::base64_encode(b"a"), "YQ==");
    }
}
