//! Windows' own text recognizer (`Windows.Media.Ocr`), which reads the languages installed in
//! Windows (Swedish with å, ä and ö, for one) where the bundled models only know the English
//! alphabet. It is driven through Windows PowerShell, which every Windows has, so there is no
//! Win32 or WinRT call here and no `unsafe`.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::sync::OnceLock;

use crate::ocr::{Line, Word};

/// No console window flashes up while PowerShell runs.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PRELUDE: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type -AssemblyName System.Runtime.WindowsRuntime
[void][Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
"#;

const LANGUAGES: &str = r#"
foreach ($l in [Windows.Media.Ocr.OcrEngine]::AvailableRecognizerLanguages) { $l.LanguageTag + "`t" + $l.DisplayName }
"#;

const RECOGNIZE: &str = r#"
[void][Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
[void][Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics, ContentType = WindowsRuntime]
[void][Windows.Globalization.Language, Windows.Globalization, ContentType = WindowsRuntime]
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
  $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, [Type]$type) {
  $task = $asTask.MakeGenericMethod($type).Invoke($null, @($op))
  [void]$task.Wait(-1)
  $task.Result
}
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($env:PDFCRAFT_OCR_IMAGE)) ([Windows.Storage.StorageFile])
$stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage((New-Object Windows.Globalization.Language($env:PDFCRAFT_OCR_LANGUAGE)))
if ($null -eq $engine) { throw ('Windows has no text recognition for ' + $env:PDFCRAFT_OCR_LANGUAGE) }
$result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
$c = [Globalization.CultureInfo]::InvariantCulture
foreach ($line in $result.Lines) {
  foreach ($w in $line.Words) {
    $r = $w.BoundingRect
    $r.X.ToString($c) + "`t" + $r.Y.ToString($c) + "`t" + $r.Width.ToString($c) + "`t" + $r.Height.ToString($c) + "`t" + $w.Text
  }
  ''
}
$stream.Dispose()
"#;

/// Run `script` (after the prelude) in Windows PowerShell with `env`, returning its output.
fn powershell(dir: &Path, script: &str, env: &[(&str, &str)]) -> Result<String, String> {
    let path = dir.join("pdfcraft-ocr.ps1");
    // A BOM tells Windows PowerShell the script is UTF-8.
    let text = format!("\u{feff}{PRELUDE}{script}");
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let exe = Path::new(&root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let mut c = std::process::Command::new(exe);
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]).arg(&path).creation_flags(CREATE_NO_WINDOW);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().map_err(|e| format!("Windows text recognition couldn't start: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Windows text recognition failed").to_string())
    }
}

/// A folder of its own in the user's temp folder, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Result<Scratch, String> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pdfcraft-winocr-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The languages Windows can read (its language tag and name), as installed in Windows.
pub fn languages() -> &'static [(String, String)] {
    static LIST: OnceLock<Vec<(String, String)>> = OnceLock::new();
    LIST.get_or_init(|| Scratch::new().and_then(|dir| powershell(&dir.0, LANGUAGES, &[])).map(|out| parse_languages(&out)).unwrap_or_default())
}

pub(crate) fn parse_languages(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.trim_end_matches('\r').split_once('\t'))
        .filter(|(tag, _)| !tag.trim().is_empty())
        .map(|(tag, name)| (tag.trim().to_string(), name.trim().to_string()))
        .collect()
}

/// The words Windows reads in a rendered page (premultiplied RGBA), in `language` (a tag
/// from [`languages`]): lines of words with boxes in image pixels.
pub fn recognize(rgba: &[u8], width: u32, height: u32, language: &str) -> Result<Vec<Line>, String> {
    if !language.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || language.is_empty() {
        return Err("not a language tag".into());
    }
    let png = crate::export::encode_png(width, height, rgba)?;
    let dir = Scratch::new()?;
    let image = dir.0.join("page.png");
    std::fs::write(&image, png).map_err(|e| e.to_string())?;
    let image = image.to_string_lossy().into_owned();
    let out = powershell(&dir.0, RECOGNIZE, &[("PDFCRAFT_OCR_IMAGE", &image), ("PDFCRAFT_OCR_LANGUAGE", language)])?;
    Ok(parse_words(&out))
}

/// Lines of `x\ty\twidth\theight\ttext` words, each line ended by an empty line.
pub(crate) fn parse_words(out: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut words = Vec::new();
    for l in out.lines().map(|l| l.trim_end_matches('\r')) {
        if l.trim().is_empty() {
            if !words.is_empty() {
                lines.push(Line { words: std::mem::take(&mut words) });
            }
            continue;
        }
        let mut f = l.splitn(5, '\t');
        let mut num = || f.next().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
        let (Some(x), Some(y), Some(w), Some(h)) = (num(), num(), num(), num()) else { continue };
        let Some(text) = f.next().map(str::trim).filter(|t| !t.is_empty()) else { continue };
        if w > 0.0 && h > 0.0 {
            words.push(Word { text: text.to_string(), rect: [x, y, x + w, y + h] });
        }
    }
    if !words.is_empty() {
        lines.push(Line { words });
    }
    lines
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_output_reads_as_lines_of_words() {
        let out = "10\t20\t30\t12\tFöremål\r\n45.5\t20\t20\t12\tår\r\n\r\n10\t40\t25\t12\tÖvrigt\r\nbad line\r\n\r\n";
        let lines = super::parse_words(out);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Föremål", "år"]);
        assert_eq!(lines[0].words[1].rect, [45.5, 20.0, 65.5, 32.0]);
        assert_eq!(lines[1].text(), "Övrigt");
        assert_eq!(
            super::parse_languages("sv-SE\tSvenska (Sverige)\r\nen-US\tEnglish\r\n"),
            [("sv-SE".to_string(), "Svenska (Sverige)".to_string()), ("en-US".to_string(), "English".to_string())]
        );
    }

    /// On this machine, if Windows can read Swedish: a rendered "Föremål på år" comes back with
    /// its letters.
    #[test]
    fn windows_reads_swedish_letters_when_installed() {
        if !super::languages().iter().any(|(tag, _)| tag.starts_with("sv")) {
            eprintln!("skipped: Windows has no Swedish text recognition here");
            return;
        }
        let mut s = crate::Session::new().with_clock(|| 1_700_000_000);
        let pdf = s.create_from_text("t", "Föremål på år och dag").unwrap();
        let id = s.open("t.pdf", None, pdf, None).unwrap();
        let r = crate::export::Exporter::new(s.get(id).unwrap());
        let mut r = r;
        let png = r.png(0, 200.0).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        let lines = super::recognize(img.as_raw(), img.width(), img.height(), "sv-SE").unwrap();
        let text: String = lines.iter().map(|l| l.text()).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Föremål") && text.contains("år"), "{text}");
    }
}
