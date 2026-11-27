//! Images to PDF: one page per image, in the order given.
//!
//! Pure Rust through printpdf (research doc
//! `docs/research/m2-render-engine.md` §3.2) — no engine, no external
//! process. The page-size strategy decides the sheet, the fit strategy how
//! the picture sits on it.
//!
//! JPEGs travel light: a JPEG already *is* the compressed sample data a PDF
//! image stream wants, so its bytes are embedded verbatim as a DCTDecode
//! stream (no decode, no re-encode). PNGs are decoded and stored losslessly.

use std::collections::BTreeMap;
use std::path::Path;

use printpdf::{
    CurTransMat, DictItem, ExternalStream, ExternalXObject, ImageCompression, ImageOptimizationOptions, Mm,
    Op, PaintMode, PdfDocument, PdfPage, PdfSaveOptions, Pt, Px, RawImage, Rect, XObjectId, XObjectTransform,
};

use crate::pdf::{file_label, file_stem};

/// Pixels per inch when the image carries no density metadata.
const DEFAULT_DPI: f32 = 96.0;

/// Refuse images past this many pixels. A 100 MP scan decodes to hundreds of
/// megabytes and the whole document is built in memory, so the size is gated
/// from the file header before any pixel buffer is allocated (research doc
/// §5-R7).
const MAX_PIXELS: u64 = 100_000_000;

/// A4: 210 × 297 mm in points.
const A4_PT: (f32, f32) = (595.276, 841.89);

/// US Letter: 8.5 × 11 in in points.
const LETTER_PT: (f32, f32) = (612.0, 792.0);

/// Which sheet each image lands on.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageSize {
    /// The page is the image's own size at its embedded DPI
    /// (`DEFAULT_DPI` when the file says nothing).
    Image,
    /// ISO A4.
    A4,
    /// US Letter.
    Letter,
}

/// How the picture fills a fixed-size page. On a `PageSize::Image` page the
/// page has the picture's own aspect ratio, so all three land on the same
/// full-page rectangle.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    /// Scale uniformly to fit inside the page, centered (letterboxed).
    Contain,
    /// Scale uniformly to cover the page, centered; the overflow is cropped.
    Cover,
    /// Fill the page exactly, distorting the aspect ratio if needed.
    Stretch,
}

/// Everything the frontend sends with one run.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub page_size: PageSize,
    pub fit: Fit,
}

/// What building the PDF actually did, so the UI can be honest about it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub output: String,
    pub pages: u32,
    pub output_bytes: u64,
}

/// What the file header says about the picture, before any pixels are read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageInfo {
    pub(crate) width_px: u32,
    pub(crate) height_px: u32,
    /// Dots per inch from the file metadata, or `DEFAULT_DPI`.
    pub(crate) dpi: f32,
}

/// Read the size and density of a PNG or JPEG from its header alone.
///
/// Both live at fixed spots near the start of the file: PNG puts the size in
/// its mandatory first chunk (IHDR) and the density in an optional pHYs;
/// JPEG carries both in marker segments before any scan data. Reading them
/// here is what lets the oversize gate run before decoding.
pub(crate) fn image_info(bytes: &[u8]) -> Result<ImageInfo, String> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        png_info(bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_info(bytes).map(|(info, _)| info)
    } else {
        Err("this is not a PNG or JPEG image".into())
    }
}

/// PNG: size from IHDR (always the first chunk), density from pHYs.
fn png_info(bytes: &[u8]) -> Result<ImageInfo, String> {
    // 8-byte signature, then the IHDR chunk: length, type, 13 bytes, CRC.
    const IHDR_END: usize = 8 + 4 + 4 + 13 + 4;
    if bytes.len() < IHDR_END || &bytes[12..16] != b"IHDR" {
        return Err("the PNG header is damaged".into());
    }
    let mut dpi = None;
    let mut offset = IHDR_END;
    // Metadata chunks come before IDAT; stop there instead of walking the
    // pixel data of a hundred-megapixel file.
    while offset + 8 <= bytes.len() {
        let len = be_u32(bytes, offset) as usize;
        if offset + 12 + len > bytes.len() {
            break;
        }
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"IDAT" || kind == b"IEND" {
            break;
        }
        if kind == b"pHYs" && len == 9 {
            dpi = phys_dpi(&bytes[offset + 8..offset + 17]);
        }
        offset += 12 + len;
    }
    Ok(ImageInfo {
        width_px: be_u32(bytes, 16),
        height_px: be_u32(bytes, 20),
        dpi: dpi.unwrap_or(DEFAULT_DPI),
    })
}

/// A pHYs chunk's density, when it names one. `unit == 1` is dots per meter;
/// `unit == 0` only carries a pixel aspect ratio and says nothing about size.
fn phys_dpi(data: &[u8]) -> Option<f32> {
    let dots_per_meter = be_u32(data, 0);
    if data[8] != 1 || dots_per_meter == 0 {
        return None;
    }
    Some(dots_per_meter as f32 * 0.0254)
}

/// What a JPEG's frame and EXIF segments say, beyond its size.
#[derive(Debug, Clone, Copy, PartialEq)]
struct JpegDetails {
    /// The SOF marker that describes the frame (0xC0..=0xCF).
    frame: u8,
    /// Color components per sample: 1, 3 or 4 (gray, YCbCr/RGB, CMYK).
    components: u8,
    /// Bits per sample. Readers only meet 8-bit JPEGs in the wild.
    precision: u8,
    /// The EXIF orientation tag (1..=8), 1 when the file says nothing.
    orientation: u8,
}

/// JPEG: size from the frame marker, density from the JFIF APP0 segment,
/// color model from the frame, orientation from an EXIF APP1 segment.
///
/// Markers are walked the way a decoder would: most carry a two-byte length,
/// a few stand alone, and the scan data after SOS is none of this function's
/// business. The first frame marker (SOF*) wins.
fn jpeg_info(bytes: &[u8]) -> Result<(ImageInfo, JpegDetails), String> {
    let damaged = || "the JPEG header is damaged".to_string();
    let mut offset = 2; // past SOI
    let mut size = None;
    let mut frame = None;
    let mut dpi = None;
    let mut orientation = 1;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            return Err(damaged());
        }
        let marker = bytes[offset + 1];
        match marker {
            0xD8 | 0x01 | 0xD0..=0xD7 => {
                offset += 2; // stand-alone markers carry no payload
                continue;
            }
            0xD9 | 0xDA => break, // end of image / start of scan
            _ => {}
        }
        let len = be_u16(bytes, offset + 2) as usize;
        if len < 2 || offset + 2 + len > bytes.len() {
            return Err(damaged());
        }
        let payload = &bytes[offset + 4..offset + 2 + len];
        let is_frame = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_frame && frame.is_none() && payload.len() >= 6 {
            frame = Some((marker, payload[5], payload[0]));
            size = Some((be_u16(payload, 3) as u32, be_u16(payload, 1) as u32));
        }
        if marker == 0xE0 && payload.starts_with(b"JFIF\0") {
            dpi = jfif_dpi(payload);
        }
        if marker == 0xE1 && orientation == 1 {
            if let Some(value) = exif_orientation(payload) {
                orientation = value;
            }
        }
        offset += 2 + len;
    }
    let (width_px, height_px) = size.ok_or_else(damaged)?;
    let (frame, components, precision) = frame.ok_or_else(damaged)?;
    Ok((
        ImageInfo {
            width_px,
            height_px,
            dpi: dpi.unwrap_or(DEFAULT_DPI),
        },
        JpegDetails {
            frame,
            components,
            precision,
            orientation,
        },
    ))
}

/// The orientation tag of a JPEG's EXIF APP1 segment, when it carries one.
///
/// `payload` is the segment body: `Exif\0\0` then a TIFF header whose first
/// IFD may hold tag 0x0112 (Orientation). The tag says how the stored samples
/// must be turned to stand upright — photo viewers do that on display, PDF
/// viewers do not, so placement compensates instead.
fn exif_orientation(payload: &[u8]) -> Option<u8> {
    let tiff = payload.strip_prefix(b"Exif\0\0")?;
    let big_endian = match tiff.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes = tiff.get(at..at + 2)?;
        Some(if big_endian {
            u16::from_be_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_le_bytes([bytes[0], bytes[1]])
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    for i in 0..u16_at(ifd)? as usize {
        let entry = ifd + 2 + i * 12;
        // Type 3 is SHORT: the value lives in the entry's value field.
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 {
            return match u16_at(entry + 8)? {
                value @ 1..=8 => Some(value as u8),
                _ => None,
            };
        }
    }
    None
}

/// A JFIF APP0 segment's density. Units: 1 is dots per inch, 2 is dots per
/// centimeter, 0 is a bare aspect ratio with no physical size.
fn jfif_dpi(payload: &[u8]) -> Option<f32> {
    if payload.len() < 10 {
        return None;
    }
    let density = be_u16(payload, 8);
    if density == 0 {
        return None;
    }
    match payload[7] {
        1 => Some(f32::from(density)),
        2 => Some(f32::from(density) * 2.54),
        _ => None,
    }
}

fn be_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

/// Read one image file, refusing empty, foreign and oversized files before
/// any pixel buffer is allocated. The image watermark reuses this gate.
pub(crate) fn read_image(path: &str) -> Result<(Vec<u8>, ImageInfo), String> {
    validate_image_path(path)?;
    let label = file_label(path);
    let bytes = std::fs::read(path).map_err(|e| format!("Failed to read {label}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("{label} is empty."));
    }
    let info = image_info(&bytes).map_err(|e| format!("Failed to read {label}: {e}"))?;
    let pixels = u64::from(info.width_px) * u64::from(info.height_px);
    if pixels > MAX_PIXELS {
        return Err(format!(
            "{label} is too large ({} megapixels, the limit is {}).",
            pixels / 1_000_000,
            MAX_PIXELS / 1_000_000
        ));
    }
    Ok((bytes, info))
}

/// One input image, ready to place.
struct Source {
    /// Stored pixel size, before any EXIF rotation.
    width_px: u32,
    height_px: u32,
    /// Dots per inch from the file metadata, or `DEFAULT_DPI`.
    dpi: f32,
    /// The EXIF orientation tag (1..=8), 1 when the file says nothing.
    orientation: u8,
    /// How the picture gets into the PDF.
    payload: Payload,
}

/// How one image's picture data gets into the PDF.
enum Payload {
    /// Decoded pixels, embedded through printpdf's own image path (PNG).
    Pixels(RawImage),
    /// The file's own JPEG bytes, embedded verbatim as a DCTDecode stream.
    Jpeg(ExternalXObject),
}

/// Read one image: JPEGs go in as they are, PNGs are decoded to pixels.
fn load_source(path: &str) -> Result<Source, String> {
    let (bytes, info) = read_image(path)?;
    let label = file_label(path);
    let (orientation, payload) = if bytes.starts_with(&[0xFF, 0xD8]) {
        let (_, details) = jpeg_info(&bytes).map_err(|e| format!("Failed to read {label}: {e}"))?;
        (
            details.orientation,
            Payload::Jpeg(jpeg_xobject(&bytes, &info, &details, &label)?),
        )
    } else {
        let mut warnings = Vec::new();
        let raw = RawImage::decode_from_bytes(&bytes, &mut warnings)
            .map_err(|e| format!("Failed to read {label}: {e}"))?;
        (1, Payload::Pixels(raw))
    };
    Ok(Source {
        width_px: info.width_px,
        height_px: info.height_px,
        dpi: info.dpi,
        orientation,
        payload,
    })
}

/// The image XObject for a JPEG, carrying the file's own bytes.
///
/// A JPEG already *is* the compressed sample data a PDF image stream wants
/// (`DCTDecode`), so the bytes go into the PDF untouched — decoding and
/// re-encoding would only lose quality and inflate photos. What cannot ride
/// along is refused up front: PDF only promises baseline, extended-sequential
/// and progressive JPEGs, and only 8-bit samples of 1 (gray), 3 (YCbCr/RGB)
/// or 4 (CMYK) components.
fn jpeg_xobject(
    bytes: &[u8],
    info: &ImageInfo,
    details: &JpegDetails,
    label: &str,
) -> Result<ExternalXObject, String> {
    if !matches!(details.frame, 0xC0..=0xC2) {
        return Err(format!("{label} is not a baseline or progressive JPEG."));
    }
    if details.precision != 8 {
        return Err(format!(
            "{label} has {}-bit samples; only 8-bit JPEGs are supported.",
            details.precision
        ));
    }
    let color_space = match details.components {
        1 => "DeviceGray",
        3 => "DeviceRGB",
        4 => "DeviceCMYK",
        n => {
            return Err(format!(
                "{label} has {n} color components, which is not supported."
            ))
        }
    };
    let mut dict: BTreeMap<String, DictItem> = BTreeMap::new();
    dict.insert("Type".into(), DictItem::Name(b"XObject".to_vec()));
    dict.insert("Subtype".into(), DictItem::Name(b"Image".to_vec()));
    dict.insert("Width".into(), DictItem::Int(i64::from(info.width_px)));
    dict.insert("Height".into(), DictItem::Int(i64::from(info.height_px)));
    dict.insert(
        "BitsPerComponent".into(),
        DictItem::Int(i64::from(details.precision)),
    );
    dict.insert(
        "ColorSpace".into(),
        DictItem::Name(color_space.as_bytes().to_vec()),
    );
    dict.insert("Filter".into(), DictItem::Name(b"DCTDecode".to_vec()));
    dict.insert("Interpolate".into(), DictItem::Bool(false));
    Ok(ExternalXObject {
        stream: ExternalStream {
            dict,
            content: bytes.to_vec(),
            // Already encoded — the writer must leave the bytes alone.
            compress: false,
        },
        width: Some(Px(info.width_px as usize)),
        height: Some(Px(info.height_px as usize)),
        dpi: None,
    })
}

/// Reject anything that is not an existing file with an image extension.
///
/// The extension only answers fast for the obvious wrong file — the format
/// itself is re-checked from the file's own bytes in `image_info`.
pub(crate) fn validate_image_path(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(format!("File not found: {}", file_label(path)));
    }
    let is_image = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")
        })
        .unwrap_or(false);
    if !is_image {
        return Err(format!("Not a PNG or JPEG image: {}", file_label(path)));
    }
    Ok(())
}

/// Where one image sits on its page, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// The size the picture stands at once its EXIF rotation is applied: the
/// tags 5..=8 turn the picture a quarter turn, swapping its sides.
fn displayed_size(width_px: u32, height_px: u32, orientation: u8) -> (u32, u32) {
    match orientation {
        5..=8 => (height_px, width_px),
        _ => (width_px, height_px),
    }
}

/// The page size in points for one image.
fn page_size(width_px: u32, height_px: u32, dpi: f32, page_size: PageSize) -> (f32, f32) {
    match page_size {
        PageSize::Image => (px_to_pt(width_px, dpi), px_to_pt(height_px, dpi)),
        PageSize::A4 | PageSize::Letter => {
            let (wide, tall) = match page_size {
                PageSize::A4 => A4_PT,
                _ => LETTER_PT,
            };
            // Orientation follows the picture: a landscape photo gets a
            // landscape sheet, and nothing is letterboxed sideways.
            if width_px > height_px {
                (tall, wide)
            } else {
                (wide, tall)
            }
        }
    }
}

/// Scale and center the image on the page for the fit strategy. `page` is in
/// points; the returned rectangle is too.
fn placement(width_px: u32, height_px: u32, page: (f32, f32), fit: Fit) -> Placement {
    let (page_w, page_h) = page;
    let w = width_px as f32;
    let h = height_px as f32;
    let (width, height) = match fit {
        Fit::Stretch => (page_w, page_h),
        Fit::Contain => scaled(w, h, (page_w / w).min(page_h / h)),
        Fit::Cover => scaled(w, h, (page_w / w).max(page_h / h)),
    };
    // Centered: uniform fits leave a band on one axis, stretch leaves none.
    Placement {
        x: (page_w - width) / 2.0,
        y: (page_h - height) / 2.0,
        width,
        height,
    }
}

/// The image rectangle after scaling both axes by `factor`.
fn scaled(width_px: f32, height_px: f32, factor: f32) -> (f32, f32) {
    (width_px * factor, height_px * factor)
}

/// Pixels to points at the image's density.
fn px_to_pt(px: u32, dpi: f32) -> f32 {
    px as f32 * 72.0 / dpi
}

/// The ops that paint one image on its page.
///
/// `UseXobject` scales the image's unit square to its pixel size at the
/// transform's DPI (72 makes one pixel one point), then applies the fit's
/// scale and offset. Cover is the only strategy that can run past the page,
/// so it clips to the sheet first — the clip is undone by the `Q` after.
fn image_ops(
    id: XObjectId,
    place: Placement,
    width_px: u32,
    height_px: u32,
    page: (f32, f32),
    fit: Fit,
) -> Vec<Op> {
    let transform = XObjectTransform {
        dpi: Some(72.0),
        translate_x: Some(Pt(place.x)),
        translate_y: Some(Pt(place.y)),
        scale_x: Some(place.width / width_px as f32),
        scale_y: Some(place.height / height_px as f32),
        ..Default::default()
    };
    let draw = Op::UseXobject { id, transform };
    if fit == Fit::Cover {
        vec![
            Op::SaveGraphicsState,
            Op::DrawRectangle {
                rectangle: clip_rect(page),
            },
            draw,
            Op::RestoreGraphicsState,
        ]
    } else {
        vec![draw]
    }
}

/// The page rectangle a cover fit clips to.
fn clip_rect(page: (f32, f32)) -> Rect {
    Rect {
        x: Pt(0.0),
        y: Pt(0.0),
        width: Pt(page.0),
        height: Pt(page.1),
        mode: Some(PaintMode::Clip),
        winding_order: None,
    }
}

/// How the stored samples must be turned to stand upright (EXIF 1..=8), as a
/// matrix on the image's unit square (origin bottom left, the first sample at
/// the top left): `[a b c d e f]`, PDF style. The tag turns the *stored*
/// picture into the displayed one, so the matrix does the same.
fn orientation_matrix(orientation: u8) -> [f64; 6] {
    match orientation {
        2 => [-1.0, 0.0, 0.0, 1.0, 1.0, 0.0],  // mirrored left–right
        3 => [-1.0, 0.0, 0.0, -1.0, 1.0, 1.0], // turned upside down
        4 => [1.0, 0.0, 0.0, -1.0, 0.0, 1.0],  // mirrored top–bottom
        5 => [0.0, -1.0, -1.0, 0.0, 1.0, 1.0], // transposed
        6 => [0.0, -1.0, 1.0, 0.0, 0.0, 1.0],  // turned 90° clockwise
        7 => [0.0, 1.0, 1.0, 0.0, 0.0, 0.0],   // transverse
        8 => [0.0, 1.0, -1.0, 0.0, 1.0, 0.0],  // turned 90° counter-clockwise
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],   // upright
    }
}

/// The matrix applying `inner` first, then `outer`.
fn concat(outer: [f64; 6], inner: [f64; 6]) -> [f64; 6] {
    [
        outer[0] * inner[0] + outer[2] * inner[1],
        outer[1] * inner[0] + outer[3] * inner[1],
        outer[0] * inner[2] + outer[2] * inner[3],
        outer[1] * inner[2] + outer[3] * inner[3],
        outer[0] * inner[4] + outer[2] * inner[5] + outer[4],
        outer[1] * inner[4] + outer[3] * inner[5] + outer[5],
    ]
}

/// The ops that paint an EXIF-rotated image on its page.
///
/// PDF viewers ignore EXIF and draw the samples as stored, so the unit square
/// goes through the orientation matrix first and the fit placement after —
/// one `cm` doing both. `UseXobject` gets `no_auto_scale`: the matrix is the
/// whole placement.
fn oriented_ops(id: XObjectId, place: Placement, orientation: u8, page: (f32, f32), fit: Fit) -> Vec<Op> {
    let placement = [
        f64::from(place.width),
        0.0,
        0.0,
        f64::from(place.height),
        f64::from(place.x),
        f64::from(place.y),
    ];
    let m = concat(placement, orientation_matrix(orientation));
    let matrix = CurTransMat::Raw([
        m[0] as f32,
        m[1] as f32,
        m[2] as f32,
        m[3] as f32,
        m[4] as f32,
        m[5] as f32,
    ]);
    let mut ops = Vec::new();
    if fit == Fit::Cover {
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::DrawRectangle {
            rectangle: clip_rect(page),
        });
    }
    ops.push(Op::SaveGraphicsState);
    ops.push(Op::SetTransformationMatrix { matrix });
    ops.push(Op::UseXobject {
        id,
        transform: XObjectTransform {
            no_auto_scale: true,
            ..Default::default()
        },
    });
    ops.push(Op::RestoreGraphicsState);
    if fit == Fit::Cover {
        ops.push(Op::RestoreGraphicsState);
    }
    ops
}

/// Lossless image encoding, no shrinking.
///
/// `PdfSaveOptions::default()` is lossy twice over: it re-encodes color
/// images to JPEG at quality 0.85 and scales down whatever passes 2 MB
/// (nearest-neighbour sampling). A conversion tool must not quietly destroy
/// image data — the Compress tool exists for when size matters more. JPEG
/// inputs never reach this machinery at all: they are embedded byte for
/// byte on the external-XObject path.
fn save_options() -> PdfSaveOptions {
    PdfSaveOptions {
        subset_fonts: false,
        image_optimization: Some(ImageOptimizationOptions {
            format: Some(ImageCompression::Flate),
            max_image_size: None,
            auto_optimize: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build one PDF out of `inputs`, one page per image, into `output`.
///
/// Every input is read before the first byte is written: a bad image fails
/// the run outright, and a failed write removes whatever landed on disk
/// (compress.rs and watermark.rs do the same).
pub fn images_to_pdf(inputs: &[String], output: &str, options: Options) -> Result<Report, String> {
    if inputs.is_empty() {
        return Err("Add at least one image.".into());
    }
    if inputs.iter().any(|path| Path::new(path) == Path::new(output)) {
        return Err("Choose a different file for the output.".into());
    }

    let mut doc = PdfDocument::new(&file_stem(output));
    let mut pages = Vec::with_capacity(inputs.len());

    for path in inputs {
        let source = load_source(path)?;
        let (shown_w, shown_h) = displayed_size(source.width_px, source.height_px, source.orientation);
        let id = match &source.payload {
            Payload::Pixels(raw) => doc.add_image(raw),
            Payload::Jpeg(xobject) => doc.add_xobject(xobject),
        };
        let page_pt = page_size(shown_w, shown_h, source.dpi, options.page_size);
        let place = placement(shown_w, shown_h, page_pt, options.fit);
        let ops = if source.orientation == 1 {
            image_ops(id, place, source.width_px, source.height_px, page_pt, options.fit)
        } else {
            oriented_ops(id, place, source.orientation, page_pt, options.fit)
        };
        pages.push(PdfPage::new(
            Mm::from(Pt(page_pt.0)),
            Mm::from(Pt(page_pt.1)),
            ops,
        ));
    }

    let bytes = doc.with_pages(pages).save(&save_options(), &mut Vec::new());

    std::fs::write(output, &bytes).map_err(|e| {
        let _ = std::fs::remove_file(output);
        format!("Failed to write output: {e}")
    })?;

    let output_bytes = std::fs::metadata(output)
        .map_err(|_| "The PDF was written but the output file is missing.".to_string())?
        .len();

    Ok(Report {
        output: output.to_string(),
        pages: inputs.len() as u32,
        output_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lopdf::{Dictionary, Document, Object};
    use printpdf::{OutputImageFormat, RawImageData, RawImageFormat};

    use super::*;

    struct Fixture {
        dir: PathBuf,
        output: String,
    }

    /// A fresh temp dir the test writes its images and PDF into.
    fn fixture(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("paperblade_convert_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let output = dir.join("output.pdf").to_str().unwrap().to_string();
        Fixture { dir, output }
    }

    impl Fixture {
        /// Write `bytes` as `name` and return its path.
        fn image(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.dir.join(name);
            std::fs::write(&path, bytes).expect("write fixture image");
            path.to_str().unwrap().to_string()
        }
    }

    fn options(page_size: PageSize, fit: Fit) -> Options {
        Options { page_size, fit }
    }

    /// A flat-color RGB image of the given size.
    fn solid(width: usize, height: usize, fill: [u8; 3]) -> RawImage {
        let pixels = fill.iter().cycle().take(width * height * 3).copied().collect();
        RawImage {
            pixels: RawImageData::U8(pixels),
            width,
            height,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        }
    }

    /// Encode an image the way an input file would carry it.
    fn encode(image: &RawImage, format: OutputImageFormat) -> Vec<u8> {
        image.encode_to_bytes(&[format]).expect("encode fixture image").0
    }

    fn png_bytes(width: usize, height: usize, fill: [u8; 3]) -> Vec<u8> {
        encode(&solid(width, height, fill), OutputImageFormat::Png)
    }

    /// The same PNG with a pHYs chunk claiming the given dots per inch. The
    /// chunk goes right after IHDR, where readers look for it.
    fn png_at_dpi(png: &[u8], dpi: f32) -> Vec<u8> {
        let dots_per_meter = (f64::from(dpi) / 0.0254).round() as u32;
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&9u32.to_be_bytes());
        chunk.extend_from_slice(b"pHYs");
        chunk.extend_from_slice(&dots_per_meter.to_be_bytes());
        chunk.extend_from_slice(&dots_per_meter.to_be_bytes());
        chunk.push(1); // unit: dots per meter
        chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());

        let mut out = png[..33].to_vec(); // signature + IHDR
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&png[33..]);
        out
    }

    /// CRC-32 (IEEE), as PNG chunks require.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A minimal JPEG header: SOI, a JFIF APP0 with the given density, and a
    /// frame marker with the given size, precision and component count. No
    /// scan data follows — enough for `jpeg_info`, far too little to decode.
    fn jpeg_header_full(
        width: u16,
        height: u16,
        units: u8,
        density: u16,
        components: u8,
        frame: u8,
        precision: u8,
    ) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]); // APP0, 16 bytes
        bytes.extend_from_slice(b"JFIF\0");
        bytes.extend_from_slice(&[0x01, 0x02, units]);
        bytes.extend_from_slice(&density.to_be_bytes());
        bytes.extend_from_slice(&density.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x00]); // zero thumbnail bytes
        bytes.extend_from_slice(&[0xFF, frame]);
        bytes.extend_from_slice(&((8 + 3 * components as u16).to_be_bytes())); // segment length
        bytes.push(precision);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.push(components);
        for id in 1..=components {
            bytes.extend_from_slice(&[id, 0x11, 0x00]); // sampling factors, quantization table
        }
        bytes
    }

    fn jpeg_header(width: u16, height: u16, units: u8, density: u16) -> Vec<u8> {
        jpeg_header_full(width, height, units, density, 1, 0xC0, 8)
    }

    /// A PNG header claiming the given size, with no pixel data behind it —
    /// enough to exercise the size gate, far too little to decode.
    fn png_huge_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&13u32.to_be_bytes());
        chunk.extend_from_slice(b"IHDR");
        chunk.extend_from_slice(&width.to_be_bytes());
        chunk.extend_from_slice(&height.to_be_bytes());
        chunk.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB, no interlace
        chunk.extend_from_slice(&crc32(&chunk[4..]).to_be_bytes());
        bytes.extend_from_slice(&chunk);
        let mut end = Vec::new();
        end.extend_from_slice(&0u32.to_be_bytes());
        end.extend_from_slice(b"IEND");
        end.extend_from_slice(&crc32(&end[4..]).to_be_bytes());
        bytes.extend_from_slice(&end);
        bytes
    }

    /// The MediaBox of page `n` (1-based) in the saved PDF, in points.
    /// MediaBox is inheritable, so the page tree is walked up like a reader
    /// would.
    fn media_box(path: &str, page: u32) -> [f64; 4] {
        let doc = Document::load(path).expect("reopen the output pdf");
        let mut id = *doc.get_pages().get(&page).expect("page exists");
        let rect = loop {
            let dict = doc.get_dictionary(id).expect("page dict");
            if let Ok(box_) = dict.get(b"MediaBox").and_then(Object::as_array) {
                break box_.clone();
            }
            id = dict
                .get(b"Parent")
                .and_then(Object::as_reference)
                .expect("page parent");
        };
        rect.iter()
            .map(|o| f64::from(o.as_float().expect("box numbers")))
            .collect::<Vec<_>>()
            .try_into()
            .expect("4 numbers")
    }

    /// The page's content stream text, for reading back the placement ops.
    fn page_content(path: &str, page: u32) -> String {
        let doc = Document::load(path).expect("reopen the output pdf");
        let id = *doc.get_pages().get(&page).expect("page exists");
        String::from_utf8_lossy(&doc.get_page_content(id).expect("page content")).into_owned()
    }

    /// The `[a b c d e f]` of the `cm` right before the `Do` painting the
    /// image: `a`/`d` are the drawn size in points, `e`/`f` the position of
    /// the image's lower-left corner.
    fn image_matrix(content: &str) -> [f64; 6] {
        let tokens: Vec<&str> = content.split_whitespace().collect();
        let draw = tokens
            .iter()
            .position(|t| *t == "Do")
            .expect("the image is drawn");
        let cm = tokens[..draw]
            .iter()
            .rposition(|t| *t == "cm")
            .expect("Do is preceded by cm");
        let mut matrix = [0.0; 6];
        for (slot, token) in matrix.iter_mut().zip(&tokens[cm - 6..cm]) {
            *slot = token.parse().expect("cm operands are numbers");
        }
        matrix
    }

    /// Matrix comparison with a tolerance: printpdf works in f32.
    fn assert_matrix(got: [f64; 6], want: [f64; 6]) {
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 0.02, "got {got:?}, want {want:?}");
        }
    }

    /// The `[a b c d e f]` of the first `cm` in the content stream: the
    /// EXIF-rotated path writes its placement matrix before `UseXobject`'s
    /// own identity matrix.
    fn first_cm(content: &str) -> [f64; 6] {
        let tokens: Vec<&str> = content.split_whitespace().collect();
        let cm = tokens.iter().position(|t| *t == "cm").expect("a cm operator");
        let mut matrix = [0.0; 6];
        for (slot, token) in matrix.iter_mut().zip(&tokens[cm - 6..cm]) {
            *slot = token.parse().expect("cm operands are numbers");
        }
        matrix
    }

    /// The same picture as a grayscale JPEG.
    fn gray_jpeg(width: usize, height: usize, fill: u8) -> Vec<u8> {
        let image = RawImage {
            pixels: RawImageData::U8(vec![fill; width * height]),
            width,
            height,
            data_format: RawImageFormat::R8,
            tag: Vec::new(),
        };
        encode(&image, OutputImageFormat::Jpeg)
    }

    /// The same JPEG with an EXIF orientation tag spliced in right after SOI.
    fn jpeg_with_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II"); // little-endian
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes()); // the IFD follows the header
        tiff.extend_from_slice(&1u16.to_le_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_le_bytes()); // tag: Orientation
        tiff.extend_from_slice(&3u16.to_le_bytes()); // type: SHORT
        tiff.extend_from_slice(&1u32.to_le_bytes()); // count
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes()); // pad the value field to 4 bytes
        tiff.extend_from_slice(&0u32.to_le_bytes()); // no next IFD

        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = jpeg[..2].to_vec(); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    /// The document's image XObject stream, exactly as it is stored.
    fn image_stream(path: &str) -> lopdf::Stream {
        let doc = Document::load(path).expect("reopen the output pdf");
        for object in doc.objects.values() {
            if let Ok(stream) = object.as_stream() {
                let is_image =
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(&b"Image"[..]);
                if is_image {
                    return stream.clone();
                }
            }
        }
        panic!("the pdf holds no image xobject");
    }

    /// The document's image XObject: its dictionary and decompressed pixels.
    fn embedded_image(path: &str) -> (Dictionary, Vec<u8>) {
        let stream = image_stream(path);
        (
            stream.dict.clone(),
            stream.decompressed_content().expect("flate pixels"),
        )
    }

    #[test]
    fn deserializes_the_options_the_frontend_sends() {
        let json = r##"{"pageSize":"a4","fit":"contain"}"##;
        let options: Options = serde_json::from_str(json).expect("contract is camelCase");
        assert_eq!(options.page_size, PageSize::A4);
        assert_eq!(options.fit, Fit::Contain);

        for bad in [
            r##"{"pageSize":"a3","fit":"contain"}"##,
            r##"{"pageSize":"a4","fit":"crop"}"##,
            r##"{"fit":"contain"}"##,
        ] {
            assert!(serde_json::from_str::<Options>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn one_page_per_image_and_the_report_counts_them() {
        let f = fixture("pages");
        let inputs = vec![
            f.image("a.png", &png_bytes(10, 20, [255, 0, 0])),
            f.image("b.png", &png_bytes(30, 40, [0, 255, 0])),
            f.image("c.png", &png_bytes(50, 60, [0, 0, 255])),
        ];

        let report = images_to_pdf(&inputs, &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        // The acceptance bar: lopdf reopens the result and finds the pages.
        let doc = Document::load(&f.output).expect("the output reopens with lopdf");
        assert_eq!(doc.get_pages().len(), 3, "one page per image");
        assert_eq!(report.pages, 3);
        assert_eq!(report.output, f.output);
        assert_eq!(
            report.output_bytes,
            std::fs::metadata(&f.output).unwrap().len(),
            "the report must not lie about the size"
        );
    }

    #[test]
    fn page_sizing_follows_the_image_density() {
        let f = fixture("density");
        let png = png_at_dpi(&png_bytes(300, 600, [1, 2, 3]), 300.0);
        let input = f.image("scan.png", &png);

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        // 300 px at 300 dpi is exactly one inch = 72 pt.
        let rect = media_box(&f.output, 1);
        assert!((rect[2] - 72.0).abs() < 0.01, "got {rect:?}");
        assert!((rect[3] - 144.0).abs() < 0.01, "got {rect:?}");
    }

    #[test]
    fn page_sizing_falls_back_to_96_dpi() {
        let f = fixture("dpi_fallback");
        let input = f.image("plain.png", &png_bytes(96, 192, [9, 9, 9]));

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Stretch))
            .expect("build should succeed");

        // No density metadata: 96 px are one inch at the 96 dpi default.
        let rect = media_box(&f.output, 1);
        assert!((rect[2] - 72.0).abs() < 0.01, "got {rect:?}");
        assert!((rect[3] - 144.0).abs() < 0.01, "got {rect:?}");
    }

    #[test]
    fn fixed_pages_take_the_orientation_of_the_image() {
        let f = fixture("orientation");
        let wide = f.image("wide.png", &png_bytes(200, 100, [1, 1, 1]));
        let tall = f.image("tall.png", &png_bytes(100, 200, [2, 2, 2]));

        images_to_pdf(&[wide], &f.output, options(PageSize::A4, Fit::Contain)).expect("build should succeed");
        let rect = media_box(&f.output, 1);
        assert!(
            (rect[2] - 841.89).abs() < 0.05,
            "landscape photo, landscape sheet: {rect:?}"
        );
        assert!((rect[3] - 595.276).abs() < 0.05, "got {rect:?}");

        images_to_pdf(&[tall], &f.output, options(PageSize::Letter, Fit::Contain))
            .expect("build should succeed");
        let rect = media_box(&f.output, 1);
        assert!(
            (rect[2] - 612.0).abs() < 0.05,
            "portrait photo, portrait sheet: {rect:?}"
        );
        assert!((rect[3] - 792.0).abs() < 0.05, "got {rect:?}");
    }

    #[test]
    fn contain_letterboxes_the_image_on_the_page() {
        let f = fixture("contain");
        // 100 × 200 px on Letter: 3.96 pt/px fits the height exactly and
        // leaves (612 − 396) / 2 = 108 pt of margin on each side.
        let input = f.image("tall.png", &png_bytes(100, 200, [3, 3, 3]));

        images_to_pdf(&[input], &f.output, options(PageSize::Letter, Fit::Contain))
            .expect("build should succeed");

        let content = page_content(&f.output, 1);
        assert!(
            !content.split_whitespace().any(|t| t == "W"),
            "contain never overflows, so it never clips: {content}"
        );
        assert_matrix(image_matrix(&content), [396.0, 0.0, 0.0, 792.0, 108.0, 0.0]);
    }

    #[test]
    fn cover_crops_the_overflow_with_a_clip() {
        let f = fixture("cover");
        // The same picture filling the width (6.12 pt/px) overshoots the
        // height by 432 pt, evenly top and bottom.
        let input = f.image("tall.png", &png_bytes(100, 200, [4, 4, 4]));

        images_to_pdf(&[input], &f.output, options(PageSize::Letter, Fit::Cover))
            .expect("build should succeed");

        let content = page_content(&f.output, 1);
        assert!(
            content.split_whitespace().any(|t| t == "W"),
            "cover clips the overflow away: {content}"
        );
        assert_matrix(image_matrix(&content), [612.0, 0.0, 0.0, 1224.0, 0.0, -216.0]);
    }

    #[test]
    fn stretch_fills_the_page_exactly() {
        let f = fixture("stretch");
        let input = f.image("tall.png", &png_bytes(100, 200, [5, 5, 5]));

        images_to_pdf(&[input], &f.output, options(PageSize::Letter, Fit::Stretch))
            .expect("build should succeed");

        assert_matrix(
            image_matrix(&page_content(&f.output, 1)),
            [612.0, 0.0, 0.0, 792.0, 0.0, 0.0],
        );
    }

    #[test]
    fn png_pixels_land_in_the_pdf_byte_for_byte() {
        let f = fixture("png_embed");
        // Two rows of two pixels — distinct rows, so a row flip would show.
        let pixels = vec![255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 255, 0];
        let image = RawImage {
            pixels: RawImageData::U8(pixels.clone()),
            width: 2,
            height: 2,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };
        let input = f.image("pixels.png", &encode(&image, OutputImageFormat::Png));

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        let (dict, embedded) = embedded_image(&f.output);
        assert_eq!(embedded, pixels, "lossless: the samples survive untouched");
        assert_eq!(dict.get(b"Width").and_then(Object::as_i64).ok(), Some(2));
        assert_eq!(dict.get(b"Height").and_then(Object::as_i64).ok(), Some(2));
    }

    #[test]
    fn jpeg_input_is_embedded_as_the_same_picture() {
        let f = fixture("jpeg_embed");
        // Two distinct rows: the embedded picture must be the source picture,
        // not just any picture of the right size.
        let source = RawImage {
            pixels: RawImageData::U8(vec![255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255]),
            width: 2,
            height: 2,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };
        let jpeg = encode(&source, OutputImageFormat::Jpeg);
        let input = f.image("photo.jpg", &jpeg);

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        // Byte for byte: the file's own JPEG stream is what the PDF carries.
        let stream = image_stream(&f.output);
        assert_eq!(stream.content, jpeg, "the JPEG bytes survive untouched");
        assert_eq!(
            stream.dict.get(b"Filter").and_then(Object::as_name).ok(),
            Some(&b"DCTDecode"[..]),
            "the reader decodes the JPEG"
        );
        assert_eq!(
            stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceRGB"[..])
        );
        assert_eq!(stream.dict.get(b"Width").and_then(Object::as_i64).ok(), Some(2));
        assert_eq!(stream.dict.get(b"Height").and_then(Object::as_i64).ok(), Some(2));
    }

    #[test]
    fn grayscale_jpeg_is_embedded_byte_for_byte() {
        let f = fixture("gray_embed");
        let jpeg = gray_jpeg(4, 2, 128);
        let input = f.image("gray.jpg", &jpeg);

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        let stream = image_stream(&f.output);
        assert_eq!(stream.content, jpeg, "the JPEG bytes survive untouched");
        assert_eq!(
            stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceGray"[..]),
            "one component is drawn as gray, not widened to RGB"
        );
        assert_eq!(stream.dict.get(b"Width").and_then(Object::as_i64).ok(), Some(4));
        assert_eq!(stream.dict.get(b"Height").and_then(Object::as_i64).ok(), Some(2));
    }

    #[test]
    fn cmyk_jpeg_is_embedded_byte_for_byte() {
        let f = fixture("cmyk_embed");
        // A CMYK frame header. The bytes ride straight through — no decode —
        // so the header is all the fixture needs.
        let jpeg = jpeg_header_full(8, 8, 1, 72, 4, 0xC0, 8);
        let input = f.image("cmyk.jpg", &jpeg);

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        let stream = image_stream(&f.output);
        assert_eq!(stream.content, jpeg, "the JPEG bytes survive untouched");
        assert_eq!(
            stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceCMYK"[..])
        );
        assert_eq!(
            stream.dict.get(b"BitsPerComponent").and_then(Object::as_i64).ok(),
            Some(8)
        );
    }

    #[test]
    fn jpeg_with_exif_rotation_is_drawn_upright() {
        let f = fixture("exif_rotated");
        // Stored 2 wide × 3 tall; the tag says the picture stands 3 × 2.
        let source = RawImage {
            pixels: RawImageData::U8(vec![0; 2 * 3 * 3]),
            width: 2,
            height: 3,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };
        let jpeg = jpeg_with_orientation(&encode(&source, OutputImageFormat::Jpeg), 6);
        let input = f.image("turned.jpg", &jpeg);

        images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain))
            .expect("build should succeed");

        // The page follows the displayed size: 3 × 2 px at 96 dpi is 2.25 × 1.5 pt.
        let rect = media_box(&f.output, 1);
        assert!((rect[2] - 2.25).abs() < 0.01, "got {rect:?}");
        assert!((rect[3] - 1.5).abs() < 0.01, "got {rect:?}");

        // The stored top edge runs down the page's right edge: the unit
        // square's top left (0, 1) lands at the top right corner.
        assert_matrix(
            first_cm(&page_content(&f.output, 1)),
            [0.0, -1.5, 2.25, 0.0, 0.0, 1.5],
        );
        assert_eq!(image_stream(&f.output).content, jpeg, "EXIF and all");
    }

    #[test]
    fn exif_orientation_is_read_from_the_tag() {
        let jpeg = jpeg_with_orientation(&jpeg_header(3, 5, 1, 72), 6);
        let (_, details) = jpeg_info(&jpeg).expect("header parses");
        assert_eq!(details.orientation, 6);
        assert_eq!((details.components, details.precision), (1, 8));

        let (_, details) = jpeg_info(&jpeg_header(3, 5, 1, 72)).expect("header parses");
        assert_eq!(details.orientation, 1, "no EXIF means upright");

        // A tag outside 1..=8 is not a rotation.
        let jpeg = jpeg_with_orientation(&jpeg_header(3, 5, 1, 72), 9);
        let (_, details) = jpeg_info(&jpeg).expect("header parses");
        assert_eq!(details.orientation, 1);
    }

    #[test]
    fn orientation_matrices_turn_the_stored_picture() {
        // Where a point of the unit square lands, stored top left at (0, 1).
        let map = |orientation: u8, x: f64, y: f64| -> (f64, f64) {
            let m = orientation_matrix(orientation);
            (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
        };
        // A quarter turn clockwise: the stored top edge becomes the right edge.
        assert_eq!(map(6, 0.0, 1.0), (1.0, 1.0));
        assert_eq!(map(6, 0.0, 0.0), (0.0, 1.0));
        // Upside down: the stored bottom left is the displayed top right.
        assert_eq!(map(3, 0.0, 0.0), (1.0, 1.0));
        // Mirrored left–right keeps the top row on top.
        assert_eq!(map(2, 0.0, 1.0), (1.0, 1.0));
        // 1 is the identity.
        assert_eq!(map(1, 1.0, 1.0), (1.0, 1.0));
    }

    #[test]
    fn rejects_jpeg_variants_the_pdf_way_cannot_carry() {
        for (name, jpeg, want) in [
            (
                // SOF3 is the lossless process; readers only take the
                // baseline, extended-sequential and progressive frames.
                "lossless.jpg",
                jpeg_header_full(4, 4, 1, 72, 1, 0xC3, 8),
                "baseline or progressive",
            ),
            ("deep.jpg", jpeg_header_full(4, 4, 1, 72, 1, 0xC0, 12), "12-bit"),
            (
                "duo.jpg",
                jpeg_header_full(4, 4, 1, 72, 2, 0xC0, 8),
                "2 color components",
            ),
        ] {
            let f = fixture("exotic_jpeg");
            let input = f.image(name, &jpeg);
            let err = images_to_pdf(&[input], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();
            assert!(err.contains(want), "got {err}");
            assert!(!Path::new(&f.output).exists(), "no output on failure");
        }
    }

    #[test]
    fn rejects_an_empty_input_list() {
        let f = fixture("empty_list");
        let err = images_to_pdf(&[], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();
        assert!(err.contains("at least one image"), "got {err}");
    }

    #[test]
    fn refuses_to_overwrite_its_own_inputs() {
        let f = fixture("same_path");
        let input = f.image("keep.png", &png_bytes(4, 4, [7, 7, 7]));

        let err = images_to_pdf(
            std::slice::from_ref(&input),
            &input,
            options(PageSize::Image, Fit::Contain),
        )
        .unwrap_err();

        assert!(err.contains("different file"), "got {err}");
    }

    #[test]
    fn rejects_a_file_that_is_not_an_image() {
        let f = fixture("not_image");

        let text = f.image("notes.txt", b"just text");
        let err = images_to_pdf(&[text], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();
        assert!(err.contains("Not a PNG or JPEG"), "got {err}");

        // Right extension, wrong bytes: the header check is the real gate.
        let fake = f.image("fake.png", b"\x89PNG but not really");
        let err = images_to_pdf(&[fake], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();
        assert!(err.contains("not a PNG or JPEG"), "got {err}");
    }

    #[test]
    fn rejects_an_empty_file() {
        let f = fixture("empty_file");
        let zero = f.image("zero.png", b"");

        let err = images_to_pdf(&[zero], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();

        assert!(err.contains("empty"), "got {err}");
    }

    #[test]
    fn rejects_an_oversized_image() {
        let f = fixture("oversized");
        // A header claiming 20 000 × 20 000 px = 400 MP, with no pixels
        // behind it: the size gate must fire before anything decodes.
        let huge = f.image("huge.png", &png_huge_header(20_000, 20_000));

        let err = images_to_pdf(&[huge], &f.output, options(PageSize::Image, Fit::Contain)).unwrap_err();

        assert!(err.contains("too large"), "got {err}");
        assert!(!Path::new(&f.output).exists(), "no output on failure");
    }

    #[test]
    fn rejects_a_jpeg_without_a_frame_marker() {
        // SOI, then straight to EOI: no frame marker, so no size.
        let err = image_info(&[0xFF, 0xD8, 0xFF, 0xD9]).unwrap_err();
        assert!(err.contains("damaged"), "got {err}");
    }

    #[test]
    fn a_failed_save_leaves_no_file_behind() {
        let f = fixture("failed_save");
        let input = f.image("ok.png", &png_bytes(4, 4, [8, 8, 8]));
        let output = f.dir.join("missing-dir").join("out.pdf");

        let err = images_to_pdf(
            &[input],
            output.to_str().unwrap(),
            options(PageSize::Image, Fit::Contain),
        )
        .unwrap_err();

        assert!(err.contains("Failed to write"), "got {err}");
        assert!(!output.exists(), "a failed run must not leave a partial file");
    }

    #[test]
    fn reads_png_sizes_and_density() {
        let png = png_bytes(12, 34, [0, 0, 0]);
        let info = image_info(&png).expect("valid png header");
        assert_eq!((info.width_px, info.height_px), (12, 34));
        assert_eq!(info.dpi, DEFAULT_DPI, "no pHYs: the 96 dpi default");

        let info = image_info(&png_at_dpi(&png, 300.0)).expect("valid png header");
        assert!((info.dpi - 300.0).abs() < 0.5, "got {}", info.dpi);
    }

    #[test]
    fn reads_jpeg_sizes_and_density() {
        let info = image_info(&jpeg_header(640, 480, 1, 300)).expect("valid jpeg header");
        assert_eq!((info.width_px, info.height_px), (640, 480));
        assert_eq!(info.dpi, 300.0);

        // Units 2 is dots per centimeter.
        let info = image_info(&jpeg_header(10, 20, 2, 100)).expect("valid jpeg header");
        assert!((info.dpi - 254.0).abs() < 0.01, "got {}", info.dpi);

        // Units 0 is an aspect ratio only — fall back to the default.
        let info = image_info(&jpeg_header(10, 20, 0, 1)).expect("valid jpeg header");
        assert_eq!(info.dpi, DEFAULT_DPI);
    }

    #[test]
    fn placement_math_centers_and_scales() {
        // A 2 × 4 px image on a 10 × 20 pt page has the same aspect ratio:
        // every strategy lands on the full-page rectangle.
        for fit in [Fit::Contain, Fit::Cover, Fit::Stretch] {
            let place = placement(2, 4, (10.0, 20.0), fit);
            let want = Placement {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 20.0,
            };
            assert_eq!(place, want, "{fit:?}");
        }

        // On a square page, contain stops at the height and centers on x;
        // cover fills the width and lets the height hang off both edges.
        assert_eq!(
            placement(2, 4, (20.0, 20.0), Fit::Contain),
            Placement {
                x: 5.0,
                y: 0.0,
                width: 10.0,
                height: 20.0
            }
        );
        assert_eq!(
            placement(2, 4, (20.0, 20.0), Fit::Cover),
            Placement {
                x: 0.0,
                y: -10.0,
                width: 20.0,
                height: 40.0
            }
        );
    }
}
