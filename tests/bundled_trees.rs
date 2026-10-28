//! Embedded type trees and bundles on bytes made here: a SerializedFile that stores its type trees (common strings from a
//! type tree package, strings of its own), the same file in a UnityFS bundle read by range, and a player that reads it
//! from the bundle.
#![allow(clippy::unwrap_used)]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the player test writes its bundle to a temporary folder"
)]

use std::sync::Arc;

use unity_formats::bundle::Bundle;
use unity_formats::player::{BundleSpec, NodeSelection, ObjRef, Player};
use unity_formats::resource::StreamingInfo;
use unity_formats::{Error, SerializedFile, TypeTreePackage, UnityVersion};

/// The common strings of the package: `int` at 0, `m_Count` at 4, `string` at 12.
const COMMON: [(u16, &str); 3] = [(0, "int"), (4, "m_Count"), (12, "string")];

/// A format 2 type tree package, uncompressed, with no classes and one common string table from version 0.
fn package_bytes() -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend(0i64.to_le_bytes());
    blob.extend(0i32.to_le_bytes()); // versions
    blob.extend(0i32.to_le_bytes()); // classes
    blob.extend(1i32.to_le_bytes()); // common string versions
    blob.extend(0u64.to_le_bytes());
    blob.extend((COMMON.len() as i32).to_le_bytes());
    for (i, (offset, _)) in COMMON.iter().enumerate() {
        blob.extend(offset.to_le_bytes());
        blob.extend((i as u16).to_le_bytes());
    }
    blob.extend(0i32.to_le_bytes()); // nodes
    blob.extend((COMMON.len() as i32).to_le_bytes());
    for (_, s) in COMMON {
        blob.push(s.len() as u8);
        blob.extend(s.as_bytes());
    }
    let mut out = 0x2A4B_5054u32.to_le_bytes().to_vec();
    out.extend([2, 0, 0, 0]);
    out.extend(0u32.to_le_bytes());
    out.extend((blob.len() as u32).to_le_bytes());
    out.extend((blob.len() as u32).to_le_bytes());
    out.extend(blob);
    out
}

/// One node of an embedded tree.
fn node(level: u8, ty: u32, name: u32, byte_size: i32, meta: u32) -> Vec<u8> {
    let mut n = Vec::new();
    n.extend(1i16.to_le_bytes());
    n.push(level);
    n.push(0);
    n.extend(ty.to_le_bytes());
    n.extend(name.to_le_bytes());
    n.extend(byte_size.to_le_bytes());
    n.extend(0i32.to_le_bytes());
    n.extend(meta.to_le_bytes());
    n.extend(0u64.to_le_bytes());
    n
}

const COMMON_BIT: u32 = 0x8000_0000;

/// A version 22 SerializedFile of Unity 6000.3.15f1 with one type (class 1) that stores its tree
/// `Base { int m_Count; string m_Label }` (`int`, `m_Count` and `string` from the common strings, `Base` and `m_Label`
/// from the tree's own), and one object of that type: 7, "hi". `common_offset` replaces `m_Count`'s offset.
fn serialized(common_offset: u32) -> Vec<u8> {
    let buffer = b"GameObject\0Base\0m_Label\0";
    let mut meta = b"6000.3.15f1\0".to_vec();
    meta.extend(19i32.to_le_bytes());
    meta.push(1); // stores type trees
    meta.extend(1i32.to_le_bytes()); // one type
    meta.extend(1i32.to_le_bytes()); // class 1
    meta.push(0);
    meta.extend((-1i16).to_le_bytes());
    meta.extend([9; 16]);
    meta.extend(3i32.to_le_bytes()); // nodes
    meta.extend((buffer.len() as i32).to_le_bytes());
    meta.extend(node(0, 0, 11, -1, 0));
    meta.extend(node(1, COMMON_BIT, COMMON_BIT | common_offset, 4, 0));
    meta.extend(node(1, COMMON_BIT | 12, 16, -1, 0x4000));
    meta.extend(buffer);
    meta.extend(0i32.to_le_bytes()); // type dependencies
    meta.extend(1i32.to_le_bytes()); // one object
    while !(48 + meta.len()).is_multiple_of(4) {
        meta.push(0);
    }
    meta.extend(1i64.to_le_bytes());
    meta.extend(0i64.to_le_bytes());
    meta.extend(12u32.to_le_bytes());
    meta.extend(0u32.to_le_bytes());
    meta.extend([0; 12]); // no scripts, externals, reference types
    meta.push(0); // user string
    let data_offset = (48 + meta.len()).next_multiple_of(16);
    let size = data_offset + 12;
    let mut out = vec![0; 48];
    out[8..12].copy_from_slice(&22u32.to_be_bytes());
    out[20..24].copy_from_slice(&(meta.len() as u32).to_be_bytes());
    out[24..32].copy_from_slice(&(size as u64).to_be_bytes());
    out[32..40].copy_from_slice(&(data_offset as u64).to_be_bytes());
    out.extend(meta);
    out.resize(data_offset, 0);
    out.extend(7i32.to_le_bytes());
    out.extend(2i32.to_le_bytes());
    out.extend(b"hi\0\0");
    out
}

/// A UnityFS bundle (format 8, flags 0x243) of `nodes`, their bytes in blocks of 24 bytes, alternately stored and LZ4
/// (one literal run).
fn bundle(nodes: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (path, flags, bytes) in nodes {
        entries.push((data.len() as i64, bytes.len() as i64, *flags, *path));
        data.extend(bytes);
    }
    let mut table = vec![0u8; 16];
    let chunks: Vec<&[u8]> = data.chunks(24).collect();
    table.extend((chunks.len() as i32).to_be_bytes());
    let mut stored = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        let packed = if i % 2 == 0 || c.len() < 15 {
            c.to_vec()
        } else {
            let mut p = vec![0xf0, (c.len() - 15) as u8];
            p.extend(*c);
            p
        };
        let compression: u16 = if packed.len() == c.len() { 0 } else { 3 };
        table.extend((c.len() as u32).to_be_bytes());
        table.extend((packed.len() as u32).to_be_bytes());
        table.extend(compression.to_be_bytes());
        stored.extend(packed);
    }
    table.extend((entries.len() as i32).to_be_bytes());
    for (offset, size, flags, path) in &entries {
        table.extend(offset.to_be_bytes());
        table.extend(size.to_be_bytes());
        table.extend(flags.to_be_bytes());
        table.extend(path.as_bytes());
        table.push(0);
    }
    let mut packed = vec![0xf0];
    let mut rest = table.len() - 15;
    while rest >= 255 {
        packed.push(255);
        rest -= 255;
    }
    packed.push(rest as u8);
    packed.extend(&table);
    let mut out = b"UnityFS\0".to_vec();
    out.extend(8u32.to_be_bytes());
    out.extend(b"5.x.x\0");
    out.extend(b"6000.3.15f1\0");
    let size_at = out.len();
    out.extend([0; 8]);
    out.extend((packed.len() as u32).to_be_bytes());
    out.extend((table.len() as u32).to_be_bytes());
    out.extend(0x243u32.to_be_bytes());
    out.resize(out.len().next_multiple_of(16), 0);
    out.extend(packed);
    out.resize(out.len().next_multiple_of(16), 0);
    out.extend(stored);
    let total = out.len() as u64;
    out[size_at..size_at + 8].copy_from_slice(&total.to_be_bytes());
    out
}

#[test]
fn a_stored_tree_resolves_its_common_strings_from_the_package() {
    let package = TypeTreePackage::parse(&package_bytes()).unwrap();
    let common = package
        .common_strings(UnityVersion::parse("6000.3.15f1").unwrap())
        .unwrap();
    assert_eq!(common.len(), 3);
    assert_eq!(common.get(4), Some("m_Count"));
    assert_eq!(common.get(5), None);
    assert_eq!(
        common.iter().map(|(o, s)| (o, s.to_owned())).collect::<Vec<_>>()[2],
        (12, "string".to_owned())
    );

    let file = SerializedFile::parse_with(serialized(4), Some(&package)).unwrap();
    let tree = file.types[0].tree.as_ref().unwrap();
    let shape: Vec<(String, String)> = tree
        .nodes
        .iter()
        .map(|n| (n.ty.to_string(), n.name.to_string()))
        .collect();
    assert_eq!(
        shape,
        [("GameObject", "Base"), ("int", "m_Count"), ("string", "m_Label")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    assert_eq!(tree.nodes[0].children, [1, 2]);
    assert_eq!(file.types[0].dependencies, Vec::<i32>::new());
    let o = file.objects[0];
    let d = file.decode(&o, &package).unwrap();
    assert_eq!(d.rest, 0);
    assert_eq!(d.value.get("m_Count").and_then(|v| v.as_i64()), Some(7));
    assert_eq!(d.value.get("m_Label").and_then(|v| v.as_str()), Some("hi"));
}

#[test]
fn a_common_string_without_its_table_is_refused_with_the_reason() {
    // no package at all: the file is not damaged, the reader lacks what it needs
    let e = SerializedFile::parse(serialized(4)).unwrap_err();
    assert!(matches!(e, Error::Unsupported(_)), "{e}");
    // the first common string the tree names is its child's type, `int` at offset 0
    assert!(
        e.to_string().contains("common string 0") && e.to_string().contains("6000.3.15f1"),
        "{e}"
    );
    // a package whose table lacks the offset: the file names a string that does not exist
    let package = TypeTreePackage::parse(&package_bytes()).unwrap();
    let e = SerializedFile::parse_with(serialized(5), Some(&package)).unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e}");
    assert!(e.to_string().contains("common string 5"), "{e}");
}

#[test]
fn a_bundled_file_reads_its_objects_by_range() {
    let package = TypeTreePackage::parse(&package_bytes()).unwrap();
    let whole = SerializedFile::parse_with(serialized(4), Some(&package)).unwrap();
    let bytes = bundle(&[
        ("CAB-1", 4, serialized(4)),
        ("CAB-1.resS", 0, (0..=99u8).collect()),
        ("Resources/other", 4, serialized(4)),
    ]);
    let dir = std::env::temp_dir().join(format!("unity-formats-bundled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.bundle"), &bytes).unwrap();
    let bundle = Arc::new(Bundle::open(&dir.join("test.bundle")).unwrap());
    assert_eq!(bundle.nodes().len(), 3);
    let from_bundle = SerializedFile::from_bundle(&bundle, "CAB-1", Some(&package)).unwrap();
    assert_eq!(from_bundle.objects, whole.objects);
    assert_eq!(from_bundle.types, whole.types);
    let o = from_bundle.objects[0];
    assert_eq!(from_bundle.bytes(&o).unwrap(), whole.bytes(&o).unwrap());
    assert!(from_bundle.bundle().is_some() && whole.bundle().is_none());
    assert!(SerializedFile::from_bundle(&bundle, "CAB-1.resS", Some(&package)).is_err());
    assert!(SerializedFile::from_bundle(&bundle, "nothing", Some(&package)).is_err());

    // the stream node is found by the last segment of the path, whatever its case, and the data must lie inside it
    let stream = |path: &str, offset, size| StreamingInfo {
        offset,
        size,
        path: path.to_owned(),
    };
    assert_eq!(
        stream("archive:/CAB-1/cab-1.ress", 10, 5)
            .read_bundled(&bundle)
            .unwrap(),
        (10..15u8).collect::<Vec<_>>()
    );
    assert!(
        stream("archive:/CAB-1/CAB-1.resS", 98, 3)
            .read_bundled(&bundle)
            .is_err()
    );
    assert!(
        stream("archive:/CAB-1/missing.resS", 0, 1)
            .read_bundled(&bundle)
            .is_err()
    );

    // a player over the bundle: the closure of one file, and every serialized file
    let path = dir.join("test.bundle");
    let loose = dir.join("data");
    std::fs::create_dir_all(&loose).unwrap();
    for nodes in [NodeSelection::All, NodeSelection::Closure(&["CAB-1"])] {
        let spec = BundleSpec { path: &path, nodes };
        let package = TypeTreePackage::parse(&package_bytes()).unwrap();
        let player = Player::open_with_bundles(&loose, package, &[spec]).unwrap();
        let expected: &[&str] = if nodes == NodeSelection::All {
            &["cab-1", "other"]
        } else {
            &["cab-1"]
        };
        assert_eq!(
            player.files.keys().map(String::as_str).collect::<Vec<_>>(),
            expected
        );
        let r = ObjRef {
            file: "cab-1".into(),
            path_id: 1,
        };
        // the class comes from the stored tree's root: the package knows no class
        assert_eq!(player.class_name(&r), Some("GameObject"));
        let v = player.decode(&r).unwrap();
        assert_eq!(v.get("m_Count").and_then(|v| v.as_i64()), Some(7));
        assert_eq!(
            player
                .field(&r, "m_Label")
                .unwrap()
                .and_then(|v| v.as_str().map(str::to_owned)),
            Some("hi".to_owned())
        );
        assert_eq!(
            player
                .read_stream("cab-1", &stream("archive:/CAB-1/CAB-1.resS", 0, 4))
                .unwrap(),
            [0, 1, 2, 3]
        );
    }
    let e = Player::open_with_bundles(
        &loose,
        TypeTreePackage::parse(&package_bytes()).unwrap(),
        &[BundleSpec {
            path: &path,
            nodes: NodeSelection::Closure(&["CAB-1.resS"]),
        }],
    )
    .unwrap_err();
    assert!(e.to_string().contains("no serialized node"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}
