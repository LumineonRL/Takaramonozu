//! `cargo run -p preprocess` - converts the .json data into .rkyv

use std::path::{Path, PathBuf};

use preprocess::Failed;
use simulator::archive;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
}

#[expect(clippy::print_stdout, reason = "the output of a CLI")]
fn report(path: &Path, what: &str, n: usize) -> Result<(), Failed> {
    let bytes = std::fs::metadata(path)?.len();
    // TIL canonicalize was a thing.
    println!("{n} {what}, {bytes} bytes -> {}", path.canonicalize()?.display());
    Ok(())
}

fn main() -> Result<(), Failed> {
    let root = root();
    let data = root.join("data");
    let out = root.join("archive");
    std::fs::create_dir_all(&out)?;

    let cards = preprocess::cards(&data)?;
    let path = out.join("cards.rkyv");
    archive::write(&path, &cards)?;
    report(&path, "cards", cards.len())?;

    let accessories = preprocess::accessories(&data, &cards.roster)?;
    let acc_path = out.join("accessories.rkyv");
    archive::write(&acc_path, &accessories)?;
    report(&acc_path, "accessories", accessories.len())?;

    let sis = preprocess::sis(&data, &cards.roster)?;
    let sis_path = out.join("sis.rkyv");
    archive::write(&sis_path, &sis)?;
    report(&sis_path, "SIS", sis.len())
}
