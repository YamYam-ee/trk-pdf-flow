//! Show a saved file in its folder in the system's file manager (the "saved" notice's button),
//! with the file selected so it is found at once.

use std::path::Path;
use std::process::Command;

/// Open the folder of `path` with the file selected where the file manager can (Explorer,
/// Finder); elsewhere the folder itself. Failures are only logged: nothing was lost.
pub fn reveal(path: &Path) {
    if let Err(e) = spawn(path) {
        log::warn!("couldn't show {} in its folder: {e}", path.display());
    }
}

/// On Windows the Shell selects the file: in the folder's window if one is already open (where
/// `explorer /select` only brings the window forward and selects nothing), otherwise in a new
/// one. The path goes in the environment, so no quoting can go wrong.
#[cfg(windows)]
const SCRIPT: &str = r#"
$p = $env:TRK_REVEAL_PATH
if (-not $p) { exit }
$dir = [IO.Path]::GetDirectoryName($p)
$name = [IO.Path]::GetFileName($p)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class TrkReveal {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
}
"@
function Find-Window {
  foreach ($w in (New-Object -ComObject Shell.Application).Windows()) {
    try { if ($w.Document.Folder.Self.Path -eq $dir) { return $w } } catch {}
  }
  return $null
}
$w = Find-Window
if (-not $w) {
  if (Test-Path -LiteralPath $p) { Start-Process explorer.exe -ArgumentList ('/select,"' + $p + '"') }
  else { Start-Process explorer.exe -ArgumentList ('"' + $dir + '"') }
  # The new window selects it itself; make sure once it is up (a slow or reused window).
  for ($i = 0; $i -lt 20 -and -not $w; $i++) { Start-Sleep -Milliseconds 150; $w = Find-Window }
}
if ($w) {
  for ($i = 0; $i -lt 10; $i++) {
    try {
      $item = $w.Document.Folder.ParseName($name)
      if ($item) { $w.Document.SelectItem($item, 29); break }
    } catch {}
    try { $w.Refresh() } catch {}
    Start-Sleep -Milliseconds 150
  }
  $h = [IntPtr]$w.HWND
  if ([TrkReveal]::IsIconic($h)) { [void][TrkReveal]::ShowWindow($h, 9) }
  [void][TrkReveal]::SetForegroundWindow($h)
}
"#;

#[cfg(windows)]
fn spawn(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Explorer and the Shell want an ordinary absolute path with backslashes.
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = full.to_string_lossy().replace('/', "\\");
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text).to_string();
    let bytes: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let exe = Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    Command::new(exe)
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &crate::drop_folder::base64_encode(&bytes)])
        .env("TRK_REVEAL_PATH", text)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
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
