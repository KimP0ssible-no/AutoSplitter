//! RapidOCR (PP-OCR det + rec ONNX models) on ONNX Runtime, following RapidOcrOnnx's
//! pre- and post-processing so accuracy matches the C++ version.

use image::{Rgb, RgbImage};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

const PADDING: u32 = 10;
const MAX_SIDE: u32 = 960;
const BOX_SCORE: f32 = 0.5;
const BINARY: f32 = 0.5;
const UNCLIP: f32 = 1.5;
const MIN_SIDE: f32 = 3.0;
const REC_HEIGHT: u32 = 48;

/// Model files for a language tag. Unknown tags use English, the bool says so.
pub fn language_pack(tag: &str) -> (&'static str, [&'static str; 3], bool) {
    let tag = tag.trim().to_ascii_lowercase().replace('_', "-");
    match tag.as_str() {
        "zh" | "zh-cn" => ("zh-CN", ["ch_PP-OCRv5_det_mobile.onnx", "ch_PP-OCRv5_rec_mobile.onnx", "ppocrv5_dict.txt"], true),
        t => ("en", ["en_PP-OCRv3_det_mobile.onnx", "en_PP-OCRv5_rec_mobile.onnx", "ppocrv5_en_dict.txt"], t == "en"),
    }
}

pub struct Ocr {
    det: Session,
    rec: Session,
    keys: Vec<String>,
    pub language: &'static str,
}

impl Ocr {
    pub fn new(models: &Path, language: &str) -> Result<Ocr, String> {
        let (language, [det, rec, dict], _) = language_pack(language);
        static ENV: std::sync::OnceLock<Result<ort::environment::Environment, String>> = std::sync::OnceLock::new();
        let env = ENV.get_or_init(|| ort::environment::init().with_name("autosplitter").with_telemetry(false).build().map_err(|e| e.to_string())).as_ref()?;
        let session = |f: &str| -> Result<Session, String> {
            Session::builder(env).and_then(|mut b| b.commit_from_file(models.join(f))).map_err(|e| format!("{f}: {e}"))
        };
        let dict = std::fs::read_to_string(models.join(dict)).map_err(|e| format!("{dict}: {e}"))?;
        // Index 0 is the CTC blank, the last index a space.
        let keys = std::iter::once(String::new()).chain(dict.lines().map(String::from)).chain(std::iter::once(" ".into())).collect();
        Ok(Ocr { det: session(det)?, rec: session(rec)?, keys, language })
    }

    /// Reads all text in the image, one detected line per output line, top to bottom.
    pub fn read(&mut self, image: &RgbImage) -> Result<String, String> {
        let (w, h) = image.dimensions();
        if w == 0 || h == 0 {
            return Ok(String::new());
        }
        // Short words ("Hard") are too small for the recogniser below about 24 px.
        let scaled;
        let image = if w < 400 || h < 80 {
            let f = if w < 200 { 3 } else { 2 };
            scaled = resize(image, w * f, h * f, true);
            &scaled
        } else {
            image
        };
        let mut padded = RgbImage::from_pixel(image.width() + 2 * PADDING, image.height() + 2 * PADDING, Rgb([255, 255, 255]));
        image::imageops::replace(&mut padded, image, PADDING.into(), PADDING.into());
        let mut text = String::new();
        for quad in self.detect(&padded)? {
            let line = self.recognize(&crop(&padded, &quad))?;
            if !line.is_empty() {
                text.push_str(&line);
                text.push('\n');
            }
        }
        Ok(if self.language == "zh-CN" { crate::matcher::keep_cjk_only(&text) } else { text })
    }

    fn detect(&mut self, img: &RgbImage) -> Result<Vec<[(f32, f32); 4]>, String> {
        let (sw, sh) = img.dimensions();
        let target = (sw.max(sh) - 2 * PADDING).min(MAX_SIDE) + 2 * PADDING;
        let ratio = target as f32 / sw.max(sh) as f32;
        let fit = |v: u32| {
            let d = (v as f32 * ratio) as u32;
            if d.is_multiple_of(32) { d } else { ((d / 32).saturating_sub(1) * 32).max(32) }
        };
        let (dw, dh) = (fit(sw), fit(sh));
        let (rx, ry) = (dw as f32 / sw as f32, dh as f32 / sh as f32);
        let input = tensor(&resize(img, dw, dh, false), [0.485 * 255.0, 0.456 * 255.0, 0.406 * 255.0], [1.0 / 0.229 / 255.0, 1.0 / 0.224 / 255.0, 1.0 / 0.225 / 255.0])?;
        let out = self.det.run(ort::inputs![input]).map_err(|e| e.to_string())?;
        let (_, pred) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;

        // Binary map, dilated by a 2x2 square as RapidOcrOnnx does.
        let on = |x: u32, y: u32| pred[(y * dw + x) as usize] > BINARY;
        let bin = image::GrayImage::from_fn(dw, dh, |x, y| {
            let hit = on(x, y) || (x > 0 && on(x - 1, y)) || (y > 0 && on(x, y - 1)) || (x > 0 && y > 0 && on(x - 1, y - 1));
            image::Luma([if hit { 255 } else { 0 }])
        });
        let mut boxes = Vec::new();
        for contour in imageproc::contours::find_contours::<i32>(&bin) {
            if contour.points.len() <= 2 {
                continue;
            }
            let quad = order(imageproc::geometry::min_area_rect(&contour.points).map(|p| (p.x as f32, p.y as f32)));
            let (qw, qh) = (dist(quad[0], quad[1]), dist(quad[1], quad[2]));
            if qw.min(qh) < MIN_SIDE || score(&quad, pred, dw, dh) < BOX_SCORE {
                continue;
            }
            // Clipper's round offset of a rectangle by area * ratio / perimeter, as its bounding rectangle.
            let d = qw * qh * UNCLIP / (2.0 * (qw + qh));
            let (cx, cy) = (quad.iter().map(|p| p.0).sum::<f32>() / 4.0, quad.iter().map(|p| p.1).sum::<f32>() / 4.0);
            let grow = |p: (f32, f32)| {
                let (vx, vy) = (p.0 - cx, p.1 - cy);
                let (ux, uy) = unit(quad[1].0 - quad[0].0, quad[1].1 - quad[0].1);
                let (along, across) = (vx * ux + vy * uy, -vx * uy + vy * ux);
                let along = along + d * along.signum();
                let across = across + d * across.signum();
                (cx + along * ux - across * uy, cy + along * uy + across * ux)
            };
            if qw.min(qh) + 2.0 * d < MIN_SIDE + 2.0 {
                continue;
            }
            boxes.push(quad.map(grow).map(|(x, y)| ((x / rx).clamp(0.0, sw as f32), (y / ry).clamp(0.0, sh as f32))));
        }
        // Reading order: top to bottom, left to right within a line.
        boxes.sort_by(|a, b| a[0].1.total_cmp(&b[0].1).then(a[0].0.total_cmp(&b[0].0)));
        for i in 1..boxes.len() {
            let mut j = i;
            while j > 0 && (boxes[j][0].1 - boxes[j - 1][0].1).abs() < 10.0 && boxes[j][0].0 < boxes[j - 1][0].0 {
                boxes.swap(j, j - 1);
                j -= 1;
            }
        }
        Ok(boxes)
    }

    fn recognize(&mut self, line: &RgbImage) -> Result<String, String> {
        let w = ((line.width() as f32 * REC_HEIGHT as f32 / line.height() as f32) as u32).max(1);
        let input = tensor(&resize(line, w, REC_HEIGHT, false), [127.5; 3], [1.0 / 127.5; 3])?;
        let out = self.rec.run(ort::inputs![input]).map_err(|e| e.to_string())?;
        let (shape, probs) = out[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let classes = shape[2] as usize;
        let mut text = String::new();
        let mut last = 0;
        for step in probs.chunks(classes) {
            let best = step.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i);
            if best > 0 && best < self.keys.len() && best != last {
                text.push_str(&self.keys[best]);
            }
            last = best;
        }
        Ok(text)
    }
}

/// NCHW float tensor in BGR channel order, the order the PaddleOCR models were trained on.
fn tensor(img: &RgbImage, mean: [f32; 3], norm: [f32; 3]) -> Result<Tensor<f32>, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut data = vec![0.0; 3 * w * h];
    for (i, p) in img.pixels().enumerate() {
        for c in 0..3 {
            data[c * w * h + i] = (p[2 - c] as f32 - mean[c]) * norm[c];
        }
    }
    Tensor::from_array(([1usize, 3, h, w], data)).map_err(|e| e.to_string())
}

/// Resize with OpenCV's pixel-centre mapping and no anti-aliasing (INTER_LINEAR or INTER_CUBIC).
fn resize(img: &RgbImage, w: u32, h: u32, cubic: bool) -> RgbImage {
    let taps = |src: u32, dst: u32| -> Vec<(i64, [f32; 4])> {
        let scale = src as f32 / dst as f32;
        (0..dst)
            .map(|d| {
                let f = (d as f32 + 0.5) * scale - 0.5;
                let s = f.floor();
                let t = f - s;
                if cubic {
                    let a = -0.75;
                    let w0 = ((a * (t + 1.0) - 5.0 * a) * (t + 1.0) + 8.0 * a) * (t + 1.0) - 4.0 * a;
                    let w1 = ((a + 2.0) * t - (a + 3.0)) * t * t + 1.0;
                    let w2 = ((a + 2.0) * (1.0 - t) - (a + 3.0)) * (1.0 - t) * (1.0 - t) + 1.0;
                    (s as i64 - 1, [w0, w1, w2, 1.0 - w0 - w1 - w2])
                } else {
                    (s as i64, [1.0 - t, t, 0.0, 0.0])
                }
            })
            .collect()
    };
    let (sw, sh) = img.dimensions();
    let (xs, ys) = (taps(sw, w), taps(sh, h));
    let at = |v: i64, max: u32| v.clamp(0, max as i64 - 1) as u32;
    // Horizontal pass to floats, then vertical.
    let mut mid = vec![[0f32; 3]; (w * sh) as usize];
    for y in 0..sh {
        for (x, (s, k)) in xs.iter().enumerate() {
            let mut acc = [0f32; 3];
            for (i, k) in k.iter().enumerate().filter(|(_, k)| **k != 0.0) {
                let p = img.get_pixel(at(s + i as i64, sw), y);
                for c in 0..3 {
                    acc[c] += p[c] as f32 * k;
                }
            }
            mid[(y * w) as usize + x] = acc;
        }
    }
    RgbImage::from_fn(w, h, |x, y| {
        let (s, k) = ys[y as usize];
        let mut acc = [0f32; 3];
        for (i, k) in k.iter().enumerate().filter(|(_, k)| **k != 0.0) {
            let p = mid[(at(s + i as i64, sh) * w + x) as usize];
            for c in 0..3 {
                acc[c] += p[c] * k;
            }
        }
        Rgb(acc.map(|v| v.round().clamp(0.0, 255.0) as u8))
    })
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn unit(x: f32, y: f32) -> (f32, f32) {
    let l = x.hypot(y).max(f32::EPSILON);
    (x / l, y / l)
}

/// Corners as top-left, top-right, bottom-right, bottom-left (RapidOcrOnnx's getMinBoxes).
fn order(mut p: [(f32, f32); 4]) -> [(f32, f32); 4] {
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (tl, bl) = if p[1].1 > p[0].1 { (p[0], p[1]) } else { (p[1], p[0]) };
    let (tr, br) = if p[3].1 > p[2].1 { (p[2], p[3]) } else { (p[3], p[2]) };
    [tl, tr, br, bl]
}

/// Mean probability inside the box.
fn score(q: &[(f32, f32); 4], pred: &[f32], w: u32, h: u32) -> f32 {
    let lo = |f: fn(&(f32, f32)) -> f32| q.iter().map(f).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
    let hi = |f: fn(&(f32, f32)) -> f32, m: u32| (q.iter().map(f).fold(f32::MIN, f32::max).ceil() as u32).min(m - 1);
    let (x0, x1, y0, y1) = (lo(|p| p.0), hi(|p| p.0, w), lo(|p| p.1), hi(|p| p.1, h));
    let inside = |x: f32, y: f32| (0..4).all(|i| {
        let (a, b) = (q[i], q[(i + 1) % 4]);
        (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0) >= -0.5
    });
    let (mut sum, mut n) = (0.0, 0);
    for y in y0..=y1 {
        for x in x0..=x1 {
            if inside(x as f32, y as f32) {
                sum += pred[(y * w + x) as usize];
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { sum / n as f32 }
}

/// The box's upright bounding rectangle: HUD text is horizontal, and straightening a
/// tilted box turns icons (a diamond) into letters (a square).
fn crop(img: &RgbImage, q: &[(f32, f32); 4]) -> RgbImage {
    let x0 = q.iter().map(|p| p.0).fold(f32::MAX, f32::min) as u32;
    let y0 = q.iter().map(|p| p.1).fold(f32::MAX, f32::min) as u32;
    let x1 = (q.iter().map(|p| p.0).fold(0.0, f32::max) as u32).min(img.width());
    let y1 = (q.iter().map(|p| p.1).fold(0.0, f32::max) as u32).min(img.height());
    image::imageops::crop_imm(img, x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_and_order() {
        let img = RgbImage::from_pixel(4, 2, Rgb([10, 20, 30]));
        assert!(resize(&img, 9, 5, true).pixels().all(|p| *p == Rgb([10, 20, 30])));
        assert_eq!(order([(5.0, 5.0), (0.0, 0.0), (5.0, 0.0), (0.0, 5.0)]), [(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (0.0, 5.0)]);
        assert_eq!(language_pack("ZH_cn").0, "zh-CN");
        assert!(!language_pack("fr").2);
    }
}
