//! Hand-worked container headers: empty metadata and one object at a padded data boundary.
#![allow(clippy::unwrap_used)]

use unity_formats::SerializedFile;

/// The container versions read, each with the header that ends where its metadata starts.
const VERSIONS: [u32; 6] = [17, 18, 19, 20, 21, 22];

fn header_size(version: u32) -> usize {
    if version < 22 { 20 } else { 48 }
}

fn header(version: u32, metadata: &[u8], data: &[u8]) -> Vec<u8> {
    let header_size = header_size(version);
    let offset = header_size + metadata.len() + 7;
    let size = offset + data.len();
    let mut out = vec![0; header_size];
    out[8..12].copy_from_slice(&version.to_be_bytes());
    if version < 22 {
        out[0..4].copy_from_slice(&(metadata.len() as u32).to_be_bytes());
        out[4..8].copy_from_slice(&(size as u32).to_be_bytes());
        out[12..16].copy_from_slice(&(offset as u32).to_be_bytes());
    } else {
        out[20..24].copy_from_slice(&(metadata.len() as u32).to_be_bytes());
        out[24..32].copy_from_slice(&(size as u64).to_be_bytes());
        out[32..40].copy_from_slice(&(offset as u64).to_be_bytes());
    }
    out.extend_from_slice(metadata);
    out.resize(offset, 0);
    out.extend_from_slice(data);
    out
}

/// One object of path id 42 and type 0 at data offset 0, `size` bytes, aligned as the table aligns it.
fn object(metadata: &mut Vec<u8>, version: u32, size: u32) {
    // Both header sizes are divisible by four; align the object table's path ID.
    while !metadata.len().is_multiple_of(4) {
        metadata.push(0);
    }
    metadata.extend_from_slice(&42_i64.to_le_bytes());
    if version < 22 {
        metadata.extend_from_slice(&0_u32.to_le_bytes());
    } else {
        metadata.extend_from_slice(&0_i64.to_le_bytes());
    }
    metadata.extend_from_slice(&size.to_le_bytes());
    metadata.extend_from_slice(&0_u32.to_le_bytes());
}

fn fixture(version: u32) -> Vec<u8> {
    // Version string, platform 19, no embedded tree; one class 1 type with no script ID and a zero hash.
    let mut metadata = b"2019.4.20f1\0".to_vec();
    metadata.extend_from_slice(&19_i32.to_le_bytes());
    metadata.push(0);
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    metadata.push(0);
    metadata.extend_from_slice(&(-1_i16).to_le_bytes());
    metadata.extend_from_slice(&[0; 16]);
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    object(&mut metadata, version, 3);
    // No script references or externals, no reference types from version 20; empty user string.
    metadata.extend_from_slice(&[0; 8]);
    if version >= 20 {
        metadata.extend_from_slice(&[0; 4]);
    }
    metadata.push(0);
    header(version, &metadata, &[0xaa, 0xbb, 0xcc])
}

/// An embedded tree of two nodes, `Base m_Base` with the child `int m_X` of 4 bytes, with its string buffer: nodes of
/// 24 bytes before version 19, 32 (a reference type hash after the meta flags) from then.
fn tree(metadata: &mut Vec<u8>, version: u32) {
    let strings = b"Base\0m_Base\0int\0m_X\0";
    metadata.extend_from_slice(&2_i32.to_le_bytes());
    metadata.extend_from_slice(&(strings.len() as i32).to_le_bytes());
    for (level, ty, name, size) in [(0_u8, 0_u32, 5_u32, -1_i32), (1, 12, 16, 4)] {
        metadata.extend_from_slice(&1_i16.to_le_bytes());
        metadata.push(level);
        metadata.push(0);
        metadata.extend_from_slice(&ty.to_le_bytes());
        metadata.extend_from_slice(&name.to_le_bytes());
        metadata.extend_from_slice(&size.to_le_bytes());
        metadata.extend_from_slice(&i32::from(level).to_le_bytes());
        metadata.extend_from_slice(&0_u32.to_le_bytes());
        if version >= 19 {
            metadata.extend_from_slice(&0x1122_3344_5566_7788_u64.to_le_bytes());
        }
    }
    metadata.extend_from_slice(strings);
}

/// A file of `version` that stores its trees: a class 1 type and, from version 20, one reference type, each with
/// [`tree`]; type dependencies (none) and the reference type's managed class follow the trees from version 21.
fn fixture_with_trees(version: u32) -> Vec<u8> {
    let mut metadata = b"2018.4.11f1\0".to_vec();
    metadata.extend_from_slice(&19_i32.to_le_bytes());
    metadata.push(1);
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    metadata.push(0);
    metadata.extend_from_slice(&(-1_i16).to_le_bytes());
    metadata.extend_from_slice(&[0x5a; 16]);
    tree(&mut metadata, version);
    if version >= 21 {
        metadata.extend_from_slice(&0_i32.to_le_bytes());
    }
    metadata.extend_from_slice(&1_i32.to_le_bytes());
    object(&mut metadata, version, 4);
    metadata.extend_from_slice(&[0; 8]);
    if version >= 20 {
        // one reference type: class 0, not stripped, script index 0 (a script ID follows), its hash and tree
        metadata.extend_from_slice(&1_i32.to_le_bytes());
        metadata.extend_from_slice(&0_i32.to_le_bytes());
        metadata.push(0);
        metadata.extend_from_slice(&0_i16.to_le_bytes());
        metadata.extend_from_slice(&[0x11; 16]);
        metadata.extend_from_slice(&[0x22; 16]);
        tree(&mut metadata, version);
        if version >= 21 {
            metadata.extend_from_slice(b"Thing\0Game\0Assembly-CSharp\0");
        }
    }
    metadata.extend_from_slice(b"user\0");
    header(version, &metadata, &7_i32.to_le_bytes())
}

#[test]
fn versions_place_metadata_after_their_headers_and_read_object_widths() {
    for version in VERSIONS {
        let bytes = fixture(version);
        let size = bytes.len();
        assert!(SerializedFile::looks_like(&bytes));
        let file = SerializedFile::parse(bytes).unwrap();
        assert_eq!(file.version, version);
        assert_eq!(file.objects.len(), 1);
        let object = file.object(42).unwrap();
        assert_eq!(object.class_id, 1);
        assert_eq!(object.offset, (size - 3) as u64);
        assert_eq!(&*file.bytes(object).unwrap(), &[0xaa, 0xbb, 0xcc]);
        // an ObjectInfo naming bytes outside the file is an error, not a panic
        let outside = unity_formats::ObjectInfo {
            offset: u64::MAX - 1,
            ..*object
        };
        assert!(
            file.bytes(&outside)
                .unwrap_err()
                .to_string()
                .contains("outside the file")
        );
    }
}

#[test]
fn both_headers_refuse_metadata_overlap_and_data_past_eof() {
    for version in VERSIONS {
        for offset in [0, u64::MAX] {
            let mut bytes = fixture(version);
            if version < 22 {
                bytes[12..16].copy_from_slice(&(offset as u32).to_be_bytes());
            } else {
                bytes[32..40].copy_from_slice(&offset.to_be_bytes());
            }
            let error = SerializedFile::parse(bytes).unwrap_err().to_string();
            assert!(error.contains("metadata ends"), "{error}");
        }
        let mut bytes = fixture(version);
        let at = if version < 22 { 0 } else { 20 };
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(SerializedFile::parse(bytes).is_err());
    }
}

#[test]
fn truncated_headers_and_unsupported_versions_are_refused() {
    for version in VERSIONS {
        let bytes = fixture(version);
        for size in 0..header_size(version) {
            assert!(!SerializedFile::looks_like(&bytes[..size]));
            assert!(SerializedFile::parse(bytes[..size].to_vec()).is_err());
        }
    }
    for version in [16_u32, 23] {
        let mut bytes = fixture(21);
        bytes[8..12].copy_from_slice(&version.to_be_bytes());
        assert!(!SerializedFile::looks_like(&bytes));
        assert!(SerializedFile::parse(bytes).is_err());
    }
}

#[test]
fn embedded_trees_and_reference_types_follow_their_versions() {
    for version in VERSIONS {
        let file =
            SerializedFile::parse(fixture_with_trees(version)).unwrap_or_else(|e| panic!("{version}: {e}"));
        assert_eq!(file.unity_version, "2018.4.11f1", "{version}");
        assert_eq!(file.user_information, "user", "{version}");
        let ty = &file.types[0];
        assert_eq!((ty.class_id, ty.old_type_hash), (1, [0x5a; 16]), "{version}");
        assert!(ty.dependencies.is_empty(), "{version}");
        let object = file.object(42).unwrap();
        let tree = file.tree_of(object).unwrap();
        let decoded = file.decode_with(object, tree).unwrap();
        assert_eq!(decoded.rest, 0, "{version}");
        assert_eq!(
            decoded.value.get("m_X").and_then(|v| v.as_i64()),
            Some(7),
            "{version}"
        );
        if version < 20 {
            assert!(file.ref_types.is_empty(), "{version}");
            continue;
        }
        let reference = &file.ref_types[0];
        assert_eq!(
            (reference.script_id, reference.old_type_hash),
            (Some([0x11; 16]), [0x22; 16]),
            "{version}"
        );
        assert!(reference.tree.is_some(), "{version}");
        let class = reference
            .reference
            .as_ref()
            .map(|c| (c.class.as_str(), c.namespace.as_str(), c.assembly.as_str()));
        assert_eq!(
            class,
            (version >= 21).then_some(("Thing", "Game", "Assembly-CSharp")),
            "{version}"
        );
    }
}
