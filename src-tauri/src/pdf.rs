//! Small helpers shared by the PDF tools: path validation, user-facing file
//! labels, and page-tree object probing.

use std::path::Path;

use lopdf::{Document, Object};

/// How many pages `path` holds. Used by tool pages to frame their page inputs.
pub fn page_count(path: &str) -> Result<u32, String> {
    validate_pdf_path(path)?;
    let doc = Document::load(path).map_err(|e| format!("Failed to read {}: {e}", file_label(path)))?;
    Ok(doc.get_pages().len() as u32)
}

/// Reject anything that is not an existing file with a `.pdf` extension.
///
/// Paths arrive from the system dialog, but the frontend is never a trust
/// boundary — every command re-checks on the Rust side.
pub fn validate_pdf_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(format!("File not found: {}", file_label(path)));
    }
    let is_pdf = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    if !is_pdf {
        return Err(format!("Not a PDF: {}", file_label(path)));
    }
    Ok(())
}

/// The file name alone, for error messages that should not leak full paths.
pub fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// The file name without its extension, used to name derived outputs.
pub fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("output")
        .to_string()
}

/// True when the object is a dictionary whose `/Type` matches `name`.
pub fn type_is(object: &Object, name: &[u8]) -> bool {
    object
        .as_dict()
        .ok()
        .and_then(|d| d.get(b"Type").ok())
        .and_then(|t| t.as_name().ok())
        .map(|n| n == name)
        .unwrap_or(false)
}

#[cfg(test)]
pub mod test_support {
    use lopdf::{dictionary, Document, Object};

    /// Write a minimal valid PDF with `page_count` blank Letter-sized pages.
    pub fn write_blank_pdf(path: &str, page_count: usize) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();

        let kids: Vec<Object> = (0..page_count)
            .map(|_| {
                Object::Reference(doc.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![
                        Object::Integer(0),
                        Object::Integer(0),
                        Object::Integer(612),
                        Object::Integer(792),
                    ],
                }))
            })
            .collect();

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => page_count as i64,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).expect("write fixture pdf");
    }
}
