//! Image watermarks stamped over every page.
//!
//! Same architecture as the text watermark ([`crate::watermark`], research
//! doc `docs/research/m2-render-engine.md` §3.1): the stamp is a fresh
//! content stream appended to each page's `/Contents` and the original
//! content bytes are never touched. The picture is one shared Image XObject
//! per document — Flate samples, PNG transparency kept as a soft mask —
//! painted with `Do` under the same ExtGState opacity trick as text.

use std::path::Path;

use lopdf::{dictionary, Dictionary, Document, Object, Stream};
use printpdf::{RawImage, RawImageData, RawImageFormat};

use crate::convert::read_image;
use crate::pdf::{file_label, validate_pdf_path};
use crate::watermark::{
    append_content_stream, effective_resources, num, page_rect, stamp_angle, stamp_centers, sub_dict,
    unique_name, Position, Report,
};

/// Everything needed to stamp one picture onto a document.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageWatermark {
    /// Path to the watermark picture: PNG or JPEG.
    pub image: String,
    /// 0 exclusive, 1 inclusive; painted through an ExtGState.
    pub opacity: f32,
    /// Degrees counter-clockwise; `Position::Diagonal` pins this at 45°.
    pub rotation: f32,
    pub position: Position,
    /// The stamp's width as a percentage of the page width.
    pub scale: f32,
}

impl ImageWatermark {
    /// Bounds that keep the stamp sane. Every message is shown to a user.
    fn validate(&self) -> Result<(), String> {
        if !self.opacity.is_finite() || self.opacity <= 0.0 || self.opacity > 1.0 {
            return Err("Opacity must be above 0 and at most 1.".into());
        }
        if !self.scale.is_finite() || !(1.0..=200.0).contains(&self.scale) {
            return Err("Size must be between 1 and 200 percent.".into());
        }
        if !self.rotation.is_finite() || !(-360.0..=360.0).contains(&self.rotation) {
            return Err("Angle must be between -360 and 360 degrees.".into());
        }
        Ok(())
    }
}

/// The watermark picture, flattened to the samples an Image XObject wants.
struct Decoded {
    width: usize,
    height: usize,
    /// Gray or RGB samples, interleaved, alpha taken out.
    samples: Vec<u8>,
    color: ColorSpace,
    /// The alpha channel as its own plane, when the picture has one.
    alpha: Option<Vec<u8>>,
}

/// The color model of [`Decoded::samples`].
#[derive(Debug, Clone, Copy, PartialEq)]
enum ColorSpace {
    Gray,
    Rgb,
}

/// Read and decode the watermark picture.
///
/// Empty, foreign and oversized files are refused before any pixel buffer is
/// allocated — `convert::read_image` is the shared gate.
fn decode_image(path: &str) -> Result<Decoded, String> {
    let (bytes, _) = read_image(path)?;
    let label = file_label(path);
    let mut warnings = Vec::new();
    let raw = RawImage::decode_from_bytes(&bytes, &mut warnings)
        .map_err(|e| format!("Failed to read {label}: {e}"))?;
    flatten(raw, &label)
}

/// Split a decoded picture into color samples and an alpha plane, narrowing
/// 16-bit samples to the 8 bits a stamp needs.
fn flatten(raw: RawImage, label: &str) -> Result<Decoded, String> {
    let unsupported = || format!("{label} has a color format this watermark does not support.");
    if raw.width == 0 || raw.height == 0 {
        return Err(format!("{label} has no pixels."));
    }
    let samples = match raw.pixels {
        RawImageData::U8(v) => v,
        RawImageData::U16(v) => v.iter().map(|s| (s >> 8) as u8).collect(),
        RawImageData::F32(_) => return Err(unsupported()),
    };
    let (color, channels) = match raw.data_format {
        RawImageFormat::R8 | RawImageFormat::R16 => (ColorSpace::Gray, 1),
        RawImageFormat::RG8 | RawImageFormat::RG16 => (ColorSpace::Gray, 2),
        RawImageFormat::RGB8 | RawImageFormat::RGB16 => (ColorSpace::Rgb, 3),
        RawImageFormat::RGBA8 | RawImageFormat::RGBA16 => (ColorSpace::Rgb, 4),
        _ => return Err(unsupported()),
    };
    let has_alpha = channels % 2 == 0;
    let color_channels = if has_alpha { channels - 1 } else { channels };
    let mut color_samples = Vec::with_capacity(raw.width * raw.height * color_channels);
    let mut alpha = has_alpha.then(Vec::new);
    for pixel in samples.chunks_exact(channels) {
        color_samples.extend_from_slice(&pixel[..color_channels]);
        if let Some(alpha) = alpha.as_mut() {
            alpha.push(pixel[color_channels]);
        }
    }
    Ok(Decoded {
        width: raw.width,
        height: raw.height,
        samples: color_samples,
        color,
        alpha,
    })
}

/// The objects every page shares: the ExtGState carrying the opacity and the
/// picture as one Image XObject.
struct SharedObjects {
    gs: (u32, u16),
    image: (u32, u16),
}

/// Add the shared drawing objects to the document. The picture is stored
/// once and painted many times, whatever the position asks for.
fn add_shared_objects(doc: &mut Document, decoded: &Decoded, opacity: f32) -> SharedObjects {
    let gs = doc.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => opacity,
        "CA" => opacity,
    });
    let image = add_image_xobject(doc, decoded);
    SharedObjects { gs, image }
}

/// The picture as an Image XObject: Flate-compressed samples, with PNG
/// transparency preserved as a soft mask (per-pixel alpha is far too useful
/// for logos to flatten away).
fn add_image_xobject(doc: &mut Document, decoded: &Decoded) -> (u32, u16) {
    let smask = decoded.alpha.as_ref().map(|alpha| {
        let mut mask = Stream::new(
            image_dict(decoded.width, decoded.height, ColorSpace::Gray),
            alpha.clone(),
        );
        let _ = mask.compress();
        doc.add_object(Object::Stream(mask))
    });
    let mut dict = image_dict(decoded.width, decoded.height, decoded.color);
    if let Some(id) = smask {
        dict.set("SMask", Object::Reference(id));
    }
    let mut stream = Stream::new(dict, decoded.samples.clone());
    let _ = stream.compress();
    doc.add_object(Object::Stream(stream))
}

/// The unchanging half of an image XObject dictionary.
fn image_dict(width: usize, height: usize, color: ColorSpace) -> Dictionary {
    let color_space = match color {
        ColorSpace::Gray => "DeviceGray",
        ColorSpace::Rgb => "DeviceRGB",
    };
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => width as i64,
        "Height" => height as i64,
        "BitsPerComponent" => 8i64,
        "ColorSpace" => color_space,
        "Interpolate" => Object::Boolean(false),
    }
}

/// The resource names the watermark adds to one page.
struct ResNames {
    gs: String,
    image: String,
}

/// Wire the shared objects into the page's resources.
///
/// The page gets its own merged copy of whatever resources it effectively
/// had (its own, or an inherited `/Resources`) — a referenced resources dict
/// is often shared by sibling pages, and renaming entries there would be a
/// surprise to all of them (same reason as the text watermark).
fn install_page_resources(
    doc: &mut Document,
    page_id: (u32, u16),
    shared: &SharedObjects,
) -> Result<ResNames, String> {
    let mut resources = effective_resources(doc, page_id);

    let mut states = sub_dict(doc, &resources, b"ExtGState");
    let gs = unique_name(&states, "PbWmGS");
    states.set(gs.clone(), Object::Reference(shared.gs));
    resources.set("ExtGState", states);

    let mut xobjects = sub_dict(doc, &resources, b"XObject");
    let image = unique_name(&xobjects, "PbWmIm");
    xobjects.set(image.clone(), Object::Reference(shared.image));
    resources.set("XObject", xobjects);

    doc.get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| format!("Could not add watermark resources: {e}"))?
        .set("Resources", resources);

    Ok(ResNames {
        gs: String::from_utf8_lossy(&gs).into_owned(),
        image: String::from_utf8_lossy(&image).into_owned(),
    })
}

/// The matrix sending the stamp's unit square onto a `width_pt` × `height_pt`
/// rectangle turned by `angle` degrees around its visual center at `(cx, cy)`.
fn stamp_matrix(angle_deg: f64, cx: f64, cy: f64, width_pt: f64, height_pt: f64) -> [f64; 6] {
    let rad = angle_deg.to_radians();
    let (sin, cos) = (rad.sin(), rad.cos());
    let (a, b) = (width_pt * cos, width_pt * sin);
    let (c, d) = (-height_pt * sin, height_pt * cos);
    let e = cx - 0.5 * (a + c);
    let f = cy - 0.5 * (b + d);
    [a, b, c, d, e, f]
}

/// The content-stream fragment drawing one stamp. Wrapped in `q…Q` so no
/// graphics state leaks into whatever draws next.
fn stamp_ops(names: &ResNames, matrix: [f64; 6]) -> String {
    format!(
        "q\n/{} gs\n{} {} {} {} {} {} cm\n/{} Do\nQ\n",
        names.gs,
        num(matrix[0]),
        num(matrix[1]),
        num(matrix[2]),
        num(matrix[3]),
        num(matrix[4]),
        num(matrix[5]),
        names.image,
    )
}

/// Stamp `wm` onto every page of `input`, writing the result to `output`.
pub fn watermark_pdf(input: &str, output: &str, wm: &ImageWatermark) -> Result<Report, String> {
    validate_pdf_path(input)?;
    wm.validate()?;
    if Path::new(output) == Path::new(input) {
        return Err("Choose a different file for the output.".into());
    }

    // The picture is decoded before the document is touched: a bad picture
    // must fail the run outright, like a bad input PDF does.
    let decoded = decode_image(&wm.image)?;

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

    let shared = add_shared_objects(&mut doc, &decoded, wm.opacity);
    let pages: Vec<(u32, (u32, u16))> = doc.get_pages().into_iter().collect();

    for (number, page_id) in &pages {
        let rect = page_rect(&doc, *page_id).map_err(|e| format!("Page {number}: {e}"))?;
        let names = install_page_resources(&mut doc, *page_id, &shared)?;
        // The stamp's width is a share of the page's own width; the height
        // keeps the picture's aspect ratio.
        let width_pt = (rect.x1 - rect.x0) * f64::from(wm.scale) / 100.0;
        let height_pt = width_pt * decoded.height as f64 / decoded.width as f64;
        let angle = stamp_angle(wm.position, f64::from(wm.rotation), rect.rotate);

        let mut content = String::new();
        for (cx, cy) in stamp_centers(&rect, wm.position, width_pt, height_pt) {
            content.push_str(&stamp_ops(
                &names,
                stamp_matrix(angle, cx, cy, width_pt, height_pt),
            ));
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

    use lopdf::Object;
    use printpdf::{OutputImageFormat, RawImageData, RawImageFormat};

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
        let dir = std::env::temp_dir().join(format!("paperblade_watermark_image_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let path = |n: &str| dir.join(n).to_str().unwrap().to_string();
        let input = path("input.pdf");
        write_pdf_with_content(&input, pages, Some(ORIGINAL));
        let output = path("output.pdf");
        Fixture { dir, input, output }
    }

    impl Fixture {
        /// Write `bytes` as `name` and return its path.
        fn image(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.dir.join(name);
            std::fs::write(&path, bytes).expect("write fixture image");
            path.to_str().unwrap().to_string()
        }
    }

    fn draft(image: &str) -> ImageWatermark {
        ImageWatermark {
            image: image.into(),
            opacity: 0.3,
            rotation: 0.0,
            position: Position::Center,
            scale: 50.0,
        }
    }

    /// An RGB PNG of the given size with flat `fill` pixels.
    fn png(width: usize, height: usize, fill: [u8; 3]) -> Vec<u8> {
        let pixels = fill.iter().cycle().take(width * height * 3).copied().collect();
        let raw = RawImage {
            pixels: RawImageData::U8(pixels),
            width,
            height,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };
        raw.encode_to_bytes(&[OutputImageFormat::Png])
            .expect("encode fixture image")
            .0
    }

    /// A grayscale JPEG of the given size.
    fn gray_jpeg(width: usize, height: usize, fill: u8) -> Vec<u8> {
        let raw = RawImage {
            pixels: RawImageData::U8(vec![fill; width * height]),
            width,
            height,
            data_format: RawImageFormat::R8,
            tag: Vec::new(),
        };
        raw.encode_to_bytes(&[OutputImageFormat::Jpeg])
            .expect("encode fixture image")
            .0
    }

    /// The concatenated content streams of every page in a saved PDF.
    fn page_contents(path: &str) -> Vec<String> {
        let doc = Document::load(path).expect("reopen the watermarked pdf");
        doc.get_pages()
            .values()
            .map(|id| String::from_utf8_lossy(&doc.get_page_content(*id).unwrap()).into_owned())
            .collect()
    }

    /// The operands of the first `cm` operator in a content stream.
    fn first_cm(content: &str) -> [f64; 6] {
        let line = content
            .lines()
            .find(|l| l.ends_with("cm"))
            .expect("the stamp writes a matrix");
        let mut values = [0.0; 6];
        for (slot, token) in values.iter_mut().zip(line.split_whitespace()) {
            *slot = token.parse().expect("cm operands are numbers");
        }
        values
    }

    /// The watermark's Image XObject stream in the saved PDF (not its soft
    /// mask — that is an image stream too).
    fn image_xobject(path: &str) -> lopdf::Stream {
        let doc = Document::load(path).expect("reopen the watermarked pdf");
        let masks: Vec<u32> = doc
            .objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .filter_map(|s| s.dict.get(b"SMask").ok()?.as_reference().ok())
            .map(|id| id.0)
            .collect();
        doc.objects
            .iter()
            .find_map(|(id, object)| {
                let stream = object.as_stream().ok()?;
                let is_image =
                    stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(&b"Image"[..]);
                (is_image && !masks.contains(&id.0)).then(|| stream.clone())
            })
            .expect("the watermark picture is embedded")
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

    #[test]
    fn deserializes_the_options_the_frontend_sends() {
        let json = r##"{"image":"/tmp/logo.png","opacity":0.2,"rotation":45,"position":"tiled","scale":25}"##;
        let wm: ImageWatermark = serde_json::from_str(json).expect("contract is camelCase");
        assert_eq!(wm.position, Position::Tiled);
        assert_eq!(wm.scale, 25.0);

        for bad in [
            r##"{"image":"/tmp/logo.png","opacity":0.2,"rotation":0,"position":"spread","scale":25}"##,
            r#"{"image":"/tmp/logo.png","opacity":0.2,"rotation":0,"position":"center"}"#,
        ] {
            assert!(serde_json::from_str::<ImageWatermark>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn appends_a_stream_without_touching_the_original_content() {
        let f = fixture("append", 1);
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

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
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        let report = watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let doc = Document::load(&f.output).expect("the watermarked pdf is structurally sound");
        assert_eq!(doc.get_pages().len(), 3);
        assert_eq!(report.pages, 3);
        assert_eq!(report.output, f.output);
        assert!(
            report.after_bytes > report.before_bytes,
            "stamping adds bytes; the report must not hide that"
        );
        for content in page_contents(&f.output) {
            assert!(content.contains("0.5 g 0 0 100 100 re f"), "original survives");
            assert!(content.contains("Do"), "the stamp is drawn");
        }
    }

    #[test]
    fn png_pixels_land_in_the_pdf_byte_for_byte() {
        let f = fixture("png_embed", 1);
        // Two rows of two pixels — distinct rows, so a row flip would show.
        let pixels = vec![255, 0, 0, 255, 0, 0, 0, 255, 0, 0, 255, 0];
        let raw = RawImage {
            pixels: RawImageData::U8(pixels.clone()),
            width: 2,
            height: 2,
            data_format: RawImageFormat::RGB8,
            tag: Vec::new(),
        };
        let png = raw.encode_to_bytes(&[OutputImageFormat::Png]).unwrap().0;
        let wm = draft(&f.image("logo.png", &png));

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let stream = image_xobject(&f.output);
        assert_eq!(stream.decompressed_content().unwrap(), pixels, "lossless");
        assert_eq!(stream.dict.get(b"Width").and_then(Object::as_i64).ok(), Some(2));
        assert_eq!(stream.dict.get(b"Height").and_then(Object::as_i64).ok(), Some(2));
        assert_eq!(
            stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceRGB"[..])
        );
    }

    #[test]
    fn jpeg_watermark_is_stored_as_samples() {
        let f = fixture("jpeg_embed", 1);
        let wm = draft(&f.image("logo.jpg", &gray_jpeg(2, 2, 128)));

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        // A stamp is painted many times from one small object, so the picture
        // is stored as samples rather than as its own JPEG stream.
        let stream = image_xobject(&f.output);
        assert_eq!(stream.decompressed_content().unwrap(), vec![128; 4]);
        assert_eq!(
            stream.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceGray"[..]),
            "a grayscale watermark stays grayscale"
        );
    }

    #[test]
    fn png_transparency_becomes_a_soft_mask() {
        let f = fixture("alpha", 1);
        // One pixel half see-through, one opaque: the mask keeps the
        // per-pixel alpha while the ExtGState scales the whole stamp.
        let raw = RawImage {
            pixels: RawImageData::U8(vec![255, 0, 0, 128, 0, 0, 255, 255]),
            width: 2,
            height: 1,
            data_format: RawImageFormat::RGBA8,
            tag: Vec::new(),
        };
        let png = raw.encode_to_bytes(&[OutputImageFormat::Png]).unwrap().0;
        let wm = draft(&f.image("logo.png", &png));

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let stream = image_xobject(&f.output);
        assert_eq!(
            stream.decompressed_content().unwrap(),
            vec![255, 0, 0, 0, 0, 255],
            "the color planes are the RGB part"
        );
        let mask_id = stream.dict.get(b"SMask").unwrap().as_reference().unwrap();
        let doc = Document::load(&f.output).unwrap();
        let mask = doc.get_object(mask_id).unwrap().as_stream().unwrap();
        assert_eq!(
            mask.decompressed_content().unwrap(),
            vec![128, 255],
            "the alpha plane"
        );
        assert_eq!(
            mask.dict.get(b"ColorSpace").and_then(Object::as_name).ok(),
            Some(&b"DeviceGray"[..])
        );
    }

    #[test]
    fn opacity_is_painted_through_an_extgstate() {
        let f = fixture("opacity", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
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
    fn scale_sizes_the_stamp_against_the_page_width() {
        let f = fixture("scale", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        wm.scale = 25.0;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        // 25% of the fixture's 612 pt page is 153 pt wide; the picture is
        // square, so it stands 153 pt tall too.
        let m = first_cm(&page_contents(&f.output)[0]);
        assert!((m[0] - 153.0).abs() < 0.01, "got {m:?}");
        assert!((m[3] - 153.0).abs() < 0.01, "got {m:?}");
    }

    #[test]
    fn centers_on_the_page() {
        let f = fixture("center", 1);
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        // The stamp's center must land on the page's center (306, 396).
        let m = first_cm(&page_contents(&f.output)[0]);
        assert!((m[4] + m[0] / 2.0 - 306.0).abs() < 0.01, "got {m:?}");
        assert!((m[5] + m[3] / 2.0 - 396.0).abs() < 0.01, "got {m:?}");
    }

    #[test]
    fn diagonal_sits_at_forty_five_degrees() {
        let f = fixture("diagonal", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        wm.position = Position::Diagonal;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let m = first_cm(&page_contents(&f.output)[0]);
        assert!((m[1].atan2(m[0]).to_degrees() - 45.0).abs() < 0.1, "got {m:?}");
    }

    #[test]
    fn rotation_is_written_into_the_matrix() {
        let f = fixture("rotation", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        wm.rotation = 30.0;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let m = first_cm(&page_contents(&f.output)[0]);
        assert!((m[1].atan2(m[0]).to_degrees() - 30.0).abs() < 0.1, "got {m:?}");
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
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let m = first_cm(&page_contents(&f.output)[0]);
        assert!((m[1].atan2(m[0]).to_degrees() + 90.0).abs() < 0.1, "got {m:?}");
    }

    #[test]
    fn tiled_repeats_the_stamp_across_the_page() {
        let f = fixture("tiled", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        wm.position = Position::Tiled;
        wm.scale = 20.0;

        watermark_pdf(&f.input, &f.output, &wm).expect("watermark should succeed");

        let stamps = page_contents(&f.output)[0].matches(" Do").count();
        assert!(stamps > 3, "a tiled watermark repeats, got {stamps} stamps");
    }

    #[test]
    fn rejects_a_watermark_file_that_is_not_an_image() {
        let f = fixture("not_image", 1);
        let wm = draft(&f.image("notes.txt", b"just text"));

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("Not a PNG or JPEG"), "got {err}");

        // Right extension, wrong bytes: the header check is the real gate.
        let wm = draft(&f.image("fake.png", b"\x89PNG but not really"));
        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("not a PNG or JPEG"), "got {err}");
        assert!(!Path::new(&f.output).exists(), "no output on failure");
    }

    #[test]
    fn rejects_an_oversized_watermark_image() {
        let f = fixture("oversized", 1);
        // A header claiming 20 000 × 20 000 px = 400 MP, with no pixels
        // behind it: the size gate must fire before anything decodes.
        let wm = draft(&f.image("huge.png", &png_huge_header(20_000, 20_000)));

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();

        assert!(err.contains("too large"), "got {err}");
        assert!(!Path::new(&f.output).exists(), "no output on failure");
    }

    #[test]
    fn rejects_opacity_outside_the_unit_interval() {
        let f = fixture("opacity_bounds", 1);
        let image = f.image("logo.png", &png(2, 2, [1, 2, 3]));
        for opacity in [0.0, 1.5] {
            let mut wm = draft(&image);
            wm.opacity = opacity;
            let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
            assert!(err.contains("Opacity"), "got {err}");
        }
    }

    #[test]
    fn rejects_a_scale_out_of_range() {
        let f = fixture("scale_bounds", 1);
        let image = f.image("logo.png", &png(2, 2, [1, 2, 3]));
        for scale in [0.5, 500.0] {
            let mut wm = draft(&image);
            wm.scale = scale;
            let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
            assert!(err.contains("Size"), "got {err}");
        }
    }

    #[test]
    fn rejects_an_angle_out_of_range() {
        let f = fixture("angle_bounds", 1);
        let mut wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        wm.rotation = 400.0;

        let err = watermark_pdf(&f.input, &f.output, &wm).unwrap_err();
        assert!(err.contains("Angle"), "got {err}");
    }

    #[test]
    fn refuses_to_overwrite_its_own_input() {
        let f = fixture("same_path", 1);
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));

        let err = watermark_pdf(&f.input, &f.input, &wm).unwrap_err();
        assert!(err.contains("different file"), "got {err}");
    }

    #[test]
    fn refuses_a_password_protected_file() {
        let f = fixture("encrypted", 1);
        let locked = f.dir.join("locked.pdf");
        let locked = locked.to_str().unwrap();
        crate::encrypt::encrypt_pdf(&f.input, locked, "hunter2").expect("make a locked fixture");
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));

        let err = watermark_pdf(locked, &f.output, &wm).unwrap_err();
        assert!(err.contains("password-protected"), "got {err}");
    }

    #[test]
    fn a_failed_save_leaves_no_file_behind() {
        let f = fixture("failed_save", 1);
        let wm = draft(&f.image("logo.png", &png(2, 2, [1, 2, 3])));
        let output = f.dir.join("missing-dir").join("out.pdf");

        let err = watermark_pdf(&f.input, output.to_str().unwrap(), &wm).unwrap_err();

        assert!(err.contains("Failed to write"), "got {err}");
        assert!(!output.exists(), "a failed run must not leave a partial file");
    }
}
