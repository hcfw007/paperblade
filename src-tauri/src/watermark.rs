//! Text watermarks drawn on top of every page.
//!
//! The watermark is a fresh content stream appended to each page's
//! `/Contents`; the original page content bytes are never touched (research
//! doc `docs/research/m2-render-engine.md` §3.1). Text uses non-embedded
//! fonts: base-14 Helvetica for ASCII, Adobe's preset STSong-Light with the
//! UniGB-UCS2-H CMap for everything else.

use std::path::Path;

use lopdf::{dictionary, Dictionary, Document, Object, Stream};

use crate::pdf::{file_label, validate_pdf_path};

/// Watermark text longer than this is almost certainly a paste accident.
const MAX_TEXT_CHARS: usize = 200;

/// How much slack the CJK width estimate carries. See `cjk_width_em`.
const CJK_WIDTH_SAFETY: f32 = 1.25;

/// Where the watermark sits on the page.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Position {
    /// One stamp, centered on the page, at the requested angle.
    Center,
    /// One stamp corner to corner along the page diagonal (fixed at 45°).
    Diagonal,
    /// Repeated in a staggered grid across the whole page.
    Tiled,
}

/// Everything needed to stamp one string onto a document.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watermark {
    pub text: String,
    /// 0 exclusive, 1 inclusive; painted through an ExtGState.
    pub opacity: f32,
    /// Degrees counter-clockwise; `Position::Diagonal` pins this at 45°.
    pub rotation: f32,
    pub position: Position,
    /// Type size in points.
    pub font_size: f32,
    /// `#rrggbb`, like an HTML color picker hands out.
    pub color: String,
}

/// What stamping actually did, so the UI can be honest about the result.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub output: String,
    pub pages: u32,
    pub before_bytes: u64,
    pub after_bytes: u64,
}

impl Watermark {
    /// Bounds that keep the stamp sane. Every message is shown to a user.
    fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("Enter the watermark text.".into());
        }
        if self.text.chars().count() > MAX_TEXT_CHARS {
            return Err(format!("Keep the watermark under {MAX_TEXT_CHARS} characters."));
        }
        if !self.opacity.is_finite() || self.opacity <= 0.0 || self.opacity > 1.0 {
            return Err("Opacity must be above 0 and at most 1.".into());
        }
        if !self.font_size.is_finite() || !(1.0..=200.0).contains(&self.font_size) {
            return Err("Font size must be between 1 and 200 points.".into());
        }
        if !self.rotation.is_finite() || !(-360.0..=360.0).contains(&self.rotation) {
            return Err("Angle must be between -360 and 360 degrees.".into());
        }
        parse_hex_color(&self.color)?;
        Ok(())
    }
}

/// Which of the two non-embedded fonts draws the watermark.
#[derive(Debug, Clone, Copy, PartialEq)]
enum WmFont {
    Latin,
    Cjk,
}

/// A watermark string ready to be shown.
struct Encoded {
    font: WmFont,
    /// Single bytes for Latin, UTF-16BE units for the CJK path.
    bytes: Vec<u8>,
    /// Advance width in em units — multiply by the font size.
    width_em: f32,
}

/// Encode `text` for the PDF, picking the font path and estimating its width.
///
/// ASCII goes to Helvetica with exact AFM widths. Anything else (Chinese
/// included) goes to the whole-string CJK path: UniGB-UCS2-H is a UCS-2 CMap,
/// so mixed Latin/CJK lines stay copyable at the cost of losing Helvetica's
/// exact metrics for the Latin part.
fn encode_text(text: &str) -> Encoded {
    if text.is_ascii() {
        Encoded {
            font: WmFont::Latin,
            bytes: text.bytes().collect(),
            width_em: latin_width_em(text),
        }
    } else {
        let mut bytes = Vec::new();
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        Encoded {
            font: WmFont::Cjk,
            bytes,
            width_em: cjk_width_em(text),
        }
    }
}

/// Helvetica AFM advance widths, per 1000 units, for ASCII 32..=126.
const HELVETICA_WIDTHS: [f32; 95] = [
    278.0, 278.0, 355.0, 556.0, 556.0, 889.0, 667.0, 191.0, 333.0, 333.0, 389.0, 584.0, 278.0, 333.0, 278.0,
    278.0, 556.0, 556.0, 556.0, 556.0, 556.0, 556.0, 556.0, 556.0, 556.0, 556.0, 278.0, 278.0, 584.0, 584.0,
    584.0, 556.0, 1015.0, 667.0, 667.0, 722.0, 722.0, 667.0, 611.0, 778.0, 722.0, 278.0, 500.0, 667.0, 556.0,
    833.0, 722.0, 778.0, 667.0, 778.0, 722.0, 667.0, 611.0, 722.0, 667.0, 944.0, 667.0, 667.0, 611.0, 278.0,
    278.0, 278.0, 469.0, 556.0, 333.0, 556.0, 556.0, 500.0, 556.0, 556.0, 278.0, 556.0, 556.0, 222.0, 222.0,
    500.0, 222.0, 833.0, 556.0, 556.0, 556.0, 556.0, 333.0, 500.0, 278.0, 556.0, 500.0, 722.0, 500.0, 500.0,
    500.0, 334.0, 260.0, 334.0, 584.0,
];

/// Exact advance width for ASCII text under Helvetica, in em units.
fn latin_width_em(text: &str) -> f32 {
    text.bytes()
        .map(|byte| HELVETICA_WIDTHS[(byte - 32) as usize] / 1000.0)
        .sum()
}

/// Estimated advance width for the non-embedded CJK path, in em units.
///
/// The rendered width depends on which font each reader substitutes for
/// STSong-Light — nobody ships Adobe's original metrics. The precedent in
/// cap-doc's watermark.rs measured real lines at ~1.12× the plain estimate
/// and padded by 1.25 so centering and tiling never run the stamp off the
/// page (research doc §5-R3). If the four-reader regression finds this
/// padding unacceptable, the fix is the pdfium fallback with an embedded
/// font (§3.1 trigger condition) — not more tuning here.
fn cjk_width_em(text: &str) -> f32 {
    let estimate: f32 = text.chars().map(char_width_em).sum();
    estimate * CJK_WIDTH_SAFETY
}

/// One glyph's estimated width in em: full-width CJK scripts fill the em
/// square, everything else takes about half of it.
fn char_width_em(c: char) -> f32 {
    if is_full_width(c) {
        1.0
    } else {
        0.5
    }
}

/// Whether the character is drawn full-width in the substituted CJK font.
fn is_full_width(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{115F}' // Hangul jamo
        | '\u{2E80}'..='\u{303E}' // CJK radicals and punctuation
        | '\u{3041}'..='\u{33FF}' // kana, CJK compatibility
        | '\u{3400}'..='\u{4DBF}' // CJK extension A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified ideographs
        | '\u{F900}'..='\u{FAFF}' // CJK compatibility ideographs
        | '\u{FE30}'..='\u{FE4F}' // vertical forms
        | '\u{FF00}'..='\u{FF60}' // full-width forms
        | '\u{FFE0}'..='\u{FFE6}'
        | '\u{20000}'..='\u{2FA1F}' // CJK extensions B and beyond
    )
}

/// Accepts `#rrggbb` or `rrggbb`, like an HTML color picker produces.
fn parse_hex_color(input: &str) -> Result<(f32, f32, f32), String> {
    let hex = input.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("\"{input}\" is not a #rrggbb color."));
    }
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16).expect("checked hex digits") as f32 / 255.0
    };
    Ok((channel(0..2), channel(2..4), channel(4..6)))
}

/// Numbers for the content stream: three decimals at most, never "-0".
fn num(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == rounded.trunc() {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded:.3}").trim_end_matches('0').to_string()
    }
}

/// Where a page lives in user space, and how it is turned for display.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PageRect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    /// The page's `/Rotate` value, in degrees.
    rotate: f64,
}

impl PageRect {
    fn center(&self) -> (f64, f64) {
        ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
}

/// The page's real bounds and rotation.
///
/// CropBox wins over MediaBox, and both are inheritable attributes — on
/// plenty of real files they live on the `/Pages` node, and bleeding
/// documents do not start at (0, 0) (research doc §6 M2a).
fn page_rect(doc: &Document, page_id: (u32, u16)) -> Result<PageRect, String> {
    let box_of = |key: &[u8]| inherited_array(doc, page_id, key);
    let bounds = box_of(b"CropBox").or_else(|| box_of(b"MediaBox"));
    let Some(nums) = bounds.filter(|n| n.len() == 4) else {
        return Err("A page has no page size (MediaBox).".into());
    };
    Ok(PageRect {
        x0: nums[0].min(nums[2]),
        y0: nums[1].min(nums[3]),
        x1: nums[0].max(nums[2]),
        y1: nums[1].max(nums[3]),
        rotate: inherited_number(doc, page_id, b"Rotate").unwrap_or(0.0),
    })
}

/// Read an inheritable numeric-array page attribute, walking up the tree.
fn inherited_array(doc: &Document, page_id: (u32, u16), key: &[u8]) -> Option<Vec<f64>> {
    walk_ancestors(doc, page_id, |dict| {
        dict.get(key)
            .ok()
            .and_then(|object| object.as_array().ok())
            .and_then(|arr| {
                arr.iter()
                    .map(|o| o.as_float().ok())
                    .collect::<Option<Vec<f32>>>()
            })
            .map(|v| v.into_iter().map(f64::from).collect())
    })
}

/// Read an inheritable scalar page attribute, walking up the tree.
fn inherited_number(doc: &Document, page_id: (u32, u16), key: &[u8]) -> Option<f64> {
    walk_ancestors(doc, page_id, |dict| {
        dict.get(key).ok().and_then(|o| o.as_float().ok()).map(f64::from)
    })
}

/// Try `read` on the page dict, then on each `/Parent`, bounded against
/// cycles in a broken page tree.
fn walk_ancestors<T>(
    doc: &Document,
    page_id: (u32, u16),
    read: impl Fn(&Dictionary) -> Option<T>,
) -> Option<T> {
    let mut current = page_id;
    for _ in 0..32 {
        let dict = doc.get_dictionary(current).ok()?;
        if let Some(value) = read(dict) {
            return Some(value);
        }
        current = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Where each stamp's anchor sits on the page.
fn stamp_centers(rect: &PageRect, position: Position, width_pt: f64, size: f64) -> Vec<(f64, f64)> {
    match position {
        Position::Center | Position::Diagonal => vec![rect.center()],
        Position::Tiled => {
            // Rows offset by half a step so the stamps read as diagonal
            // bands once the rotation is applied.
            let step_x = (width_pt + size).max(size * 2.0);
            let step_y = size * 3.0;
            let mut points = Vec::new();
            let mut row = 0u32;
            let mut y = rect.y0 + step_y / 2.0;
            while y < rect.y1 {
                let offset = if row % 2 == 1 { step_x / 2.0 } else { 0.0 };
                let mut x = rect.x0 + step_x / 2.0 + offset;
                while x < rect.x1 {
                    points.push((x, y));
                    x += step_x;
                }
                y += step_y;
                row += 1;
            }
            points
        }
    }
}

/// The angle each stamp is drawn at.
///
/// Diagonal is a fixed 45° — it *is* the angle the position name promises,
/// which is why the UI pins the angle input in that mode. The page's own
/// `/Rotate` turns everything the reader draws, so it is subtracted to keep
/// the stamp upright on rotated pages.
fn stamp_angle(position: Position, rotation: f64, page_rotate: f64) -> f64 {
    let base = match position {
        Position::Diagonal => 45.0,
        Position::Center | Position::Tiled => rotation,
    };
    base - page_rotate
}

/// Text matrix placing the stamp so its visual center lands on `(cx, cy)`
/// and its baseline runs at `angle` degrees.
fn text_matrix(angle_deg: f64, cx: f64, cy: f64, width_pt: f64, size: f64) -> [f64; 6] {
    let rad = angle_deg.to_radians();
    let (sin, cos) = (rad.sin(), rad.cos());
    // Shift back along the baseline by half the width, and drop the anchor
    // 0.35em below the glyph box center.
    let tx = cx - (width_pt / 2.0) * cos + (0.35 * size) * sin;
    let ty = cy - (width_pt / 2.0) * sin - (0.35 * size) * cos;
    [cos, sin, -sin, cos, tx, ty]
}

/// One stamp's drawing parameters, reused for every placement on a page.
struct Stamp<'a> {
    font_name: &'a str,
    gs_name: &'a str,
    encoded: &'a Encoded,
    size: f64,
    color: (f32, f32, f32),
    angle: f64,
}

/// The content-stream fragment drawing one stamp. Wrapped in `q…Q` so no
/// text state or color leaks into whatever draws next.
fn stamp_ops(stamp: &Stamp, cx: f64, cy: f64) -> String {
    let (r, g, b) = stamp.color;
    let size = stamp.size;
    let width_pt = f64::from(stamp.encoded.width_em) * size;
    let m = text_matrix(stamp.angle, cx, cy, width_pt, size);
    let hex: String = stamp
        .encoded
        .bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    format!(
        "q\n/{} gs\n{r} {g} {b} rg\nBT\n/{} {size} Tf\n\
         {} {} {} {} {} {} Tm\n<{hex}> Tj\nET\nQ\n",
        stamp.gs_name,
        stamp.font_name,
        num(m[0]),
        num(m[1]),
        num(m[2]),
        num(m[3]),
        num(m[4]),
        num(m[5]),
        r = num(f64::from(r)),
        g = num(f64::from(g)),
        b = num(f64::from(b)),
        size = num(size),
    )
}

/// A minimal ToUnicode CMap. The UniGB-UCS2-H codes already *are* the
/// characters' UCS-2 code points, so every glyph maps back to itself — the
/// CMap just makes that explicit so copy/paste works even in readers that
/// cannot reverse the Adobe-GB1 ordering on their own.
fn to_unicode_cmap(text: &str) -> Vec<u8> {
    let mut entries: Vec<String> = text
        .chars()
        .map(|c| {
            let mut buf = [0u16; 2];
            let units = c.encode_utf16(&mut buf);
            let hex: String = units.iter().map(|unit| format!("{unit:04X}")).collect();
            format!("<{hex}> <{hex}>")
        })
        .collect();
    entries.sort();
    entries.dedup();

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CMapName /PaperBlade-Watermark def\n/CMapType 2 def\n\
         2 begincodespacerange\n<0000> <FFFF>\n<00000000> <FFFFFFFF>\nendcodespacerange\n",
    );
    // bfchar blocks hold at most 100 entries each.
    for chunk in entries.chunks(100) {
        cmap.push_str(&format!(
            "{} beginbfchar\n{}\nendbfchar\n",
            chunk.len(),
            chunk.join("\n")
        ));
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap.into_bytes()
}

/// The objects every page shares: the non-embedded fonts and the ExtGState
/// carrying the opacity.
struct SharedObjects {
    latin_font: (u32, u16),
    /// None when the text is pure ASCII — no point advertising STSong.
    cjk_font: Option<(u32, u16)>,
    gs: (u32, u16),
}

/// Add the shared drawing objects to the document.
fn add_shared_objects(doc: &mut Document, text: &str, encoded: &Encoded, opacity: f32) -> SharedObjects {
    let latin_font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let cjk_font = (encoded.font == WmFont::Cjk).then(|| add_cjk_font(doc, text));
    let gs = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => opacity,
        "CA" => opacity,
    });
    SharedObjects {
        latin_font,
        cjk_font,
        gs,
    }
}

/// The Adobe preset CJK font: STSong-Light through the predefined
/// UniGB-UCS2-H CMap. Non-embedded on purpose — readers substitute a Song
/// face from their own set instead of us shipping megabytes of font
/// (research doc §3.1; width accuracy is the known cost, §5-R3).
fn add_cjk_font(doc: &mut Document, text: &str) -> (u32, u16) {
    let descriptor = dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "STSong-Light",
        "Flags" => 4i64,
        "FontBBox" => vec![
            Object::Integer(-25),
            Object::Integer(-254),
            Object::Integer(1000),
            Object::Integer(880),
        ],
        "ItalicAngle" => 0i64,
        "Ascent" => 880i64,
        "Descent" => -120i64,
        "CapHeight" => 880i64,
        "StemV" => 93i64,
    };
    let cid_font = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType0",
        "BaseFont" => "STSong-Light",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("GB1"),
            "Supplement" => 2i64,
        },
        "FontDescriptor" => descriptor,
        "DW" => 1000i64,
    });
    let to_unicode = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        to_unicode_cmap(text),
    )));
    doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "STSong-Light",
        "Encoding" => "UniGB-UCS2-H",
        "DescendantFonts" => vec![Object::Reference(cid_font)],
        "ToUnicode" => to_unicode,
    })
}

/// The resource names the watermark adds to one page.
struct ResNames {
    latin: String,
    cjk: Option<String>,
    gs: String,
}

/// Wire the shared objects into the page's resources.
///
/// The page gets its own merged copy of whatever resources it effectively
/// had (its own, or an inherited `/Resources` — the nearest one wins and is
/// *not* merged up the tree by readers). Copying beats mutating in place:
/// a referenced resources dict is often shared by sibling pages, and
/// renaming entries there would be a surprise to all of them.
fn install_page_resources(
    doc: &mut Document,
    page_id: (u32, u16),
    shared: &SharedObjects,
) -> Result<ResNames, String> {
    let mut resources = effective_resources(doc, page_id);

    let mut fonts = sub_dict(doc, &resources, b"Font");
    let latin = unique_name(&fonts, "PbWmF1");
    fonts.set(latin.clone(), Object::Reference(shared.latin_font));
    let cjk = shared.cjk_font.map(|id| {
        let name = unique_name(&fonts, "PbWmF2");
        fonts.set(name.clone(), Object::Reference(id));
        String::from_utf8_lossy(&name).into_owned()
    });
    resources.set("Font", fonts);

    let mut states = sub_dict(doc, &resources, b"ExtGState");
    let gs = unique_name(&states, "PbWmGS");
    states.set(gs.clone(), Object::Reference(shared.gs));
    resources.set("ExtGState", states);

    doc.get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| format!("Could not add watermark resources: {e}"))?
        .set("Resources", resources);

    Ok(ResNames {
        latin: String::from_utf8_lossy(&latin).into_owned(),
        cjk,
        gs: String::from_utf8_lossy(&gs).into_owned(),
    })
}

/// The page's effective `/Resources`: its own, or the nearest ancestor's.
fn effective_resources(doc: &Document, page_id: (u32, u16)) -> Dictionary {
    walk_ancestors(doc, page_id, |dict| match dict.get(b"Resources").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok().cloned(),
        Object::Dictionary(d) => Some(d.clone()),
        _ => None,
    })
    .unwrap_or_default()
}

/// A resources sub-dictionary (`/Font` or `/ExtGState`), dereferencing a
/// shared one so the caller can copy it.
fn sub_dict(doc: &Document, resources: &Dictionary, key: &[u8]) -> Dictionary {
    match resources.get(key).ok() {
        Some(Object::Reference(id)) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
        Some(Object::Dictionary(d)) => d.clone(),
        _ => Dictionary::new(),
    }
}

/// A resource name that cannot shadow anything the page already uses.
fn unique_name(taken: &Dictionary, base: &str) -> Vec<u8> {
    let mut name = base.to_string();
    let mut suffix = 2;
    while taken.get(name.as_bytes()).is_ok() {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name.into_bytes()
}

/// Append `bytes` as one more content stream on the page, on top.
///
/// `/Contents` comes in three shapes: a single stream reference, a direct
/// array, or a reference *to* an array (the trap the research doc flags in
/// §2.4). Normalizing to one direct array of stream references keeps every
/// original stream byte-for-byte intact and just adds the watermark last.
fn append_content_stream(doc: &mut Document, page_id: (u32, u16), bytes: Vec<u8>) -> Result<(), String> {
    let existing = doc
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Contents").ok())
        .cloned();

    let mut list: Vec<Object> = Vec::new();
    match existing {
        Some(Object::Reference(id)) => match doc.objects.get(&id) {
            Some(Object::Stream(_)) => list.push(Object::Reference(id)),
            Some(Object::Array(arr)) => list.extend(arr.iter().cloned()),
            _ => {}
        },
        Some(Object::Array(arr)) => list.extend(arr),
        Some(Object::Stream(stream)) => {
            // Rare, but legal: promote the direct stream to an object so it
            // can sit beside the new one.
            list.push(Object::Reference(doc.add_object(Object::Stream(stream))));
        }
        _ => {}
    }

    let watermark_id = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), bytes)));
    list.push(Object::Reference(watermark_id));

    doc.get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| format!("Could not attach the watermark: {e}"))?
        .set("Contents", list);
    Ok(())
}

/// Stamp `wm` onto every page of `input`, writing the result to `output`.
pub fn watermark_pdf(input: &str, output: &str, wm: &Watermark) -> Result<Report, String> {
    validate_pdf_path(input)?;
    wm.validate()?;
    if Path::new(output) == Path::new(input) {
        return Err("Choose a different file for the output.".into());
    }

    let before_bytes = std::fs::metadata(input)
        .map_err(|e| format!("Failed to read {}: {e}", file_label(input)))?
        .len();

    let mut doc = Document::load(input).map_err(|e| format!("Failed to read {}: {e}", file_label(input)))?;
    if doc.is_encrypted() {
        return Err(format!(
            "{} is password-protected. Remove the password first.",
            file_label(input)
        ));
    }

    let encoded = encode_text(&wm.text);
    let color = parse_hex_color(&wm.color)?;
    let shared = add_shared_objects(&mut doc, &wm.text, &encoded, wm.opacity);
    let pages: Vec<(u32, (u32, u16))> = doc.get_pages().into_iter().collect();

    for (number, page_id) in &pages {
        let rect = page_rect(&doc, *page_id).map_err(|e| format!("Page {number}: {e}"))?;
        let names = install_page_resources(&mut doc, *page_id, &shared)?;
        let font_name = match (encoded.font, &names.cjk) {
            (WmFont::Cjk, Some(name)) => name.clone(),
            _ => names.latin.clone(),
        };
        let width_pt = f64::from(encoded.width_em) * f64::from(wm.font_size);
        let stamp = Stamp {
            font_name: &font_name,
            gs_name: &names.gs,
            encoded: &encoded,
            size: f64::from(wm.font_size),
            color,
            angle: stamp_angle(wm.position, f64::from(wm.rotation), rect.rotate),
        };

        let mut content = String::new();
        for (cx, cy) in stamp_centers(&rect, wm.position, width_pt, f64::from(wm.font_size)) {
            content.push_str(&stamp_ops(&stamp, cx, cy));
        }
        append_content_stream(&mut doc, *page_id, content.into_bytes())?;
    }

    // Never leave a half-written file behind on failure (compress.rs does
    // the same for its engine runs).
    doc.save(output).map_err(|e| {
        let _ = std::fs::remove_file(output);
        format!("Failed to write output: {e}")
    })?;

    let after_bytes = std::fs::metadata(output)
        .map_err(|_| "The watermark was applied but the output file is missing.".to_string())?
        .len();

    Ok(Report {
        output: output.to_string(),
        pages: pages.len() as u32,
        before_bytes,
        after_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::pdf::test_support::write_pdf_with_content;
    use crate::pdf::type_is;

    /// The fixture pages' known content stream.
    const ORIGINAL: &[u8] = b"0.5 g 0 0 100 100 re f\n";

    struct Fixture {
        dir: PathBuf,
        input: String,
        output: String,
    }

    /// A fresh temp dir holding `input.pdf` with `pages` pages of known
    /// content, plus the path the stamped copy goes to.
    fn fixture(name: &str, pages: usize) -> Fixture {
        let dir = std::env::temp_dir().join(format!("paperblade_watermark_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let path = |n: &str| dir.join(n).to_str().unwrap().to_string();
        let input = path("input.pdf");
        write_pdf_with_content(&input, pages, Some(ORIGINAL));
        let output = path("output.pdf");
        Fixture { dir, input, output }
    }

    fn draft() -> Watermark {
        Watermark {
            text: "DRAFT".into(),
            opacity: 0.3,
            rotation: 0.0,
            position: Position::Center,
            font_size: 42.0,
            color: "#3366cc".into(),
        }
    }

    /// The concatenated content streams of every page in a saved PDF.
    fn page_contents(path: &str) -> Vec<String> {
        let doc = Document::load(path).expect("reopen the watermarked pdf");
        doc.get_pages()
            .values()
            .map(|id| String::from_utf8_lossy(&doc.get_page_content(*id).unwrap()).into_owned())
            .collect()
    }

    /// The operands of the first `Tm` operator in a content stream.
    fn first_tm(content: &str) -> [f64; 6] {
        let line = content
            .lines()
            .find(|l| l.ends_with("Tm"))
            .expect("the stamp writes a text matrix");
        let mut values = [0.0; 6];
        for (slot, token) in values.iter_mut().zip(line.split_whitespace()) {
            *slot = token.parse().expect("Tm operands are numbers");
        }
        values
    }

    #[test]
    fn appends_a_stream_without_touching_the_original_content() {
        let f = fixture("append", 1);
        watermark_pdf(&f.input, &f.output, &draft()).expect("watermark should succeed");

        let doc = Document::load(&f.output).expect("reopens cleanly");
        let page_id = *doc.get_pages().values().next().unwrap();
        let contents = doc
            .get_dictionary(page_id)
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .as_array()
            .expect("normalized to an array");
        assert_eq!(contents.len(), 2, "original stream plus the watermark");

        let first = contents[0].as_reference().unwrap();
        let original = doc.get_object(first).unwrap().as_stream().unwrap();
        assert_eq!(
            original.decompressed_content().unwrap(),
            ORIGINAL.to_vec(),
            "the original page content must stay byte-for-byte intact"
        );
    }

    #[test]
    fn the_output_reopens_and_keeps_every_page() {
        let f = fixture("round_trip", 3);
        let report = watermark_pdf(&f.input, &f.output, &draft()).expect("watermark should succeed");

        let doc = Document::load(&f.output).expect("the watermarked pdf is structurally sound");
        assert_eq!(doc.get_pages().len(), 3);
        assert_eq!(report.pages, 3);
        assert!(
            report.after_bytes > report.before_bytes,
            "stamping adds bytes; the report must not hide that"
        );
        for content in page_contents(&f.output) {
            assert!(content.contains("0.5 g 0 0 100 100 re f"), "original survives");
            assert!(content.contains("Tj"), "the stamp is drawn");
        }
    }

    #[test]
    fn tiled_repeats_the_stamp_across_the_page() {
        let f = fixture("tiled", 1);
        let mut wm = draft();
        wm.position = Position::Tiled;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let contents = page_contents(&f.output);
        let stamps = contents[0].matches("Tj").count();
        assert!(stamps > 3, "a tiled watermark repeats, got {stamps} stamps");
    }

    #[test]
    fn diagonal_sits_at_forty_five_degrees() {
        let f = fixture("diagonal", 1);
        let mut wm = draft();
        wm.position = Position::Diagonal;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let m = first_tm(&page_contents(&f.output)[0]);
        // The content stream rounds to three decimals.
        assert!((m[0] - 45.0_f64.to_radians().cos()).abs() < 0.001, "got {m:?}");
        assert!((m[1] - 45.0_f64.to_radians().sin()).abs() < 0.001, "got {m:?}");
    }

    #[test]
    fn rotation_is_written_into_the_text_matrix() {
        let f = fixture("rotation", 1);
        let mut wm = draft();
        wm.rotation = 30.0;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let m = first_tm(&page_contents(&f.output)[0]);
        // The content stream rounds to three decimals.
        assert!((m[0] - 30.0_f64.to_radians().cos()).abs() < 0.001, "got {m:?}");
        assert!((m[1] - 30.0_f64.to_radians().sin()).abs() < 0.001, "got {m:?}");
    }

    #[test]
    fn a_rotated_page_is_compensated() {
        let f = fixture("page_rotate", 1);
        let mut doc = Document::load(&f.input).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        doc.get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Rotate", 90i64);
        doc.save(&f.input).unwrap();

        // A horizontal stamp on a page the reader turns 90° must be drawn at
        // -90° to appear upright.
        watermark_pdf(&f.input, &f.output, &draft()).expect("watermark should succeed");

        let m = first_tm(&page_contents(&f.output)[0]);
        assert!((m[0]).abs() < 1e-6, "cos(-90) is 0, got {m:?}");
        assert!((m[1] + 1.0).abs() < 1e-6, "sin(-90) is -1, got {m:?}");
    }

    #[test]
    fn opacity_is_painted_through_an_extgstate() {
        let f = fixture("opacity", 1);
        let mut wm = draft();
        wm.opacity = 0.45;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let doc = Document::load(&f.output).unwrap();
        let alphas: Vec<f32> = doc
            .objects
            .values()
            .filter(|o| type_is(o, b"ExtGState"))
            .map(|o| o.as_dict().unwrap().get(b"ca").unwrap().as_float().unwrap())
            .collect();
        assert_eq!(alphas, vec![0.45], "the stamp's opacity lives in one ExtGState");
        assert!(page_contents(&f.output)[0].contains("/PbWmGS gs"));
    }

    #[test]
    fn color_and_size_land_in_the_content_stream() {
        let f = fixture("color_size", 1);
        let mut wm = draft();
        wm.color = "#3366cc".into();
        wm.font_size = 42.0;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let content = &page_contents(&f.output)[0];
        assert!(content.contains("0.2 0.4 0.8 rg"), "got {content}");
        assert!(content.contains("/PbWmF1 42 Tf"), "got {content}");
    }

    /// The first font dictionary in the output matching `/BaseFont` and
    /// `/Subtype` (STSong appears twice: the Type0 font and its CIDFont).
    fn find_font(doc: &Document, base_font: &[u8], subtype: &[u8]) -> Option<Dictionary> {
        doc.objects.values().find_map(|o| {
            let d = o.as_dict().ok()?;
            if !type_is(o, b"Font") {
                return None;
            }
            let base_ok = d.get(b"BaseFont").and_then(Object::as_name).ok()? == base_font;
            let sub_ok = d.get(b"Subtype").and_then(Object::as_name).ok()? == subtype;
            (base_ok && sub_ok).then(|| d.clone())
        })
    }

    #[test]
    fn latin_text_uses_helvetica_with_hex_encoding() {
        let f = fixture("latin", 1);
        let mut wm = draft();
        wm.text = "Hi".into();

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let doc = Document::load(&f.output).unwrap();
        assert!(
            find_font(&doc, b"Helvetica", b"Type1").is_some(),
            "ASCII text draws with the base-14 Helvetica"
        );
        assert!(
            page_contents(&f.output)[0].contains("<4869> Tj"),
            "'Hi' as hex bytes"
        );
    }

    #[test]
    fn cjk_text_uses_the_preset_chinese_font() {
        let f = fixture("cjk", 1);
        let mut wm = draft();
        wm.text = "保密".into();

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let doc = Document::load(&f.output).unwrap();
        let font = find_font(&doc, b"STSong-Light", b"Type0").expect("CJK text draws with STSong-Light");
        assert_eq!(
            font.get(b"Encoding").and_then(Object::as_name).ok(),
            Some(&b"UniGB-UCS2-H"[..]),
        );
        assert!(font.get(b"ToUnicode").is_ok(), "copy/paste needs the CMap");
        let cid_ref = font.get(b"DescendantFonts").and_then(Object::as_array).unwrap()[0]
            .as_reference()
            .unwrap();
        let cid = doc.get_dictionary(cid_ref).unwrap();
        // MuPDF refuses a CID font without a FontDescriptor (research doc §2.4).
        assert!(cid.get(b"FontDescriptor").is_ok());
        assert!(
            page_contents(&f.output)[0].contains("<4FDD5BC6> Tj"),
            "'保密' as UTF-16BE show text"
        );
    }

    #[test]
    fn to_unicode_maps_each_glyph_back_to_itself() {
        let cmap = String::from_utf8(to_unicode_cmap("中a")).unwrap();
        assert!(cmap.contains("<4E2D> <4E2D>"), "got {cmap}");
        assert!(cmap.contains("<0061> <0061>"), "got {cmap}");
        assert!(cmap.contains("beginbfchar"), "got {cmap}");
    }

    #[test]
    fn latin_widths_come_from_the_afm_table() {
        assert!((latin_width_em("M") - 0.833).abs() < 0.001);
        assert!((latin_width_em("ii") - 0.444).abs() < 0.001);
    }

    #[test]
    fn cjk_widths_carry_the_safety_margin() {
        // Full-width glyph = 1 em, Latin inside the CJK path = 0.5 em, and
        // the whole estimate is padded for substituted-font drift (§5-R3).
        assert!((cjk_width_em("中") - 1.25).abs() < 0.001);
        assert!((cjk_width_em("a中") - 1.875).abs() < 0.001);
    }

    #[test]
    fn deserializes_the_options_the_frontend_sends() {
        let json = r##"{"text":"DRAFT","opacity":0.2,"rotation":45,"position":"tiled","fontSize":36,"color":"#888888"}"##;
        let wm: Watermark = serde_json::from_str(json).expect("contract is camelCase");
        assert_eq!(wm.position, Position::Tiled);
        assert_eq!(wm.font_size, 36.0);

        for bad in [
            r##"{"text":"x","opacity":0.2,"rotation":0,"position":"spread","fontSize":36,"color":"#888888"}"##,
            r#"{"text":"x","opacity":0.2,"rotation":0,"position":"center","fontSize":36}"#,
        ] {
            assert!(serde_json::from_str::<Watermark>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_a_blank_watermark_text() {
        let f = fixture("blank_text", 1);
        let mut wm = draft();
        wm.text = "   ".into();

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("Enter the watermark text"), "got {err}");
    }

    #[test]
    fn rejects_a_paste_accident_length_text() {
        let f = fixture("long_text", 1);
        let mut wm = draft();
        wm.text = "x".repeat(MAX_TEXT_CHARS + 1);

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("200 characters"), "got {err}");
    }

    #[test]
    fn rejects_opacity_outside_the_unit_interval() {
        let f = fixture("opacity_bounds", 1);
        for opacity in [0.0, 1.5] {
            let mut wm = draft();
            wm.opacity = opacity;
            let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
            assert!(err.contains("Opacity"), "got {err}");
        }
    }

    #[test]
    fn rejects_a_font_size_out_of_range() {
        let f = fixture("size_bounds", 1);
        for font_size in [0.5, 500.0] {
            let mut wm = draft();
            wm.font_size = font_size;
            let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
            assert!(err.contains("Font size"), "got {err}");
        }
    }

    #[test]
    fn rejects_an_angle_out_of_range() {
        let f = fixture("angle_bounds", 1);
        let mut wm = draft();
        wm.rotation = 400.0;

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("Angle"), "got {err}");
    }

    #[test]
    fn rejects_a_color_that_is_not_hex() {
        let f = fixture("bad_color", 1);
        for color in ["red", "#12345", "#3366cc00"] {
            let mut wm = draft();
            wm.color = color.into();
            let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
            assert!(err.contains("#rrggbb"), "got {err}");
        }
    }

    #[test]
    fn refuses_to_overwrite_its_own_input() {
        let f = fixture("same_path", 1);
        let err = watermark_pdf(&f.input, &f.input, &draft()).unwrap_err();
        assert!(err.contains("different file"), "got {err}");
    }

    #[test]
    fn refuses_a_password_protected_file() {
        let f = fixture("encrypted", 1);
        let locked = f.dir.join("locked.pdf");
        let locked = locked.to_str().unwrap();
        crate::encrypt::encrypt_pdf(&f.input, locked, "hunter2").expect("make a locked fixture");

        let err = watermark_pdf(locked, &f.output, &draft()).unwrap_err();
        assert!(err.contains("password-protected"), "got {err}");
    }

    #[test]
    fn a_failed_save_leaves_no_file_behind() {
        let f = fixture("failed_save", 1);
        let output = f.dir.join("missing-dir").join("out.pdf");

        let err = watermark_pdf(&f.input, output.to_str().unwrap(), &draft()).unwrap_err();

        assert!(err.contains("Failed to write"), "got {err}");
        assert!(!output.exists(), "a failed run must not leave a partial file");
    }

    #[test]
    fn centers_on_a_page_that_does_not_start_at_the_origin() {
        let f = fixture("offset_box", 1);
        let mut doc = Document::load(&f.input).unwrap();
        let page_id = *doc.get_pages().values().next().unwrap();
        doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap().set(
            "MediaBox",
            vec![
                Object::Integer(100),
                Object::Integer(50),
                Object::Integer(712),
                Object::Integer(842),
            ],
        );
        doc.save(&f.input).unwrap();

        watermark_pdf(&f.input, &f.output, &draft()).expect("watermark should succeed");

        // The page's center is (406, 446), not (306, 396): the stamp's
        // midpoint must land there even though the box skips (0, 0).
        let m = first_tm(&page_contents(&f.output)[0]);
        let width_pt = 3.333 * 42.0; // "DRAFT" in Helvetica ems
        assert!((m[4] + width_pt / 2.0 - 406.0).abs() < 0.01, "got {m:?}");
        assert!((m[5] + 0.35 * 42.0 - 446.0).abs() < 0.01, "got {m:?}");
    }
}
