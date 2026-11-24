//! Read the text in an image file, as AutoSplitter's OCR does:
//! `cargo run --release --example ocr_file -- image.png [en|zh-CN]`.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let image = image::open(&args[0]).expect("cannot open the image").to_rgb8();
    let mut ocr = autosplitter::ocr::Ocr::new(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("models"), args.get(1).map_or("en", String::as_str)).expect("models missing");
    let started = std::time::Instant::now();
    let text = ocr.read(&image).expect("OCR failed");
    println!("{} x {}, {} ms:\n{text}", image.width(), image.height(), started.elapsed().as_millis());
}
