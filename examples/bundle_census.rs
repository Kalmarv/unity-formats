//! Reads UnityFS bundles end to end and prints what it finds, one JSON object per line: each bundle's header and block
//! counts, each node's bytes (a SHA-256), and for each serialized node its header, types (with a digest of each stored
//! type tree), object table digest and externals, every object decoded by its stored type tree, every streamed resource
//! checked against its stream node. Used to compare this reader with another one's on the same bundles
//! (tests/bundles.rs does, with tests/oracles/unitypy_bundles.py).
//!
//! `cargo run --release --example bundle_census -- <type tree package (.tpk)> <bundle>...`

#[path = "../tests/census/mod.rs"]
mod census;

use std::path::Path;

use unity_formats::TypeTreePackage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(package) = args.next() else {
        eprintln!("usage: bundle_census <type tree package (.tpk)> <bundle>...");
        return Ok(());
    };
    let package = TypeTreePackage::parse(&std::fs::read(package)?)?;
    for arg in args {
        for line in census::bundle(Path::new(&arg), &package, true) {
            println!("{line}");
        }
    }
    Ok(())
}
