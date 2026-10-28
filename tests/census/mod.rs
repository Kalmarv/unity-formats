//! A census of UnityFS bundles as JSON lines, shared by tests/bundles.rs (which compares it with an independent reader's)
//! and the `bundle_census` example (which prints it): each bundle's header and block counts, each node's bytes (a
//! SHA-256), and for each serialized node its header, types (with a digest of each stored type tree), object table
//! digest and externals, optionally every object decoded by its stored type tree and every streamed resource checked
//! against its stream node.
#![allow(dead_code, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use unity_formats::TypeTreePackage;
use unity_formats::bundle::Bundle;
use unity_formats::resource::{StreamingInfo, streaming_infos};
use unity_formats::serialized::{MONO_BEHAVIOUR, SerializedFile};
use unity_formats::typetree::TypeTree;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `level|type|name|byte size|version|meta flags` of each node in depth-first order.
pub fn tree_lines(tree: &TypeTree) -> Vec<String> {
    fn walk(tree: &TypeTree, at: usize, level: usize, out: &mut Vec<String>) {
        let Some(n) = tree.nodes.get(at) else { return };
        out.push(format!(
            "{level}|{}|{}|{}|{}|{}",
            n.ty, n.name, n.byte_size, n.version, n.meta_flag
        ));
        for &c in &n.children {
            walk(tree, c, level + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(tree, 0, 0, &mut out);
    out
}

fn digest(lines: &[String]) -> String {
    let mut h = Sha256::new();
    for l in lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    hex(&h.finalize())
}

/// What decoding every object of a serialized node by its stored tree found.
pub fn decoded(file: &SerializedFile, bundle: &Arc<Bundle>, package: &TypeTreePackage) -> Json {
    let mut objects: Vec<_> = file.objects.iter().collect();
    // in file order, so the bundle's recent blocks serve the next object
    objects.sort_by_key(|o| o.offset);
    let (mut decoded, mut covered, mut no_tree) = (0u64, 0u64, 0u64);
    let mut refusals: BTreeMap<String, u64> = BTreeMap::new();
    let (mut streams, mut stream_bytes) = (0u64, 0u64);
    let (mut same_as_package, mut differs, mut absent) = (0u64, Vec::new(), 0u64);
    let mut checked_types = BTreeSet::new();
    for o in &objects {
        let Some(tree) = file.tree_of(o) else {
            no_tree += 1;
            continue;
        };
        if checked_types.insert(o.type_index) && o.class_id != MONO_BEHAVIOUR {
            match package.type_tree(o.class_id, file.unity) {
                Ok(p) if tree_lines(&p) == tree_lines(tree) => same_as_package += 1,
                Ok(_) => differs.push(o.class_id),
                Err(_) => absent += 1,
            }
        }
        match file.decode_with(o, tree) {
            Ok(d) => {
                decoded += 1;
                if d.rest == 0 {
                    covered += 1;
                } else {
                    *refusals
                        .entry(format!("bytes left after the tree (class {})", o.class_id))
                        .or_default() += 1;
                }
                for (_, info) in streaming_infos(&d.value) {
                    if let Ok(Some(s)) = StreamingInfo::of(info) {
                        match s.node_in(bundle) {
                            Ok(_) => {
                                streams += 1;
                                stream_bytes += s.size;
                            }
                            Err(e) => {
                                *refusals.entry(format!("stream: {e}")).or_default() += 1;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                // the part of the message that does not name the object
                let reason = msg.split_once("): ").map_or(msg.as_str(), |(_, r)| r).to_owned();
                *refusals
                    .entry(format!("class {}: {reason}", o.class_id))
                    .or_default() += 1;
            }
        }
    }
    differs.sort_unstable();
    differs.dedup();
    json!({
        "decoded": decoded,
        "covered": covered,
        "no_tree": no_tree,
        "refusals": refusals,
        "streams_resolved": streams,
        "stream_bytes": stream_bytes,
        "trees_same_as_package": same_as_package,
        "trees_differ_from_package": differs,
        "trees_not_in_package": absent,
    })
}

/// A serialized node of `bundle` as JSON; with `decode`, every object decoded as well.
pub fn serialized(bundle: &Arc<Bundle>, path: &str, package: &TypeTreePackage, decode: bool) -> Json {
    let file = match SerializedFile::from_bundle(bundle, path, Some(package)) {
        Ok(f) => f,
        Err(e) => return json!({"error": e.to_string()}),
    };
    let types: Vec<Json> = file
        .types
        .iter()
        .map(|t| {
            let tree = t.tree.as_ref().map(|tree| {
                let lines = tree_lines(tree);
                json!({"nodes": lines.len(), "digest": digest(&lines)})
            });
            json!({
                "class_id": t.class_id,
                "stripped": t.stripped,
                "script_type_index": t.script_type_index,
                "script_id": t.script_id.map(|h| hex(&h)),
                "old_type_hash": hex(&t.old_type_hash),
                "tree": tree,
                "dependencies": t.dependencies,
            })
        })
        .collect();
    let mut objects: Vec<_> = file.objects.iter().collect();
    objects.sort_by_key(|o| o.path_id);
    let lines: Vec<String> = objects
        .iter()
        .map(|o| {
            format!(
                "{}|{}|{}|{}|{}",
                o.path_id, o.class_id, o.offset, o.size, o.type_index
            )
        })
        .collect();
    let mut out = json!({
        "version": file.version,
        "unity_version": file.unity_version,
        "target_platform": file.target_platform,
        "types": types,
        "objects": {"count": file.objects.len(), "digest": digest(&lines)},
        "script_types": file.script_types,
        "externals": file.externals.iter().map(|e| &e.path).collect::<Vec<_>>(),
        "ref_types": file.ref_types.len(),
        "user_information": file.user_information,
    });
    if decode {
        out["decode"] = decoded(&file, bundle, package);
    }
    out
}

/// The census lines of the bundle at `path`: its header line, then one per node. A bundle that is refused is one line
/// saying why.
pub fn bundle(path: &Path, package: &TypeTreePackage, decode: bool) -> Vec<Json> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bundle = match Bundle::open(path) {
        Ok(b) => Arc::new(b),
        Err(e) => return vec![json!({"bundle": name, "refused": e.to_string()})],
    };
    let h = bundle.header();
    let mut by_compression: BTreeMap<u16, u64> = BTreeMap::new();
    for b in bundle.blocks() {
        *by_compression.entry(b.flags & 0x3f).or_insert(0) += 1;
    }
    let mut lines = vec![json!({
        "bundle": name,
        "header": {
            "format_version": h.format_version,
            "player_version": h.player_version,
            "engine_version": h.engine_version,
            "size": h.size,
            "flags": h.flags,
        },
        "blocks": by_compression,
        "data_len": bundle.data_len(),
        "block_bytes": bundle.data_len(),
        "nodes": bundle.nodes().len(),
    })];
    for node in bundle.nodes() {
        let mut sha = Sha256::new();
        // hash the node in pieces so a gigabyte node is not held whole
        let (mut at, piece) = (0u64, 16u64 << 20);
        while at < node.size {
            let n = piece.min(node.size - at);
            sha.update(bundle.read(node, at, usize::try_from(n).unwrap()).unwrap());
            at += n;
        }
        let mut line = json!({
            "bundle": name,
            "node": node.path,
            "offset": node.offset,
            "size": node.size,
            "flags": node.flags,
            "sha256": hex(&sha.finalize()),
        });
        if node.is_serialized() {
            line["serialized"] = serialized(&bundle, &node.path, package, decode);
        }
        lines.push(line);
    }
    lines
}
