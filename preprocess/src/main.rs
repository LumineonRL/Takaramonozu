//! `cargo run -p preprocess` -- build the archive from `data/`.

use std::path::PathBuf;

use preprocess::Failed;
use simulator::archive;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

#[expect(clippy::print_stdout, reason = "the output of a CLI")]
fn main() -> Result<(), Failed> {
    let root = root();
    let out = root.join("archive");
    std::fs::create_dir_all(&out)?;

    let db = preprocess::cards(&root.join("data"))?;
    let path = out.join("cards.rkyv");
    archive::write(&path, &db)?;

    let bytes = std::fs::metadata(&path)?.len();
    // TIL canonicalize was a thing.
    println!("{} cards, {bytes} bytes -> {}", db.len(), path.canonicalize()?.display());
    Ok(())
}
