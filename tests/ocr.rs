//! OCR accuracy on the legacy screenshots, scored like the legacy benchmark: the file name
//! (`_` for a space, `-N` for a variant) must be in the OCR text, exactly or by the app's matcher.
//! Needs the models: run packaging/fetch-models.sh first.

use autosplitter::matcher::fuzzy_contains;
use autosplitter::ocr::Ocr;
use std::path::Path;

fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn accuracy(language: &str, dir: &str) -> f32 {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut ocr = Ocr::new(&root.join("models"), language).expect("models missing: run packaging/fetch-models.sh");
    let mut files: Vec<_> = std::fs::read_dir(root.join("tests/images").join(dir)).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let mut passed = 0;
    for f in &files {
        let stem = f.file_stem().unwrap().to_str().unwrap();
        let stem = stem.rsplit_once('-').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).map_or(stem, |(s, _)| s);
        let expected = normalize(&stem.replace('_', " "));
        let started = std::time::Instant::now();
        let text = normalize(&ocr.read(&image::open(f).unwrap().to_rgb8()).unwrap());
        let ok = text.contains(&expected) || fuzzy_contains(&text, &expected, 0.92);
        passed += usize::from(ok);
        println!("{} {:>4} ms  {expected:?} -> {text:?}", if ok { "PASS" } else { "FAIL" }, started.elapsed().as_millis());
    }
    let pct = passed as f32 * 100.0 / files.len() as f32;
    println!("{language}: {passed}/{} = {pct:.1}%", files.len());
    pct
}

#[test]
fn english_at_least_legacy() {
    assert!(accuracy("en", "en") >= 88.0);
}

#[test]
fn chinese_at_least_legacy() {
    assert!(accuracy("zh-CN", "zh-CN") >= 100.0);
}
