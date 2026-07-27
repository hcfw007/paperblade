use std::path::{Path, PathBuf};
use std::process::Command;

use crate::pdf::{file_label, validate_pdf_path};

/// Quality presets, mapped onto Ghostscript's `-dPDFSETTINGS` values.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// 72 dpi images. Smallest, fine for reading on a screen.
    Screen,
    /// 150 dpi images. A reasonable default.
    Ebook,
    /// 300 dpi images. Keeps enough detail to print.
    Printer,
}

impl Quality {
    fn as_setting(self) -> &'static str {
        match self {
            Quality::Screen => "/screen",
            Quality::Ebook => "/ebook",
            Quality::Printer => "/printer",
        }
    }
}

/// What compressing actually achieved, so the UI can be honest about it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub output: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    /// True when the "compressed" file came out larger than the original.
    /// Ghostscript re-encodes unconditionally, so a PDF whose images are
    /// already below the preset's dpi can easily grow.
    pub grew: bool,
}

/// The Ghostscript invocation for one compression run.
///
/// Split out from the process call so the argument list is testable without a
/// Ghostscript binary present.
fn gs_args(input: &str, output: &str, quality: Quality) -> Vec<String> {
    vec![
        "-sDEVICE=pdfwrite".into(),
        "-dCompatibilityLevel=1.4".into(),
        format!("-dPDFSETTINGS={}", quality.as_setting()),
        // Non-interactive: never prompt, never wait for input, exit when done.
        "-dNOPAUSE".into(),
        "-dBATCH".into(),
        "-dQUIET".into(),
        // Refuse the PostScript operators that let a document touch the
        // filesystem or spawn commands. Default since 9.50, set explicitly
        // because it is the one flag that must never be dropped.
        "-dSAFER".into(),
        format!("-sOutputFile={output}"),
        input.into(),
    ]
}

/// Shrink `input` into `output` using the bundled Ghostscript at `gs`.
pub fn compress_pdf(gs: &Path, input: &str, output: &str, quality: Quality) -> Result<Report, String> {
    validate_pdf_path(input)?;
    if Path::new(output) == Path::new(input) {
        return Err("Choose a different file for the output.".into());
    }

    let before_bytes = std::fs::metadata(input)
        .map_err(|e| format!("Failed to read {}: {e}", file_label(input)))?
        .len();

    let result = Command::new(gs)
        .args(gs_args(input, output, quality))
        .output()
        .map_err(|e| format!("Could not run the compression engine: {e}"))?;

    if !result.status.success() {
        // Ghostscript reports the real reason on stderr; the exit code alone
        // says nothing useful. A failed run can still leave a partial file.
        let _ = std::fs::remove_file(output);
        return Err(engine_error(&result.stderr));
    }

    let after_bytes = std::fs::metadata(output)
        .map_err(|_| "The compression engine reported success but wrote no file.".to_string())?
        .len();

    Ok(Report {
        output: output.to_string(),
        before_bytes,
        after_bytes,
        grew: after_bytes >= before_bytes,
    })
}

/// Turn Ghostscript's stderr into one line worth showing a user.
fn engine_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let detail = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("****"))
        .unwrap_or("");

    if detail.is_empty() {
        "Compression failed. The file may be damaged or password-protected.".into()
    } else {
        format!("Compression failed: {detail}")
    }
}

/// Locate the Ghostscript binary.
///
/// A packaged build ships one next to the executable as a Tauri sidecar; a dev
/// checkout falls back to whatever is on `PATH`. Returning the path rather than
/// a bare command keeps the spawn explicit about which binary ran.
pub fn find_ghostscript() -> Result<PathBuf, String> {
    if let Some(bundled) = bundled_ghostscript() {
        return Ok(bundled);
    }

    let found = Command::new("/usr/bin/which")
        .arg("gs")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|path| !path.is_empty());

    match found {
        Some(path) => Ok(PathBuf::from(path)),
        None => Err(
            "Ghostscript is not available. This build does not bundle it yet — \
             install it with `brew install ghostscript` to use compression."
                .into(),
        ),
    }
}

/// The sidecar that sits beside the app executable in a packaged build.
fn bundled_ghostscript() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidate = dir.join("gs");
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::test_support::write_blank_pdf;

    fn dir_for(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("paperblade_compress_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn always_passes_safer_and_a_non_interactive_flag_set() {
        let args = gs_args("in.pdf", "out.pdf", Quality::Ebook);

        // -dSAFER blocks the PostScript operators that reach the filesystem.
        // Losing it silently would turn any opened PDF into a code-execution
        // surface, so it is asserted rather than assumed.
        assert!(args.contains(&"-dSAFER".to_string()), "SAFER must be set");
        for flag in ["-dNOPAUSE", "-dBATCH"] {
            assert!(args.contains(&flag.to_string()), "{flag} must be set");
        }
    }

    #[test]
    fn maps_each_quality_onto_its_ghostscript_preset() {
        for (quality, expected) in [
            (Quality::Screen, "/screen"),
            (Quality::Ebook, "/ebook"),
            (Quality::Printer, "/printer"),
        ] {
            let args = gs_args("in.pdf", "out.pdf", quality);
            assert!(
                args.contains(&format!("-dPDFSETTINGS={expected}")),
                "{quality:?} should map to {expected}"
            );
        }
    }

    #[test]
    fn puts_the_input_last_and_the_output_in_a_flag() {
        let args = gs_args("/tmp/in.pdf", "/tmp/out.pdf", Quality::Screen);

        assert_eq!(args.last().unwrap(), "/tmp/in.pdf", "input goes last");
        assert!(args.contains(&"-sOutputFile=/tmp/out.pdf".to_string()));
    }

    #[test]
    fn refuses_to_overwrite_its_own_input() {
        let dir = dir_for("same_path");
        let path = dir.join("a.pdf");
        let path = path.to_str().unwrap();
        write_blank_pdf(path, 1);

        let err = compress_pdf(Path::new("/bin/true"), path, path, Quality::Ebook).unwrap_err();
        assert!(err.contains("different file"), "got {err}");
    }

    #[test]
    fn reports_a_readable_error_when_the_engine_fails() {
        let dir = dir_for("engine_fail");
        let input = dir.join("a.pdf");
        let output = dir.join("b.pdf");
        let (input, output) = (input.to_str().unwrap(), output.to_str().unwrap());
        write_blank_pdf(input, 1);

        // /usr/bin/false exits non-zero without writing anything.
        let err = compress_pdf(Path::new("/usr/bin/false"), input, output, Quality::Ebook).unwrap_err();

        assert!(err.starts_with("Compression failed"), "got {err}");
        assert!(
            !Path::new(output).exists(),
            "a failed run must not leave a partial file behind"
        );
    }

    #[test]
    fn surfaces_the_first_meaningful_stderr_line() {
        assert!(engine_error(b"").contains("may be damaged"));
        assert_eq!(
            engine_error(b"**** banner\n\n   Error: /undefinedfilename\n"),
            "Compression failed: Error: /undefinedfilename"
        );
    }
}
