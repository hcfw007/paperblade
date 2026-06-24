use std::collections::BTreeMap;
use std::path::Path;

use lopdf::{Document, Object, ObjectId};

/// Merge several PDFs, in the given order, into a single output file.
///
/// Each source document is renumbered into a disjoint id range, every page is
/// re-parented onto one shared `Pages` node, and the catalog is rebuilt to
/// point at it. Page order follows the input order, then page order within each
/// file. Bookmarks/outlines are intentionally dropped for now.
pub fn merge_pdfs(inputs: &[String], output: &str) -> Result<(), String> {
    if inputs.len() < 2 {
        return Err("Select at least two PDF files to merge.".into());
    }

    let mut max_id = 1u32;
    // Pages kept in a Vec to preserve input + page order; objects pooled by id.
    let mut documents_pages: Vec<(ObjectId, Object)> = Vec::new();
    let mut documents_objects: BTreeMap<ObjectId, Object> = BTreeMap::new();

    for path in inputs {
        validate_pdf_path(path)?;
        let mut doc = Document::load(path)
            .map_err(|e| format!("Failed to read {}: {e}", file_label(path)))?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;

        for (_, object_id) in doc.get_pages() {
            let object = doc
                .get_object(object_id)
                .map_err(|e| format!("Corrupt page in {}: {e}", file_label(path)))?
                .to_owned();
            documents_pages.push((object_id, object));
        }
        documents_objects.extend(doc.objects);
    }

    assemble(documents_pages, documents_objects, output)
}

/// Build the merged document from the pooled pages/objects and write it out.
fn assemble(
    documents_pages: Vec<(ObjectId, Object)>,
    documents_objects: BTreeMap<ObjectId, Object>,
    output: &str,
) -> Result<(), String> {
    let mut merged = Document::with_version("1.5");

    // Find the first Catalog and Pages node; copy every other object verbatim.
    let mut catalog: Option<(ObjectId, Object)> = None;
    let mut pages: Option<(ObjectId, Object)> = None;

    for (object_id, object) in &documents_objects {
        if type_is(object, b"Catalog") {
            catalog.get_or_insert_with(|| (*object_id, object.clone()));
        } else if type_is(object, b"Pages") {
            pages.get_or_insert_with(|| (*object_id, object.clone()));
        } else if !type_is(object, b"Page") {
            merged.objects.insert(*object_id, object.clone());
        }
    }

    let (catalog_id, catalog_obj) = catalog.ok_or("No PDF catalog found in inputs.")?;
    let (pages_id, pages_obj) = pages.ok_or("No page tree found in inputs.")?;

    // Re-parent every page onto the single merged Pages node.
    let kids: Vec<Object> = documents_pages.iter().map(|(id, _)| Object::Reference(*id)).collect();
    let page_count = documents_pages.len() as u32;
    for (object_id, object) in &documents_pages {
        if let Ok(dict) = object.as_dict() {
            let mut dict = dict.clone();
            dict.set("Parent", pages_id);
            merged.objects.insert(*object_id, Object::Dictionary(dict));
        }
    }

    // Rebuild the Pages node with the merged kids/count.
    if let Ok(dict) = pages_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Count", page_count);
        dict.set("Kids", kids);
        merged.objects.insert(pages_id, Object::Dictionary(dict));
    }

    // Rebuild the Catalog: point at our Pages, drop stale outlines.
    if let Ok(dict) = catalog_obj.as_dict() {
        let mut dict = dict.clone();
        dict.set("Pages", pages_id);
        dict.remove(b"Outlines");
        merged.objects.insert(catalog_id, Object::Dictionary(dict));
    }

    merged.trailer.set("Root", catalog_id);
    merged.max_id = merged.objects.len() as u32;
    merged.renumber_objects();
    merged.compress();
    merged
        .save(output)
        .map_err(|e| format!("Failed to write output: {e}"))?;
    Ok(())
}

/// True when the object is a dictionary whose `/Type` matches `name`.
fn type_is(object: &Object, name: &[u8]) -> bool {
    object
        .as_dict()
        .ok()
        .and_then(|d| d.get(b"Type").ok())
        .and_then(|t| t.as_name().ok())
        .map(|n| n == name)
        .unwrap_or(false)
}

fn validate_pdf_path(path: &str) -> Result<(), String> {
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

fn file_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object};

    /// Write a minimal valid single-page PDF to `path`.
    fn write_one_page_pdf(path: &str) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).expect("write fixture pdf");
    }

    #[test]
    fn merges_two_single_page_pdfs_into_two_pages() {
        let dir = std::env::temp_dir();
        let a = dir.join("paperblade_test_a.pdf");
        let b = dir.join("paperblade_test_b.pdf");
        let out = dir.join("paperblade_test_merged.pdf");
        let (a, b, out) = (
            a.to_str().unwrap().to_string(),
            b.to_str().unwrap().to_string(),
            out.to_str().unwrap().to_string(),
        );

        write_one_page_pdf(&a);
        write_one_page_pdf(&b);

        merge_pdfs(&[a, b], &out).expect("merge should succeed");

        let merged = Document::load(&out).expect("load merged pdf");
        assert_eq!(merged.get_pages().len(), 2, "merged pdf should have 2 pages");
    }

    #[test]
    fn rejects_single_input() {
        let err = merge_pdfs(&["only.pdf".into()], "out.pdf").unwrap_err();
        assert!(err.contains("at least two"));
    }
}
