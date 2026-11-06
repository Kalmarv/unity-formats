//! UnityFS bundles: the archive AssetBundles, Addressables content and a player's `data.unity3d` are.
//!
//! Layout (format versions 6 to 8; every number big-endian): the signature `UnityFS\0`, the format version, the
//! player version and the engine version (NUL-terminated), the bundle's size (`u64`), the compressed and the
//! uncompressed size of the *block table* (`u32` each) and the header flags; then, from format 7 on aligned to 16 bytes,
//! the block table, compressed as the low six bits of the flags say (0 none, 1 LZMA, 2 LZ4, 3 LZ4HC), unless flag `0x80`
//! puts it at the end of the file instead; then, when flag `0x200` is set, padding to 16 bytes; then the blocks. The decompressed block table holds a 16-byte hash, the blocks (`u32`
//! uncompressed size, `u32` compressed size, `u16` flags whose low six bits are the block's compression) and the nodes:
//! the files in the blocks' concatenated bytes (`i64` offset, `i64` size, `u32` flags, a NUL-terminated path; flag `4`
//! marks a SerializedFile, `0` a stream file such as `.resS`).
//!
//! The bundles of PEAK, Sledding Game and Big Walk are format 8, flags `0x243`, blocks stored or LZ4HC of at most
//! 128 KiB. LZMA, as Unity writes it (the five property bytes and the raw stream, as UnityPy and AssetStudio read
//! it), the formats before 8 and the table at the end of the file are read as UnityPy and AssetStudio read them, on
//! bundles built in the tests. What it refuses, as [`Error::Unsupported`]: another format version, other header flags
//! (a table not stored with the directory), a LZHAM compressed block table or block, and compression numbers it does
//! not know. A bundle whose sizes do not add up is
//! [`Error::Invalid`]. A node is read by range: only the blocks it touches are decompressed, a few recent blocks are
//! kept, so a node of a gigabyte costs its reader nothing until it asks for bytes.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reader I/O: reads bundle files"
)]

use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use primase_foundations::compress::{Decompressor, Lz4, Lzma};

use crate::reader::Reader;
use crate::{Error, Result, invalid};

const MAGIC: &[u8; 8] = b"UnityFS\0";

/// The format versions read: 6, and 7 on, whose block table starts at a multiple of 16.
pub const FORMAT_VERSIONS: std::ops::RangeInclusive<u32> = 6..=8;

/// Block table flags: the low six bits name its compression.
const COMPRESSION_MASK: u32 = 0x3f;
/// The block table is stored with the directory (the only layout read).
const TABLE_WITH_DIRECTORY: u32 = 0x40;
/// The block table is at the end of the file instead of after the header; the blocks follow the header.
const TABLE_AT_END: u32 = 0x80;
/// The blocks start at the next multiple of 16 after the table.
const PAD_BEFORE_DATA: u32 = 0x200;

/// Node flag: the node is a SerializedFile.
pub const NODE_SERIALIZED: u32 = 4;

/// How many decompressed blocks a [`Bundle`] keeps for ranged reads.
const CACHE_BLOCKS: usize = 16;

/// The fixed fields of a bundle's header, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleHeader {
    /// The format version (8).
    pub format_version: u32,
    /// The player version string (`5.x.x` in every bundle read).
    pub player_version: String,
    /// The engine version the bundle was built with (`6000.3.15f1`).
    pub engine_version: String,
    /// The bundle's size in bytes; equal to the file's length.
    pub size: u64,
    /// The compressed size of the block table.
    pub table_compressed: u32,
    /// The uncompressed size of the block table.
    pub table_uncompressed: u32,
    /// The header flags (`0x243`).
    pub flags: u32,
}

/// One block of the bundle's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// The block's size once decompressed.
    pub uncompressed: u32,
    /// The block's size as stored.
    pub compressed: u32,
    /// The block's flags; the low six bits are its compression (0 stored, 1 LZMA, 2 LZ4, 3 LZ4HC).
    pub flags: u16,
}

/// One file in a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleNode {
    /// The file's path in the bundle (`BuildPlayer-Level_5.sharedAssets`, `Resources/unity_builtin_extra`).
    pub path: String,
    /// Where the file starts in the blocks' decompressed bytes.
    pub offset: u64,
    /// The file's size in bytes.
    pub size: u64,
    /// The node's flags: [`NODE_SERIALIZED`] (4) for a SerializedFile, 0 for a stream file.
    pub flags: u32,
}

impl BundleNode {
    /// Whether the node is a SerializedFile (its flags say so).
    pub fn is_serialized(&self) -> bool {
        self.flags & NODE_SERIALIZED != 0
    }
}

/// What a bundle's bytes come from: a file, or any reader that can seek.
pub trait BundleSource: Read + Seek + Send {}
impl<T: Read + Seek + Send> BundleSource for T {}

/// A block's place: where its decompressed bytes start in the data, and where its stored bytes start in the file.
#[derive(Clone, Copy, Debug)]
struct Placed {
    block: Block,
    data_start: u64,
    file_start: u64,
}

/// An open UnityFS bundle: its header, blocks and nodes, with the file kept open to read nodes by range.
pub struct Bundle {
    path: PathBuf,
    header: BundleHeader,
    hash: [u8; 16],
    nodes: Vec<BundleNode>,
    blocks: Vec<Placed>,
    data_len: u64,
    source: Mutex<Box<dyn BundleSource>>,
    recent: Mutex<Vec<(usize, Arc<[u8]>)>>,
}

impl fmt::Debug for Bundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bundle")
            .field("path", &self.path)
            .field("header", &self.header)
            .field("nodes", &self.nodes.len())
            .field("blocks", &self.blocks.len())
            .finish()
    }
}

fn unsupported_compression(what: &str, code: u32) -> Error {
    let name = match code {
        4 => "LZHAM",
        _ => "an unknown compression",
    };
    Error::Unsupported(format!(
        "{what} compressed with {name} (compression {code}); stored, LZMA, LZ4 and LZ4HC are read"
    ))
}

/// `bytes` decompressed as `compression` says to `expected` bytes.
fn inflate(what: &str, compression: u32, bytes: &[u8], expected: usize) -> Result<Vec<u8>> {
    match compression {
        0 => {
            if bytes.len() == expected {
                Ok(bytes.to_vec())
            } else {
                Err(invalid(format!(
                    "{what} is stored with {} bytes, its size is {expected}",
                    bytes.len()
                )))
            }
        }
        1 => Lzma
            .decompress(bytes, expected)
            .map_err(|e| invalid(format!("{what}: LZMA: {e}"))),
        2 | 3 => Lz4
            .decompress(bytes, expected)
            .map_err(|e| invalid(format!("{what}: LZ4: {e}"))),
        c => Err(unsupported_compression(what, c)),
    }
}

impl Bundle {
    /// Open the bundle at `path`: read its header and block table and check that the sizes add up (the header's size is
    /// the file's, the blocks end where the file does, every node lies in the blocks' bytes). Reads nothing of the data.
    pub fn open(path: &Path) -> Result<Self> {
        let io = |source| Error::Io {
            path: path.to_path_buf(),
            source,
        };
        let file = std::fs::File::open(path).map_err(io)?;
        Self::from_source(path, Box::new(file))
    }

    /// As [`Bundle::open`], on any seekable `source` (`path` only names it in errors).
    pub fn from_source(path: &Path, mut source: Box<dyn BundleSource>) -> Result<Self> {
        let named = |e: Error| match e {
            Error::Invalid(m) => invalid(format!("{}: {m}", path.display())),
            Error::Unsupported(m) => Error::Unsupported(format!("{}: {m}", path.display())),
            other => other,
        };
        Self::parse(path, &mut source)
            .map_err(named)
            .map(|parsed| Bundle {
                path: path.to_path_buf(),
                header: parsed.header,
                hash: parsed.hash,
                nodes: parsed.nodes,
                blocks: parsed.blocks,
                data_len: parsed.data_len,
                source: Mutex::new(source),
                recent: Mutex::new(Vec::new()),
            })
    }

    fn parse(path: &Path, source: &mut Box<dyn BundleSource>) -> Result<Parsed> {
        let io = |source| Error::Io {
            path: path.to_path_buf(),
            source,
        };
        let len = source.seek(SeekFrom::End(0)).map_err(io)?;
        source.seek(SeekFrom::Start(0)).map_err(io)?;
        // the header up to its flags: two short strings and 28 bytes of numbers
        let mut head = Vec::new();
        source.by_ref().take(1024).read_to_end(&mut head).map_err(io)?;
        let mut r = Reader::new(&head, true);
        if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
            return Err(invalid("not a UnityFS bundle (signature)"));
        }
        let format_version = r.u32()?;
        if !FORMAT_VERSIONS.contains(&format_version) {
            return Err(Error::Unsupported(format!(
                "UnityFS format version {format_version} (read: {} to {})",
                FORMAT_VERSIONS.start(),
                FORMAT_VERSIONS.end()
            )));
        }
        let player_version = r.cstr()?;
        let engine_version = r.cstr()?;
        let size = r.u64()?;
        let table_compressed = r.u32()?;
        let table_uncompressed = r.u32()?;
        let flags = r.u32()?;
        let header = BundleHeader {
            format_version,
            player_version,
            engine_version,
            size,
            table_compressed,
            table_uncompressed,
            flags,
        };
        if size != len {
            return Err(invalid(format!(
                "the header says {size} bytes, the file has {len}"
            )));
        }
        let known = COMPRESSION_MASK | TABLE_WITH_DIRECTORY | TABLE_AT_END | PAD_BEFORE_DATA;
        if flags & !known != 0 {
            return Err(Error::Unsupported(format!(
                "UnityFS header flags {flags:#x} (read: {TABLE_WITH_DIRECTORY:#x} plus a compression, with or without \
                 {PAD_BEFORE_DATA:#x})"
            )));
        }
        if flags & TABLE_WITH_DIRECTORY == 0 {
            return Err(Error::Unsupported(format!(
                "UnityFS header flags {flags:#x}: the block table not stored with the directory"
            )));
        }
        // from format 7 on, what follows the header starts at the next multiple of 16
        let header_end = u64::try_from(r.pos).map_err(|_| invalid("header too long"))?;
        let header_end = if format_version >= 7 {
            header_end
                .checked_next_multiple_of(16)
                .ok_or_else(|| invalid("header too long"))?
        } else {
            header_end
        };
        let at_end = flags & TABLE_AT_END != 0;
        let table_at = if at_end {
            len.checked_sub(u64::from(table_compressed))
                .filter(|&at| at >= header_end)
        } else {
            Some(header_end)
        };
        let (table_at, table_end) = table_at
            .and_then(|at| Some((at, at.checked_add(u64::from(table_compressed))?)))
            .filter(|&(_, end)| end <= len)
            .ok_or_else(|| {
                invalid(format!(
                    "the block table ({table_compressed} bytes) does not fit the file's {len} bytes after its \
                     {header_end}-byte header"
                ))
            })?;
        // the blocks follow the table, or the header when the table is at the end; they end where the table starts
        let (before_data, data_end) = if at_end {
            (header_end, table_at)
        } else {
            (table_end, len)
        };
        source.seek(SeekFrom::Start(table_at)).map_err(io)?;
        let mut packed = vec![0; usize::try_from(table_compressed).map_err(|_| invalid("block table size"))?];
        source.read_exact(&mut packed).map_err(io)?;
        let table = inflate(
            "the block table",
            flags & COMPRESSION_MASK,
            &packed,
            usize::try_from(table_uncompressed).map_err(|_| invalid("block table size"))?,
        )?;
        let mut t = Reader::new(&table, true);
        let hash: [u8; 16] = t
            .take(16)?
            .try_into()
            .map_err(|_| invalid("the block table's hash"))?;
        let block_count = t.i32()?;
        let block_count = usize::try_from(block_count)
            .ok()
            .filter(|&n| n <= t.remaining() / 10)
            .ok_or_else(|| {
                invalid(format!(
                    "{block_count} blocks in a block table of {} bytes",
                    table.len()
                ))
            })?;
        let data_at = if flags & PAD_BEFORE_DATA != 0 {
            before_data
                .checked_next_multiple_of(16)
                .ok_or_else(|| invalid("data offset"))?
        } else {
            before_data
        };
        let mut blocks = Vec::with_capacity(block_count);
        let (mut data_len, mut file_at) = (0u64, data_at);
        for i in 0..block_count {
            let block = Block {
                uncompressed: t.u32()?,
                compressed: t.u32()?,
                flags: t.u16()?,
            };
            let compression = u32::from(block.flags) & COMPRESSION_MASK;
            if !matches!(compression, 0..=3) {
                return Err(unsupported_compression(&format!("block {i}"), compression));
            }
            if compression == 0 && block.compressed != block.uncompressed {
                return Err(invalid(format!(
                    "block {i} is stored with {} bytes, its size is {}",
                    block.compressed, block.uncompressed
                )));
            }
            blocks.push(Placed {
                block,
                data_start: data_len,
                file_start: file_at,
            });
            data_len = data_len
                .checked_add(u64::from(block.uncompressed))
                .ok_or_else(|| invalid("blocks add up past 2^64 bytes"))?;
            file_at = file_at
                .checked_add(u64::from(block.compressed))
                .ok_or_else(|| invalid("blocks add up past 2^64 bytes"))?;
        }
        if file_at != data_end {
            return Err(invalid(format!(
                "the blocks end at {file_at} (data at {data_at}), not at {data_end} where the {} ends",
                if at_end {
                    "data before the block table"
                } else {
                    "file"
                }
            )));
        }
        let node_count = t.i32()?;
        let node_count = usize::try_from(node_count)
            .ok()
            .filter(|&n| n <= t.remaining() / 21)
            .ok_or_else(|| invalid(format!("{node_count} nodes in {} bytes", t.remaining())))?;
        let mut nodes: Vec<BundleNode> = Vec::with_capacity(node_count);
        for i in 0..node_count {
            let offset = t.i64()?;
            let size = t.i64()?;
            let flags = t.u32()?;
            let path = t.cstr()?;
            let (offset, size) = u64::try_from(offset)
                .ok()
                .zip(u64::try_from(size).ok())
                .ok_or_else(|| invalid(format!("node {i} ({path}): offset {offset}, size {size}")))?;
            if offset.checked_add(size).is_none_or(|end| end > data_len) {
                return Err(invalid(format!(
                    "node {i} ({path}): bytes {offset}+{size} are outside the blocks' {data_len} bytes"
                )));
            }
            if path.is_empty() || nodes.iter().any(|n| n.path == path) {
                return Err(invalid(format!(
                    "node {i}: the path {path:?} is empty or twice in the bundle"
                )));
            }
            nodes.push(BundleNode {
                path,
                offset,
                size,
                flags,
            });
        }
        if t.remaining() != 0 {
            return Err(invalid(format!(
                "{} bytes after the nodes in the block table",
                t.remaining()
            )));
        }
        Ok(Parsed {
            header,
            hash,
            nodes,
            blocks,
            data_len,
        })
    }

    /// The file the bundle was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The header as stored.
    pub fn header(&self) -> &BundleHeader {
        &self.header
    }

    /// The 16 bytes at the start of the block table (nothing reads them).
    pub fn hash(&self) -> [u8; 16] {
        self.hash
    }

    /// The files in the bundle, in the order the directory lists them.
    pub fn nodes(&self) -> &[BundleNode] {
        &self.nodes
    }

    /// The node whose path is exactly `path`.
    pub fn node(&self, path: &str) -> Option<&BundleNode> {
        self.nodes.iter().find(|n| n.path == path)
    }

    /// The blocks, in data order.
    pub fn blocks(&self) -> impl ExactSizeIterator<Item = Block> + '_ {
        self.blocks.iter().map(|p| p.block)
    }

    /// The size of all blocks once decompressed.
    pub fn data_len(&self) -> u64 {
        self.data_len
    }

    /// Block `i` decompressed.
    fn inflate_block(&self, i: usize) -> Result<Vec<u8>> {
        let placed = self
            .blocks
            .get(i)
            .ok_or_else(|| invalid(format!("block {i} of {}", self.blocks.len())))?;
        let mut packed =
            vec![0; usize::try_from(placed.block.compressed).map_err(|_| invalid("block size"))?];
        {
            let io = |source| Error::Io {
                path: self.path.clone(),
                source,
            };
            let mut source = self.source.lock().unwrap_or_else(PoisonError::into_inner);
            source.seek(SeekFrom::Start(placed.file_start)).map_err(io)?;
            source.read_exact(&mut packed).map_err(io)?;
        }
        inflate(
            &format!("{}: block {i}", self.path.display()),
            u32::from(placed.block.flags) & COMPRESSION_MASK,
            &packed,
            usize::try_from(placed.block.uncompressed).map_err(|_| invalid("block size"))?,
        )
    }

    /// Block `i` decompressed, from the recent blocks when it is one of them.
    fn block(&self, i: usize) -> Result<Arc<[u8]>> {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(at) = recent.iter().position(|(n, _)| *n == i) {
            let hit = recent.remove(at);
            let data = hit.1.clone();
            recent.push(hit);
            return Ok(data);
        }
        drop(recent);
        let data: Arc<[u8]> = self.inflate_block(i)?.into();
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        if recent.len() >= CACHE_BLOCKS {
            recent.remove(0);
        }
        recent.push((i, data.clone()));
        Ok(data)
    }

    /// `len` bytes of `node`, from `offset` bytes into it. Only the blocks the range touches are decompressed. A range
    /// outside the node is an error.
    pub fn read(&self, node: &BundleNode, offset: u64, len: usize) -> Result<Vec<u8>> {
        let outside = || {
            invalid(format!(
                "{}: {}: bytes {offset}+{len} are outside the node's {} bytes",
                self.path.display(),
                node.path,
                node.size
            ))
        };
        let want = u64::try_from(len).map_err(|_| outside())?;
        if offset.checked_add(want).is_none_or(|end| end > node.size) {
            return Err(outside());
        }
        let start = node.offset.checked_add(offset).ok_or_else(outside)?;
        let end = start.checked_add(want).ok_or_else(outside)?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| invalid(format!("{len} bytes do not fit memory")))?;
        let mut i = self
            .blocks
            .partition_point(|p| p.data_start.saturating_add(u64::from(p.block.uncompressed)) <= start);
        while out.len() < len {
            let placed = self.blocks.get(i).ok_or_else(|| {
                invalid(format!(
                    "{}: ran out of blocks at {}",
                    self.path.display(),
                    out.len()
                ))
            })?;
            let data = self.block(i)?;
            let from = start.max(placed.data_start).saturating_sub(placed.data_start);
            let to = end
                .min(
                    placed
                        .data_start
                        .saturating_add(u64::from(placed.block.uncompressed)),
                )
                .saturating_sub(placed.data_start);
            let slice = usize::try_from(from)
                .ok()
                .zip(usize::try_from(to).ok())
                .and_then(|(f, t)| data.get(f..t))
                .ok_or_else(|| {
                    invalid(format!(
                        "{}: block {i} is shorter than its size",
                        self.path.display()
                    ))
                })?;
            out.extend_from_slice(slice);
            i = i.saturating_add(1);
        }
        Ok(out)
    }

    /// All of `node`'s bytes. Blocks are decompressed one after the other and not kept.
    pub fn read_node(&self, node: &BundleNode) -> Result<Vec<u8>> {
        let len = usize::try_from(node.size)
            .map_err(|_| invalid(format!("{}: {} bytes do not fit memory", node.path, node.size)))?;
        let mut out = Vec::new();
        out.try_reserve_exact(len)
            .map_err(|_| invalid(format!("{}: {len} bytes do not fit memory", node.path)))?;
        let start = node.offset;
        let end = start
            .checked_add(node.size)
            .ok_or_else(|| invalid("node past 2^64 bytes"))?;
        let first = self
            .blocks
            .partition_point(|p| p.data_start.saturating_add(u64::from(p.block.uncompressed)) <= start);
        for i in first..self.blocks.len() {
            let Some(placed) = self.blocks.get(i) else { break };
            if placed.data_start >= end {
                break;
            }
            let data = self.inflate_block(i)?;
            let from = start.max(placed.data_start).saturating_sub(placed.data_start);
            let to = end
                .min(
                    placed
                        .data_start
                        .saturating_add(u64::from(placed.block.uncompressed)),
                )
                .saturating_sub(placed.data_start);
            let slice = usize::try_from(from)
                .ok()
                .zip(usize::try_from(to).ok())
                .and_then(|(f, t)| data.get(f..t))
                .ok_or_else(|| {
                    invalid(format!(
                        "{}: block {i} is shorter than its size",
                        self.path.display()
                    ))
                })?;
            out.extend_from_slice(slice);
        }
        if out.len() != len {
            return Err(invalid(format!(
                "{}: {} read {} of its {len} bytes",
                self.path.display(),
                node.path,
                out.len()
            )));
        }
        Ok(out)
    }
}

/// What [`Bundle::parse`] reads, before the source is kept.
struct Parsed {
    header: BundleHeader,
    hash: [u8; 16],
    nodes: Vec<BundleNode>,
    blocks: Vec<Placed>,
    data_len: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// An LZ4 block of one literal run, encoded by hand.
    fn literal_lz4(data: &[u8]) -> Vec<u8> {
        assert!(data.len() >= 15);
        let mut out = vec![0xf0];
        let mut rest = data.len() - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(u8::try_from(rest).unwrap());
        out.extend(data);
        out
    }

    /// LZMA as Unity stores it: the five property bytes and the raw stream, no size and no end marker.
    fn unity_lzma(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let options = lzma_rs::compress::Options {
            unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
        };
        lzma_rs::lzma_compress_with_options(&mut &data[..], &mut out, &options).unwrap();
        out
    }

    fn compressed(data: &[u8], compression: u32) -> Vec<u8> {
        match compression {
            0 => data.to_vec(),
            1 => unity_lzma(data),
            _ => literal_lz4(data),
        }
    }

    /// A bundle of format 8 built the way the games' are: the table with the directory, padded to 16, data padded.
    /// Each block is (bytes, compression).
    fn build(
        blocks: &[(&[u8], u32)],
        nodes: &[(i64, i64, u32, &str)],
        table_compression: u32,
        flags: u32,
    ) -> Vec<u8> {
        build_format(8, blocks, nodes, table_compression, flags)
    }

    /// A bundle of a format version as UnityPy and AssetStudio read one: the header aligned to 16 from format 7 on,
    /// then the table (or, with flag 0x80, the blocks, and the table last), the blocks padded to 16 with flag 0x200.
    fn build_format(
        format: u32,
        blocks: &[(&[u8], u32)],
        nodes: &[(i64, i64, u32, &str)],
        table_compression: u32,
        flags: u32,
    ) -> Vec<u8> {
        let mut table = vec![7u8; 16];
        table.extend((blocks.len() as i32).to_be_bytes());
        let mut data = Vec::new();
        for (bytes, compression) in blocks {
            let stored = compressed(bytes, *compression);
            table.extend((bytes.len() as u32).to_be_bytes());
            table.extend((stored.len() as u32).to_be_bytes());
            table.extend((*compression as u16).to_be_bytes());
            data.extend(stored);
        }
        table.extend((nodes.len() as i32).to_be_bytes());
        for (offset, size, flags, path) in nodes {
            table.extend(offset.to_be_bytes());
            table.extend(size.to_be_bytes());
            table.extend(flags.to_be_bytes());
            table.extend(path.as_bytes());
            table.push(0);
        }
        let packed = compressed(&table, table_compression);
        let mut out = b"UnityFS\0".to_vec();
        out.extend(format.to_be_bytes());
        out.extend(b"5.x.x\0");
        out.extend(b"6000.3.15f1\0");
        let size_at = out.len();
        out.extend([0; 8]);
        out.extend((packed.len() as u32).to_be_bytes());
        out.extend((table.len() as u32).to_be_bytes());
        out.extend(flags.to_be_bytes());
        if format >= 7 {
            out.resize(out.len().next_multiple_of(16), 0);
        }
        let at_end = flags & TABLE_AT_END != 0;
        if !at_end {
            out.extend(&packed);
        }
        if flags & PAD_BEFORE_DATA != 0 {
            out.resize(out.len().next_multiple_of(16), 0);
        }
        out.extend(data);
        if at_end {
            out.extend(&packed);
        }
        let total = out.len() as u64;
        out[size_at..size_at + 8].copy_from_slice(&total.to_be_bytes());
        out
    }

    fn open(bytes: Vec<u8>) -> Result<Bundle> {
        Bundle::from_source(Path::new("test.bundle"), Box::new(Cursor::new(bytes)))
    }

    fn sample() -> Vec<u8> {
        let a: Vec<u8> = (0..40u8).collect();
        let b: Vec<u8> = (100..130u8).collect();
        let c: Vec<u8> = (200..240u8).collect();
        build(
            &[(&a, 3), (&b, 0), (&c, 2)],
            &[
                (0, 25, 4, "first"),
                (25, 70, 0, "Resources/second"),
                (95, 15, 4, "third"),
            ],
            3,
            0x243,
        )
    }

    /// Nodes are read whole and by range across stored and LZ4 blocks, the same bytes either way.
    #[test]
    fn nodes_read_whole_and_by_range_across_blocks() {
        let bundle = open(sample()).unwrap();
        assert_eq!(bundle.header().engine_version, "6000.3.15f1");
        assert_eq!(bundle.header().flags, 0x243);
        assert_eq!(bundle.blocks().len(), 3);
        assert_eq!(bundle.data_len(), 110);
        let all: Vec<u8> = (0..40u8).chain(100..130).chain(200..240).collect();
        for node in bundle.nodes() {
            let (from, size) = (node.offset as usize, node.size as usize);
            assert_eq!(
                bundle.read_node(node).unwrap(),
                all[from..from + size],
                "{}",
                node.path
            );
            assert_eq!(bundle.read(node, 0, size).unwrap(), all[from..from + size]);
            for (offset, len) in [(0, 0), (1, size - 1), (size / 2, size - size / 2), (size, 0)] {
                assert_eq!(
                    bundle.read(node, offset as u64, len).unwrap(),
                    all[from + offset..from + offset + len],
                    "{} {offset}+{len}",
                    node.path
                );
            }
            assert!(bundle.read(node, 1, size).is_err());
        }
        assert!(bundle.node("Resources/second").unwrap().flags == 0);
        assert!(bundle.node("third").unwrap().is_serialized());
        assert!(bundle.node("missing").is_none());
    }

    /// A stored block table, and no padding before the data, read as well.
    #[test]
    fn a_stored_table_and_unpadded_data_are_read() {
        let a: Vec<u8> = (0..40u8).collect();
        let bytes = build(&[(&a, 3)], &[(5, 20, 4, "only")], 0, 0x40);
        let bundle = open(bytes).unwrap();
        assert_eq!(
            bundle.read_node(bundle.node("only").unwrap()).unwrap(),
            (5..25u8).collect::<Vec<_>>()
        );
    }

    /// LZMA blocks and an LZMA table, as Unity writes them, read to the same bytes as the stored ones; a block whose
    /// LZMA makes fewer bytes than the table says is refused.
    #[test]
    fn lzma_tables_and_blocks_are_read() {
        let a: Vec<u8> = (0..40u8).collect();
        let b: Vec<u8> = b"Unity's LZMA has no end marker. ".repeat(9);
        let all: Vec<u8> = a.iter().chain(&b).copied().collect();
        let nodes = [(0, 30, 4, "first"), (30, 298, 0, "second.resS")];
        let bundle = open(build(&[(&a, 1), (&b, 1)], &nodes, 1, 0x241)).unwrap();
        assert_eq!(bundle.blocks().nth(1).unwrap().flags, 1);
        for node in bundle.nodes() {
            let (from, size) = (node.offset as usize, node.size as usize);
            assert_eq!(
                bundle.read_node(node).unwrap(),
                all[from..from + size],
                "{}",
                node.path
            );
        }
        let mut short = build(&[(&a, 3), (&b, 1)], &nodes, 0, 0x240);
        // the second block's uncompressed size: after the hash, the count and the first block's ten bytes
        let table_at = short.windows(16).position(|w| w == [7; 16]).unwrap();
        let size_at = table_at + 16 + 4 + 10;
        short[size_at..size_at + 4].copy_from_slice(&(b.len() as u32 + 1).to_be_bytes());
        // the node fits the claimed size; the read finds the LZMA one byte short
        let bundle = open(short).unwrap();
        let e = bundle
            .read_node(bundle.node("second.resS").unwrap())
            .unwrap_err()
            .to_string();
        assert!(e.contains("LZMA"), "{e}");
    }

    /// Formats 6 and 7 and a block table at the end of the file read the bytes format 8 with the table first does,
    /// with and without padding before the blocks; a format 6 header is not aligned, so the same bundle read as
    /// format 7 is refused.
    #[test]
    fn older_formats_and_a_table_at_the_end_are_read() {
        let a: Vec<u8> = (0..40u8).collect();
        let b: Vec<u8> = (100..130u8).collect();
        let blocks: [(&[u8], u32); 2] = [(&a, 3), (&b, 1)];
        let nodes = [(0, 25, 4, "first"), (25, 45, 0, "second")];
        let all: Vec<u8> = a.iter().chain(&b).copied().collect();
        for format in 6..=8 {
            for flags in [0x43, 0x243, 0xc3, 0x2c1, 0xc0] {
                let bytes = build_format(format, &blocks, &nodes, flags & 0x3f, flags);
                let bundle = open(bytes).unwrap_or_else(|e| panic!("format {format} flags {flags:#x}: {e}"));
                assert_eq!(bundle.header().format_version, format);
                for node in bundle.nodes() {
                    let (from, size) = (node.offset as usize, node.size as usize);
                    assert_eq!(
                        bundle.read_node(node).unwrap(),
                        all[from..from + size],
                        "format {format} flags {flags:#x} {}",
                        node.path
                    );
                }
            }
        }
        // "UnityFS\0", the version, "5.x.x\0", "6000.3.15f1\0" and 20 bytes of sizes and flags end at byte 50
        let mut six = build_format(6, &blocks, &nodes, 0, 0x40);
        six[11] = 7;
        assert!(open(six).is_err());
    }

    /// What the reader does not read is refused with the reason, and a damaged bundle with what is wrong.
    #[test]
    fn refusals_name_their_reason() {
        let good = sample();
        let with_flags = |flags: u32| {
            let a: Vec<u8> = (0..40u8).collect();
            open(build(&[(&a, 3)], &[(0, 10, 4, "n")], 3, flags))
        };
        for (flags, needle) in [
            (0x244, "LZHAM"),
            (0x203, "not stored with the directory"),
            (0x1243, "header flags"),
        ] {
            let e = with_flags(flags).unwrap_err();
            assert!(matches!(e, Error::Unsupported(_)), "{flags:#x}: {e}");
            assert!(e.to_string().contains(needle), "{flags:#x}: {e}");
        }
        for format in [5, 9] {
            let mut other = good.clone();
            other[11] = format;
            let e = open(other).unwrap_err();
            assert!(
                matches!(e, Error::Unsupported(_))
                    && e.to_string().contains(&format!("format version {format}")),
                "{e}"
            );
        }
        let a: Vec<u8> = (0..40u8).collect();
        let lzham = open(build(&[(&a, 4)], &[(0, 10, 4, "n")], 0, 0x240)).unwrap_err();
        assert!(
            lzham.to_string().contains("block 0 compressed with LZHAM"),
            "{lzham}"
        );
        // a table at the end that claims more bytes than follow the header
        let mut long_table = build(&[(&a, 0)], &[(0, 10, 4, "n")], 0, 0xc0);
        // the table's compressed size follows "UnityFS\0", the version, the two version strings and the bundle size
        let compressed_at = 8 + 4 + 6 + 12 + 8;
        long_table[compressed_at..compressed_at + 4].copy_from_slice(&1000u32.to_be_bytes());
        let e = open(long_table).unwrap_err().to_string();
        assert!(e.contains("does not fit"), "{e}");
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(open(magic).unwrap_err().to_string().contains("signature"));
        let mut cut = good.clone();
        cut.pop();
        let e = open(cut).unwrap_err().to_string();
        assert!(e.contains("the header says"), "{e}");
        let mut extra = good.clone();
        extra.push(0);
        assert!(open(extra).unwrap_err().to_string().contains("the header says"));
        let a: Vec<u8> = (0..40u8).collect();
        let outside = open(build(&[(&a, 3)], &[(30, 20, 4, "n")], 3, 0x243))
            .unwrap_err()
            .to_string();
        assert!(outside.contains("outside the blocks"), "{outside}");
        let twice = open(build(&[(&a, 3)], &[(0, 1, 4, "n"), (1, 1, 4, "n")], 3, 0x243))
            .unwrap_err()
            .to_string();
        assert!(twice.contains("twice"), "{twice}");
        let negative = open(build(&[(&a, 3)], &[(-1, 1, 4, "n")], 3, 0x243))
            .unwrap_err()
            .to_string();
        assert!(negative.contains("offset -1"), "{negative}");
    }

    /// A block that claims more bytes than its LZ4 can make is refused before anything is allocated for it.
    #[test]
    fn a_block_cannot_claim_more_than_its_lz4_can_make() {
        let a: Vec<u8> = (0..40u8).collect();
        let mut bytes = build(&[(&a, 3)], &[(0, 10, 4, "n")], 0, 0x240);
        // the block's uncompressed size is the u32 after the 16-byte hash and the block count in the stored table
        let table_at = bytes.windows(16).position(|w| w == [7; 16]).unwrap();
        let size_at = table_at + 16 + 4;
        bytes[size_at..size_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let bundle = open(bytes).unwrap();
        let e = bundle
            .read(bundle.node("n").unwrap(), 0, 1)
            .unwrap_err()
            .to_string();
        assert!(e.contains("lz4") || e.contains("LZ4"), "{e}");
    }
}
