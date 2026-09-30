//! Imports one file and writes the layer it makes as a PNG, with what the import noted, whether
//! the pixels are approximate or not. For measuring the gaps the parity run reports as pending.
//!
//!     cargo run --release -p image-import --example import -- <input> <output.png> [exposure temperature tint boost]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: import <input> <output.png> [exposure temperature tint boost]");
        std::process::exit(2);
    }
    let value = |i: usize| args.get(i).and_then(|v| v.parse::<f32>().ok());
    let raw = (args.len() > 3).then(|| image_import::RawSettings { exposure: value(3), temperature: value(4), tint: value(5), boost: value(6) });
    match image_import::import_file(std::path::Path::new(&args[1]), raw.as_ref()) {
        Ok(imported) => {
            let layer = imported.project.images.values().next().expect("one layer");
            layer.pixels.save(&args[2]).expect("writing the PNG");
            for note in &imported.notes {
                println!("note: {note}");
            }
            if let Some(why) = &imported.approximation {
                println!("approximate: {why}");
            }
        }
        Err(e) => {
            println!("refused: {e}");
            std::process::exit(1);
        }
    }
}
