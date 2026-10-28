//! The SerializedFile container: what `globalgamemanagers`, `*.assets` and `levelN` are.
//!
//! Layout (container versions 21 and 22): a big-endian header followed by metadata in the file's
//! endianness: Unity version, target platform, whether type trees are embedded, the types (class id, stripped,
//! script index, hashes), the object table (path id, data offset, size, type index), script references, external
//! files, reference types and a user string. Objects' bytes follow at the data offset.
//! Version 21 stores metadata size, file size and data offset as 32-bit header fields, followed by an
//! endianness byte and three reserved bytes; metadata starts at byte 20. Version 22 repeats those sizes
//! after the legacy fields as 32/64/64-bit numbers plus eight reserved bytes; metadata starts at byte 48.
//! Object offsets are unsigned 32-bit in version 21 and signed 64-bit in version 22.
//!
//! Only versions verified against real data are read; others are refused. A file's types may carry their type trees
//! (an AssetBundle's files do; a player build strips them and objects are decoded with a [`TypeTreePackage`] instead):
//! they are read into [`SerializedType::tree`], their common-string offsets resolved from the package for the file's
//! Unity version.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::bundle::Bundle;
use crate::reader::Reader;
use crate::tpk::{CommonStrings, TypeTreePackage};
use crate::typetree::{Node, TypeTree};
use crate::value::Value;
use crate::version::UnityVersion;
use crate::{Error, Result, invalid};

/// Container versions read (verified on real files).
pub const VERSIONS: std::ops::RangeInclusive<u32> = 21..=22;

/// The MonoBehaviour class: its type tree covers the base fields; the script's own fields follow.
pub const MONO_BEHAVIOUR: i32 = 114;

/// One entry of a file's type table: the class the objects of this type are of, as the file declares it.
#[derive(Clone, Debug, PartialEq)]
pub struct SerializedType {
    /// Unity's class id (1 GameObject, 4 Transform, 114 MonoBehaviour, ...).
    pub class_id: i32,
    /// Whether the build stripped the class's type tree.
    pub stripped: bool,
    /// Index into the script references (-1: none).
    pub script_type_index: i16,
    /// The script's 16-byte hash: present for a MonoBehaviour type, and for a reference type whose script index is
    /// not -1.
    pub script_id: Option<[u8; 16]>,
    /// The 16-byte hash the file stores for the class's layout.
    pub old_type_hash: [u8; 16],
    /// The type tree the file stores for the type, when it stores trees (an AssetBundle's files do). A MonoBehaviour
    /// type's tree holds its script's fields too.
    pub tree: Option<Arc<TypeTree>>,
    /// The types a type's tree refers to (the file's type indices), for a type of a file that stores trees.
    pub dependencies: Vec<i32>,
    /// The managed class a reference type stands for, for a reference type of a file that stores trees.
    pub reference: Option<ReferenceClass>,
}

/// The managed class a reference type (`[SerializeReference]`) names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceClass {
    /// The class name.
    pub class: String,
    /// Its namespace.
    pub namespace: String,
    /// The assembly it is in.
    pub assembly: String,
}

/// One entry of the object table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    /// The object's id within its file; unique per file.
    pub path_id: i64,
    /// The object's bytes: offset in the file and size.
    pub offset: u64,
    /// The object's size in bytes.
    pub size: u32,
    /// Index into [`SerializedFile::types`].
    pub type_index: u32,
    /// The class id of the object's type.
    pub class_id: i32,
}

/// Another file this one's pointers can name (file id = index + 1).
#[derive(Clone, Debug, PartialEq)]
pub struct External {
    /// The external file's 16-byte GUID as stored.
    pub guid: [u8; 16],
    /// The file's kind as stored (`type`).
    pub kind: i32,
    /// The external file's path as stored; [`crate::player::Player::resolve`] names the file by its last path segment,
    /// lower-cased.
    pub path: String,
}

/// A decoded object: its value, and the bytes after what its type tree covers.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// The decoded object.
    pub value: Value,
    /// How many of the object's bytes lie after what the type tree covers.
    pub rest: usize,
}

impl Decoded {
    /// Whether the tree covered the object: every byte, or a MonoBehaviour's base fields (its script's fields follow,
    /// and a stripped build has no type trees for them).
    pub fn covered(&self, class_id: i32) -> bool {
        self.rest == 0 || class_id == MONO_BEHAVIOUR
    }
}

/// A parsed SerializedFile: its header fields, type table, object table, script references and external files, with
/// the file's bytes kept so objects can be read from it.
#[derive(Clone, Debug)]
pub struct SerializedFile {
    /// The container version (21 or 22: see [`VERSIONS`]).
    pub version: u32,
    /// The Unity version string the file was built with (`2019.4.20f1`).
    pub unity_version: String,
    /// [`SerializedFile::unity_version`], parsed.
    pub unity: UnityVersion,
    /// Unity's `BuildTarget` number the file was built for.
    pub target_platform: i32,
    /// Whether the metadata and the objects' bytes are big-endian.
    pub big_endian: bool,
    /// The type table.
    pub types: Vec<SerializedType>,
    /// The object table, in file order.
    pub objects: Vec<ObjectInfo>,
    /// Script references: (file index, path id).
    pub script_types: Vec<(i32, i64)>,
    /// The files this file's pointers can name (file id = index + 1).
    pub externals: Vec<External>,
    /// The table of types of managed references (`[SerializeReference]`).
    pub ref_types: Vec<SerializedType>,
    /// The user string at the end of the metadata.
    pub user_information: String,
    data: Backing,
    by_path_id: BTreeMap<i64, usize>,
}

/// Where a file's bytes are: in memory, or in a node of a bundle that is read by range as objects are asked for.
#[derive(Clone, Debug)]
enum Backing {
    Memory(Vec<u8>),
    Node { bundle: Arc<Bundle>, node: usize },
}

fn hash(r: &mut Reader) -> Result<[u8; 16]> {
    Ok(r.take(16)?.try_into().expect("16 bytes"))
}

fn count(r: &mut Reader, what: &str) -> Result<usize> {
    let n = r.i32()?;
    let n = usize::try_from(n).map_err(|_| invalid(format!("negative {what} count {n}")))?;
    if n > r.remaining() {
        return Err(invalid(format!("{what} count {n} larger than the metadata")));
    }
    Ok(n)
}

/// What reading a file's types needs besides its bytes: the common string table of its Unity version, and the
/// strings its trees repeat, shared.
struct TreeContext<'a> {
    common: Option<CommonStrings<'a>>,
    version: UnityVersion,
    strings: BTreeMap<String, Arc<str>>,
}

impl TreeContext<'_> {
    fn intern(&mut self, s: &str) -> Arc<str> {
        if let Some(a) = self.strings.get(s) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(s);
        self.strings.insert(s.to_owned(), a.clone());
        a
    }

    /// The string a node's offset names: in the tree's own buffer, or (top bit set) in the common string table.
    fn string(&mut self, buffer: &[u8], offset: u32) -> Result<Arc<str>> {
        if offset & COMMON_STRING != 0 {
            let at = offset & !COMMON_STRING;
            let common = self.common.ok_or_else(|| {
                Error::Unsupported(format!(
                    "a type tree names common string {at}: read the file with the type tree package of Unity {}",
                    self.version
                ))
            })?;
            let s = common.get(at).ok_or_else(|| {
                invalid(format!(
                    "a type tree names common string {at}, which the table of Unity {} lacks",
                    self.version
                ))
            })?;
            return Ok(self.intern(s));
        }
        let rest = usize::try_from(offset).ok().and_then(|o| buffer.get(o..));
        let bytes = rest
            .and_then(|r| r.iter().position(|&b| b == 0).and_then(|n| r.get(..n)))
            .ok_or_else(|| {
                invalid(format!(
                    "a type tree names string {offset}, outside its {} byte buffer",
                    buffer.len()
                ))
            })?;
        let s = std::str::from_utf8(bytes)
            .map_err(|_| invalid(format!("type tree string {offset} is not UTF-8")))?;
        Ok(self.intern(s))
    }
}

/// An offset with this bit set names a string of the engine's common string table instead of the tree's buffer.
const COMMON_STRING: u32 = 0x8000_0000;

/// The bytes of one embedded type tree node in the metadata (version 19 and later): version `i16`, level `u8`, type
/// flags `u8`, type string offset `u32`, name string offset `u32`, byte size `i32`, index `i32`, meta flags `u32`,
/// reference type hash `u64`.
const TREE_NODE_BYTES: usize = 32;

/// One type's embedded tree: a node count, the string buffer's size, the nodes (each [`TREE_NODE_BYTES`], with the
/// nesting level), the buffer. The nodes are in depth-first order; a node with a level is the child of the nearest
/// node before it with a lower one.
fn read_tree(r: &mut Reader, context: &mut TreeContext) -> Result<Arc<TypeTree>> {
    let nodes = r.i32()?;
    let buffer_size = r.i32()?;
    let nodes = usize::try_from(nodes)
        .ok()
        .filter(|&n| n > 0 && n <= r.remaining() / TREE_NODE_BYTES)
        .ok_or_else(|| invalid(format!("a type tree of {nodes} nodes in {} bytes", r.remaining())))?;
    let buffer_size = usize::try_from(buffer_size)
        .map_err(|_| invalid(format!("a type tree string buffer of {buffer_size} bytes")))?;
    let raw = r.take(nodes.saturating_mul(TREE_NODE_BYTES))?;
    let buffer = r.take(buffer_size)?;
    let mut nodes: Vec<Node> = Vec::with_capacity(nodes);
    // the open ancestors: node indices by level
    let mut open: Vec<usize> = Vec::new();
    let mut n = Reader::new(raw, r.big_endian);
    while n.remaining() > 0 {
        let version = n.i16()?;
        let level = usize::from(n.u8()?);
        let _type_flags = n.u8()?;
        let (type_offset, name_offset) = (n.u32()?, n.u32()?);
        let byte_size = n.i32()?;
        let _index = n.i32()?;
        let meta_flag = n.u32()?;
        let _reference_hash = n.u64()?;
        if level > open.len() || (level == 0 && !nodes.is_empty()) {
            return Err(invalid(format!(
                "type tree node {} is at level {level}, below {} open levels",
                nodes.len(),
                open.len()
            )));
        }
        open.truncate(level);
        let at = nodes.len();
        if let Some(&parent) = open.last() {
            nodes
                .get_mut(parent)
                .ok_or_else(|| invalid("a type tree node went missing"))?
                .children
                .push(at);
        }
        open.push(at);
        nodes.push(Node {
            ty: context.string(buffer, type_offset)?,
            name: context.string(buffer, name_offset)?,
            byte_size,
            version: i32::from(version),
            meta_flag,
            children: Vec::new(),
        });
    }
    Ok(Arc::new(TypeTree::new(nodes)?))
}

/// A type entry. `trees` reads the tree and what follows it when the file stores trees.
fn serialized_type(
    r: &mut Reader,
    reference: bool,
    trees: Option<&mut TreeContext>,
) -> Result<SerializedType> {
    let class_id = r.i32()?;
    let stripped = r.bool()?;
    let script_type_index = r.i16()?;
    let script_id = if (reference && script_type_index >= 0) || (!reference && class_id == MONO_BEHAVIOUR) {
        Some(hash(r)?)
    } else {
        None
    };
    let old_type_hash = hash(r)?;
    let (mut tree, mut dependencies, mut reference_class) = (None, Vec::new(), None);
    if let Some(context) = trees {
        tree = Some(read_tree(r, context)?);
        if reference {
            reference_class = Some(ReferenceClass {
                class: r.cstr()?,
                namespace: r.cstr()?,
                assembly: r.cstr()?,
            });
        } else {
            let n = count(r, "type dependency")?;
            dependencies = (0..n).map(|_| r.i32()).collect::<Result<Vec<_>>>()?;
        }
    }
    Ok(SerializedType {
        class_id,
        stripped,
        script_type_index,
        script_id,
        old_type_hash,
        tree,
        dependencies,
        reference: reference_class,
    })
}

/// A SerializedFile's header, version 21 or 22.
struct Header {
    version: u32,
    big_endian: bool,
    /// Where the metadata starts (the header's length).
    metadata_start: usize,
    metadata_size: usize,
    file_size: u64,
    data_offset: u64,
    /// The bytes from the start of the file to the end of the metadata.
    head_len: u64,
}

impl Header {
    /// The header at the start of `head`.
    fn of(head: &[u8]) -> Result<Header> {
        let mut h = Reader::new(head, true);
        let legacy_metadata = h.u32()?;
        let legacy_size = h.u32()?;
        let version = h.u32()?;
        let legacy_offset = h.u32()?;
        if !VERSIONS.contains(&version) {
            return Err(Error::Unsupported(format!(
                "SerializedFile version {version} (read: {}..={})",
                VERSIONS.start(),
                VERSIONS.end()
            )));
        }
        let big_endian = h.bool()?;
        h.take(3)?;
        let (metadata_size, file_size, data_offset) = if version == 22 {
            let metadata_size = h.u32()?;
            let file_size = h.u64()?;
            let data_offset = h.u64()?;
            let _reserved = h.u64()?;
            (metadata_size, file_size, data_offset)
        } else {
            (legacy_metadata, u64::from(legacy_size), u64::from(legacy_offset))
        };
        let metadata_size =
            usize::try_from(metadata_size).map_err(|_| invalid("metadata size overflows its byte range"))?;
        let head_len =
            u64::try_from(h.pos.saturating_add(metadata_size)).map_err(|_| invalid("metadata size"))?;
        Ok(Header {
            version,
            big_endian,
            metadata_start: h.pos,
            metadata_size,
            file_size,
            data_offset,
            head_len,
        })
    }
}

impl SerializedFile {
    /// Whether `bytes` start like a SerializedFile of a supported version (to tell them from other files).
    pub fn looks_like(bytes: &[u8]) -> bool {
        let be = |at: usize| -> Option<u32> {
            Some(u32::from_be_bytes(
                bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        let Ok(len) = u64::try_from(bytes.len()) else {
            return false;
        };
        let check = || -> Option<bool> {
            Some(match be(8)? {
                21 => {
                    let end = 20u64.checked_add(u64::from(be(0)?))?;
                    let (size, offset) = (u64::from(be(4)?), u64::from(be(12)?));
                    be(0)? > 0 && size == len && end <= offset && offset <= size
                }
                22 => len >= 48 && be(0)? == 0,
                _ => false,
            })
        };
        check().unwrap_or(false)
    }

    /// The container version of `bytes` when they are a SerializedFile of any version, supported or not: a supported
    /// version by its field alone (a damaged file of it is for [`SerializedFile::parse`] to refuse), another version
    /// only when the header's own size fields agree with `bytes.len()`, so that other files (streamed resources,
    /// configs) are not taken for one. `None` for bytes that are no SerializedFile.
    pub fn container_version(bytes: &[u8]) -> Option<u32> {
        let be = |at: usize| -> Option<u32> {
            Some(u32::from_be_bytes(
                bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
            ))
        };
        let len = u64::try_from(bytes.len()).ok()?;
        let version = be(8)?;
        if VERSIONS.contains(&version) {
            return Some(version);
        }
        let consistent = if version < *VERSIONS.start() {
            // the legacy header: metadata size, file size, version, data offset
            version > 0 && u64::from(be(4)?) == len && u64::from(be(12)?) <= len
        } else {
            // the header of 22 and later: the legacy fields are zero, the file size follows the metadata size
            be(0)? == 0
                && be(4)? == 0
                && be(12)? == 0
                && u64::from_be_bytes(bytes.get(24..32)?.try_into().ok()?) == len
        };
        consistent.then_some(version)
    }

    /// Parse the SerializedFile `data`. The header's sizes must match `data`, the object table must lie inside it and
    /// the metadata must end exactly where the header says; a version outside [`VERSIONS`], a duplicate path id and
    /// every length that runs past the bytes are refused with an [`Error`] that names what was found. A file that
    /// stores its type trees is read whole, but a string of the engine's common string table that a tree names is
    /// refused as [`Error::Unsupported`]: [`SerializedFile::parse_with`] reads such files.
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        Self::parse_with(data, None)
    }

    /// As [`SerializedFile::parse`], reading the common strings of embedded type trees from `package` (the table of the
    /// file's own Unity version).
    pub fn parse_with(data: Vec<u8>, package: Option<&TypeTreePackage>) -> Result<Self> {
        let size = u64::try_from(data.len()).map_err(|_| invalid("file past 2^64 bytes"))?;
        let mut file = Self::parse_head(&data, size, package)?;
        file.data = Backing::Memory(data);
        Ok(file)
    }

    /// Read the file `path` of `bundle` without reading its objects: the header and metadata are read, each object's
    /// bytes later, by range, when [`SerializedFile::bytes`] asks for them. `path` must name a serialized node.
    pub fn from_bundle(bundle: &Arc<Bundle>, path: &str, package: Option<&TypeTreePackage>) -> Result<Self> {
        let (index, node) = bundle
            .nodes()
            .iter()
            .enumerate()
            .find(|(_, n)| n.path == path)
            .ok_or_else(|| invalid(format!("{}: no node {path:?}", bundle.path().display())))?;
        let named = |e: Error| match e {
            Error::Invalid(m) => invalid(format!("{}: {path}: {m}", bundle.path().display())),
            Error::Unsupported(m) => Error::Unsupported(format!("{}: {path}: {m}", bundle.path().display())),
            other => other,
        };
        let first = bundle
            .read(
                node,
                0,
                usize::try_from(node.size.min(Self::HEADER_BYTES)).unwrap_or(0),
            )
            .map_err(named)?;
        let head_len = Header::of(&first).map_err(named)?.head_len;
        let head = bundle
            .read(
                node,
                0,
                usize::try_from(head_len.min(node.size)).map_err(|_| invalid("metadata size"))?,
            )
            .map_err(named)?;
        let mut file = Self::parse_head(&head, node.size, package).map_err(named)?;
        file.data = Backing::Node {
            bundle: bundle.clone(),
            node: index,
        };
        Ok(file)
    }

    /// The bytes that hold every header version read.
    const HEADER_BYTES: u64 = 48;

    /// Parse the header and metadata at the start of a file of `file_size` bytes, `head` being at least those; the
    /// returned file has no bytes of its own yet.
    fn parse_head(head: &[u8], file_size: u64, package: Option<&TypeTreePackage>) -> Result<Self> {
        let Header {
            version,
            big_endian,
            metadata_start,
            metadata_size,
            file_size: stated_size,
            data_offset,
            ..
        } = Header::of(head)?;
        if stated_size != file_size {
            return Err(invalid(format!(
                "header file size {stated_size}, file has {file_size} bytes"
            )));
        }
        let meta_end = metadata_start
            .checked_add(metadata_size)
            .ok_or_else(|| invalid("metadata size overflows its byte range"))?;
        if meta_end as u64 > data_offset || data_offset > file_size {
            return Err(invalid(format!(
                "metadata ends at {meta_end}, data at {data_offset}, file size {file_size}"
            )));
        }
        let metadata = head.get(..meta_end).ok_or_else(|| {
            invalid(format!(
                "metadata ends at {meta_end}, past the {} bytes read",
                head.len()
            ))
        })?;
        let mut r = Reader::new(metadata, big_endian);
        r.pos = metadata_start;
        let unity_version = r.cstr()?;
        let unity = UnityVersion::parse(&unity_version)?;
        let target_platform = r.i32()?;
        let has_trees = r.bool()?;
        let mut context = TreeContext {
            common: package.and_then(|p| p.common_strings(unity)),
            version: unity,
            strings: BTreeMap::new(),
        };
        let types = (0..count(&mut r, "type")?)
            .map(|_| serialized_type(&mut r, false, has_trees.then_some(&mut context)))
            .collect::<Result<Vec<_>>>()?;
        let mut objects = Vec::new();
        for _ in 0..count(&mut r, "object")? {
            r.align(4)?;
            let path_id = r.i64()?;
            let start = if version == 21 {
                i64::from(r.u32()?)
            } else {
                r.i64()?
            };
            let size = r.u32()?;
            let type_index = r.u32()?;
            let ty = types.get(type_index as usize).ok_or_else(|| {
                invalid(format!(
                    "object {path_id}: type index {type_index} of {}",
                    types.len()
                ))
            })?;
            let offset = u64::try_from(start)
                .ok()
                .and_then(|s| s.checked_add(data_offset))
                .filter(|&o| o.checked_add(u64::from(size)).is_some_and(|end| end <= file_size))
                .ok_or_else(|| invalid(format!("object {path_id}: bytes {start}+{size} outside the file")))?;
            objects.push(ObjectInfo {
                path_id,
                offset,
                size,
                type_index,
                class_id: ty.class_id,
            });
        }
        let mut script_types = Vec::new();
        for _ in 0..count(&mut r, "script")? {
            let file = r.i32()?;
            r.align(4)?;
            script_types.push((file, r.i64()?));
        }
        let mut externals = Vec::new();
        for _ in 0..count(&mut r, "external")? {
            let _empty = r.cstr()?;
            let guid = hash(&mut r)?;
            let kind = r.i32()?;
            let path = r.cstr()?;
            externals.push(External { guid, kind, path });
        }
        let ref_types = (0..count(&mut r, "reference type")?)
            .map(|_| serialized_type(&mut r, true, has_trees.then_some(&mut context)))
            .collect::<Result<Vec<_>>>()?;
        let user_information = r.cstr()?;
        if r.pos != meta_end {
            return Err(invalid(format!(
                "metadata read to {}, header says it ends at {meta_end}",
                r.pos
            )));
        }
        let mut by_path_id = BTreeMap::new();
        for (i, o) in objects.iter().enumerate() {
            if by_path_id.insert(o.path_id, i).is_some() {
                return Err(invalid(format!("path id {} twice", o.path_id)));
            }
        }
        Ok(SerializedFile {
            version,
            unity_version,
            unity,
            target_platform,
            big_endian,
            types,
            objects,
            script_types,
            externals,
            ref_types,
            user_information,
            data: Backing::Memory(Vec::new()),
            by_path_id,
        })
    }

    /// The object with this path id, if the file has one.
    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.by_path_id.get(&path_id).and_then(|&i| self.objects.get(i))
    }

    /// The type tree the file stores for the type of `o`; `None` for a file that stores none (a player's).
    pub fn tree_of(&self, o: &ObjectInfo) -> Option<&Arc<TypeTree>> {
        self.types.get(usize::try_from(o.type_index).ok()?)?.tree.as_ref()
    }

    /// The bytes of an object of this file. `o` should come from this file's own [`SerializedFile::objects`]: an
    /// [`ObjectInfo`] of another file that names bytes outside this one is an error naming the object and its range.
    /// A file read from memory lends its bytes; one read from a bundle ([`SerializedFile::from_bundle`]) reads them
    /// from the bundle, decompressing the blocks they lie in.
    pub fn bytes(&self, o: &ObjectInfo) -> Result<Cow<'_, [u8]>> {
        let outside = |len: u64| {
            invalid(format!(
                "object {}: bytes {}+{} are outside the file's {len} bytes",
                o.path_id, o.offset, o.size,
            ))
        };
        match &self.data {
            Backing::Memory(data) => {
                let len = u64::try_from(data.len()).unwrap_or(u64::MAX);
                let start = usize::try_from(o.offset).map_err(|_| outside(len))?;
                let end = start
                    .checked_add(usize::try_from(o.size).map_err(|_| outside(len))?)
                    .ok_or_else(|| outside(len))?;
                data.get(start..end)
                    .map(Cow::Borrowed)
                    .ok_or_else(|| outside(len))
            }
            Backing::Node { bundle, node } => {
                let node = bundle
                    .nodes()
                    .get(*node)
                    .ok_or_else(|| invalid("the file's bundle node is gone"))?;
                if o.offset
                    .checked_add(u64::from(o.size))
                    .is_none_or(|end| end > node.size)
                {
                    return Err(outside(node.size));
                }
                let len = usize::try_from(o.size).map_err(|_| outside(node.size))?;
                bundle.read(node, o.offset, len).map(Cow::Owned)
            }
        }
    }

    /// The bundle and node this file was read from ([`SerializedFile::from_bundle`]); `None` for a file read from
    /// memory.
    pub fn bundle(&self) -> Option<(&Arc<Bundle>, &crate::bundle::BundleNode)> {
        match &self.data {
            Backing::Memory(_) => None,
            Backing::Node { bundle, node } => bundle.nodes().get(*node).map(|n| (bundle, n)),
        }
    }

    /// Decode an object's bytes from its start with `tree` (its class's type tree), reporting the bytes after what the
    /// tree covers in [`Decoded::rest`] (see [`Decoded::covered`]).
    pub fn decode_with(&self, o: &ObjectInfo, tree: &TypeTree) -> Result<Decoded> {
        let bytes = self.bytes(o)?;
        let (value, used) = tree
            .decode(&bytes, self.big_endian)
            .map_err(|e| invalid(format!("object {} (class {}): {e}", o.path_id, o.class_id)))?;
        let rest = bytes.len().checked_sub(used).ok_or_else(|| {
            invalid(format!(
                "object {} (class {}): decoded {used} of {} bytes",
                o.path_id,
                o.class_id,
                bytes.len()
            ))
        })?;
        Ok(Decoded { value, rest })
    }

    /// Decode an object with its class's type tree from `package`. The tree must cover the object's bytes, but for a
    /// MonoBehaviour, whose script fields follow its base fields.
    pub fn decode(&self, o: &ObjectInfo, package: &TypeTreePackage) -> Result<Decoded> {
        let d = match self.tree_of(o) {
            Some(tree) => self.decode_with(o, tree)?,
            None => self.decode_with(o, &package.type_tree(o.class_id, self.unity)?)?,
        };
        if !d.covered(o.class_id) {
            return Err(invalid(format!(
                "object {} (class {}): {} bytes after its type tree",
                o.path_id, o.class_id, d.rest
            )));
        }
        Ok(d)
    }
}
