use std::path::Path;

use lopdf::{Document, Object};

use crate::pdf::{file_label, file_stem, type_is, validate_pdf_path};

/// How a document should be sliced up.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SplitMode {
    /// One output file per comma-separated range, e.g. `"1-3, 5, 8-10"`.
    Ranges { ranges: String },
    /// One output file per chunk of `size` consecutive pages.
    EveryN { size: u32 },
}

/// An inclusive, 1-based page span.
type Span = (u32, u32);

/// Split `input` into several PDFs under `output_dir`, one per resolved span.
///
/// Each output is a clone of the source with the out-of-span pages deleted, so
/// fonts, images and annotations referenced by the kept pages survive intact.
/// Returns the written paths, in span order. Existing files are overwritten:
/// names are derived from the page numbers, so re-running is idempotent.
pub fn split_pdf(input: &str, mode: &SplitMode, output_dir: &str) -> Result<Vec<String>, String> {
    validate_pdf_path(input)?;
    if !Path::new(output_dir).is_dir() {
        return Err("Output folder not found.".into());
    }

    let doc = Document::load(input).map_err(|e| format!("Failed to read {}: {e}", file_label(input)))?;
    let total = doc.get_pages().len() as u32;
    if total == 0 {
        return Err(format!("{} has no pages.", file_label(input)));
    }

    let spans = resolve_spans(mode, total)?;
    let stem = file_stem(input);

    spans
        .iter()
        .map(|span| write_span(&doc, *span, total, output_dir, &stem))
        .collect()
}

/// Turn a mode into concrete page spans, validated against the page count.
fn resolve_spans(mode: &SplitMode, total: u32) -> Result<Vec<Span>, String> {
    match mode {
        SplitMode::Ranges { ranges } => parse_ranges(ranges, total),
        SplitMode::EveryN { size } => {
            if *size == 0 {
                return Err("Pages per file must be at least 1.".into());
            }
            Ok((1..=total)
                .step_by(*size as usize)
                .map(|start| (start, (start + size - 1).min(total)))
                .collect())
        }
    }
}

/// Parse `"1-3, 5, 8-10"` into inclusive spans, rejecting anything out of bounds.
fn parse_ranges(input: &str, total: u32) -> Result<Vec<Span>, String> {
    let spans: Vec<Span> = input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| parse_span(part, total))
        .collect::<Result<_, _>>()?;

    if spans.is_empty() {
        return Err("Enter at least one page range, e.g. 1-3, 5.".into());
    }
    Ok(spans)
}

/// Parse a single `"5"` or `"8-10"` token.
fn parse_span(part: &str, total: u32) -> Result<Span, String> {
    let (start, end) = match part.split_once('-') {
        Some((a, b)) => (parse_page(a, total)?, parse_page(b, total)?),
        None => {
            let page = parse_page(part, total)?;
            (page, page)
        }
    };
    if start > end {
        return Err(format!("Range {part} runs backwards."));
    }
    Ok((start, end))
}

fn parse_page(token: &str, total: u32) -> Result<u32, String> {
    let page: u32 = token
        .trim()
        .parse()
        .map_err(|_| format!("\"{}\" is not a page number.", token.trim()))?;
    if page < 1 || page > total {
        return Err(format!("Page {page} is out of range — this PDF has {total}."));
    }
    Ok(page)
}

/// Write one slice of `doc` covering `span`, and return the path written.
fn write_span(
    doc: &Document,
    span: Span,
    total: u32,
    output_dir: &str,
    stem: &str,
) -> Result<String, String> {
    let (start, end) = span;
    let drop: Vec<u32> = (1..=total).filter(|p| *p < start || *p > end).collect();

    let mut slice = doc.clone();
    slice.delete_pages(&drop);
    prune_dangling_kids(&mut slice);
    slice.prune_objects();
    slice.renumber_objects();
    slice.compress();

    let name = if start == end {
        format!("{stem}-{start}.pdf")
    } else {
        format!("{stem}-{start}-{end}.pdf")
    };
    let path = Path::new(output_dir).join(&name);
    slice
        .save(&path)
        .map_err(|e| format!("Failed to write {name}: {e}"))?;

    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Output path for {name} is not valid text."))
}

/// Drop `Kids` entries pointing at objects that no longer exist.
///
/// `Document::delete_pages` removes the page objects and fixes `Count`, but
/// leaves the references behind. Readers tolerate them as nulls; stripping them
/// keeps the page tree honest and lets `prune_objects` reclaim the rest.
fn prune_dangling_kids(doc: &mut Document) {
    let live: Vec<_> = doc.objects.keys().copied().collect();
    let page_trees: Vec<_> = doc
        .objects
        .iter()
        .filter(|(_, object)| type_is(object, b"Pages"))
        .map(|(id, _)| *id)
        .collect();

    for tree_id in page_trees {
        let Some(kids) = doc
            .objects
            .get(&tree_id)
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Kids").ok())
            .and_then(|k| k.as_array().ok())
        else {
            continue;
        };

        let kept: Vec<Object> = kids
            .iter()
            .filter(|kid| kid.as_reference().map(|id| live.contains(&id)).unwrap_or(true))
            .cloned()
            .collect();

        if kept.len() != kids.len() {
            if let Some(dict) = doc.objects.get_mut(&tree_id).and_then(|o| o.as_dict_mut().ok()) {
                dict.set("Kids", kept);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::test_support::write_blank_pdf;

    /// A fresh temp dir holding a `source.pdf` with `pages` blank pages.
    fn fixture(name: &str, pages: usize) -> (String, String) {
        let dir = std::env::temp_dir().join(format!("paperblade_split_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let input = dir.join("source.pdf");
        let input = input.to_str().unwrap().to_string();
        write_blank_pdf(&input, pages);
        (input, dir.to_str().unwrap().to_string())
    }

    fn page_count(path: &str) -> usize {
        Document::load(path).expect("load output pdf").get_pages().len()
    }

    #[test]
    fn ranges_produce_one_file_per_span() {
        let (input, dir) = fixture("ranges", 10);
        let mode = SplitMode::Ranges {
            ranges: "1-3, 5, 8-10".into(),
        };

        let outputs = split_pdf(&input, &mode, &dir).expect("split should succeed");

        assert_eq!(outputs.len(), 3);
        assert!(outputs[0].ends_with("source-1-3.pdf"), "got {}", outputs[0]);
        assert!(outputs[1].ends_with("source-5.pdf"), "got {}", outputs[1]);
        assert!(outputs[2].ends_with("source-8-10.pdf"), "got {}", outputs[2]);
        assert_eq!(page_count(&outputs[0]), 3);
        assert_eq!(page_count(&outputs[1]), 1);
        assert_eq!(page_count(&outputs[2]), 3);
    }

    #[test]
    fn every_n_covers_every_page_with_a_short_final_chunk() {
        let (input, dir) = fixture("every_n", 10);
        let mode = SplitMode::EveryN { size: 4 };

        let outputs = split_pdf(&input, &mode, &dir).expect("split should succeed");

        assert_eq!(outputs.len(), 3);
        let counts: Vec<usize> = outputs.iter().map(|p| page_count(p)).collect();
        assert_eq!(counts, vec![4, 4, 2], "chunks should tile the document");
    }

    #[test]
    fn every_n_larger_than_document_yields_one_full_copy() {
        let (input, dir) = fixture("every_n_big", 3);
        let mode = SplitMode::EveryN { size: 10 };

        let outputs = split_pdf(&input, &mode, &dir).expect("split should succeed");

        assert_eq!(outputs.len(), 1);
        assert_eq!(page_count(&outputs[0]), 3);
    }

    #[test]
    fn rejects_page_beyond_the_document() {
        let err = parse_ranges("1-3, 42", 10).unwrap_err();
        assert!(err.contains("out of range"), "got {err}");
    }

    #[test]
    fn rejects_backwards_range() {
        let err = parse_ranges("7-2", 10).unwrap_err();
        assert!(err.contains("backwards"), "got {err}");
    }

    #[test]
    fn rejects_non_numeric_range() {
        let err = parse_ranges("1-three", 10).unwrap_err();
        assert!(err.contains("not a page number"), "got {err}");
    }

    #[test]
    fn rejects_empty_ranges() {
        let err = parse_ranges("  ,  ", 10).unwrap_err();
        assert!(err.contains("at least one page range"), "got {err}");
    }

    #[test]
    fn rejects_zero_chunk_size() {
        let err = resolve_spans(&SplitMode::EveryN { size: 0 }, 10).unwrap_err();
        assert!(err.contains("at least 1"), "got {err}");
    }
}
