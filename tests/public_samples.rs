//! Public files of other Unity readers, read here as their own tools read them.
//!
//! `unitypy_samples_read_as_unitypy_reads_them`: UnityPy's test bundles (`oracles.corpus.general/unitypy-samples`:
//! UnityFS format 6, LZ4HC, from Unity 5.6.7, 2017.4.37 and 2018.4) against UnityPy's reading of them
//! (`oracles.unitypy.general/samples.json`, tests/oracles/unitypy_samples.py): every bundle's format and header flags,
//! and every node's path, size and SHA-256, equal.
//!
//! `assetripper_packages_read_the_same_in_every_compression`: AssetRipper's type tree package as its CI publishes it
//! uncompressed, LZ4, LZMA and Brotli (`oracles.corpus.general/assetripper-tpk`): every class's name and type tree, for
//! the class ids up to 2000 at a Unity version of each generation, equal across the four.

#![allow(clippy::unwrap_used)]

mod inputs;

use inputs::{Game, OracleSet, oracle};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use unity_formats::TypeTreePackage;
use unity_formats::bundle::Bundle;
use unity_formats::version::UnityVersion;

#[test]
fn unitypy_samples_read_as_unitypy_reads_them() {
    let (Some(dir), Some(expected)) = (
        oracle(OracleSet::Corpus, Game::General, "unitypy-samples"),
        oracle(OracleSet::UnityPy, Game::General, "samples.json"),
    ) else {
        return;
    };
    let expected: Json = serde_json::from_slice(&std::fs::read(expected).unwrap()).unwrap();
    let expected = expected.as_object().unwrap();
    assert_eq!(expected.len(), 4);
    for (name, want) in expected {
        let bundle = Bundle::open(&dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let header = bundle.header();
        assert_eq!(
            (u64::from(header.format_version), u64::from(header.flags)),
            (want["format"].as_u64().unwrap(), want["flags"].as_u64().unwrap()),
            "{name}"
        );
        let got: Vec<(String, u64, String)> = bundle
            .nodes()
            .iter()
            .map(|node| {
                let bytes = bundle.read_node(node).unwrap();
                let sha: String = Sha256::digest(&bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                (node.path.clone(), bytes.len() as u64, sha)
            })
            .collect();
        let want: Vec<(String, u64, String)> = want["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["path"].as_str().unwrap().to_owned(),
                    n["size"].as_u64().unwrap(),
                    n["sha256"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(got, want, "{name}");
    }
}

#[test]
fn assetripper_packages_read_the_same_in_every_compression() {
    let Some(dir) = oracle(OracleSet::Corpus, Game::General, "assetripper-tpk") else {
        return;
    };
    let versions = ["5.6.7f1", "2019.4.20f1", "6000.3.15f1"].map(|v| UnityVersion::parse(v).unwrap());
    let reading = |file: &str| {
        let bytes = std::fs::read(dir.join(file)).unwrap();
        let package = TypeTreePackage::parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut classes = Vec::new();
        for version in versions {
            for id in 0..=2000 {
                let Some(name) = package.class_name(id, version) else {
                    continue;
                };
                let tree = package.type_tree(id, version).map_err(|e| e.to_string());
                classes.push((version, id, name.to_owned(), tree));
            }
        }
        classes
    };
    let uncompressed = reading("uncompressed.tpk");
    assert!(uncompressed.len() > 300, "{} classes", uncompressed.len());
    for file in ["lz4.tpk", "lzma.tpk", "brotli.tpk"] {
        assert!(reading(file) == uncompressed, "{file}");
    }
}
