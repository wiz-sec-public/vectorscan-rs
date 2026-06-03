//! Minimal example that links `vectorscan-rs` (and thus the native vectorscan
//! static library) into a normal Rust binary. On the `*-windows-msvc` target
//! this exercises the static MSVC-ABI link path end to end.

use vectorscan_rs::{BlockDatabase, BlockScanner, Error, Flag, Pattern, Scan};

fn main() -> Result<(), Error> {
    let patterns = vec![
        Pattern::new(b"foo.*bar".to_vec(), Flag::default(), Some(0)),
        Pattern::new(b"hello".to_vec(), Flag::default(), Some(1)),
    ];
    let db = BlockDatabase::new(patterns)?;
    let mut scanner = BlockScanner::new(&db)?;

    let data: &[u8] = b"xx foo123bar yy hello world";
    println!("scanning: {:?}", String::from_utf8_lossy(data));

    let mut matches: Vec<(u32, u64, u64)> = Vec::new();
    scanner.scan(data, |id, from, to, _flags| {
        matches.push((id, from, to));
        Scan::Continue
    })?;

    for (id, from, to) in &matches {
        println!("  match id={id} at [{from}..{to}]");
    }
    println!(
        "done: {} match(es); vectorscan statically linked into this MSVC binary",
        matches.len()
    );
    Ok(())
}
