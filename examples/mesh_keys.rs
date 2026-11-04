//! Lists the keys of the vertex and index bytes of every Mesh of a Unity player, so a frame captured from the running
//! game can be tied to the meshes its buffers came from.
//!
//! ```text
//! cargo run --release --example mesh_keys -- <the game's *_Data folder> <a type tree package (.tpk)> > keys.tsv
//! ```
//!
//! Each line written is `key<TAB>bytes<TAB>name`: the key is XXH3-128 as 32 lowercase hex digits (the key locum names
//! captured bytes by; `locum-cli index --keys keys.tsv` reads the list), and the name is `<file>/<mesh>@<path id>` and
//! what the bytes are: `#vb<n>` (vertex stream n, the vertices of one vertex buffer), `#vertices` (the whole vertex
//! data, written when it is not one stream's bytes) or `#faces` (the index buffer of all submeshes). Compressed meshes
//! have no stored vertex streams and give only their indices.

use std::path::Path;

use unity_formats::listing::list;
use unity_formats::player::Player;
use unity_formats::resource::StreamingInfo;
use unity_formats::value::Array;
use unity_formats::{TypeTreePackage, Value};
use xxhash_rust::xxh3::xxh3_128;

/// The size of one component of a vertex channel's `format` (Unity 2019 on's VertexFormat: Float, Float16, UNorm8,
/// SNorm8, UNorm16, SNorm16, UInt8, SInt8, UInt16, SInt16, UInt32, SInt32). Earlier versions number the formats
/// otherwise; a mesh whose channels do not lay out its bytes is reported and keyed whole.
fn component_size(format: i64) -> Option<usize> {
    [4, 2, 1, 1, 2, 2, 1, 1, 2, 2, 4, 4]
        .get(usize::try_from(format).ok()?)
        .copied()
}

/// Each stream's (offset, length) in the vertex data, as Unity lays them out: a stream's stride is the sum of its
/// channels' sizes and each stream starts 16-byte aligned.
fn streams(vd: &Value, vertices: usize) -> Option<Vec<(usize, usize)>> {
    let Some(Value::Array(Array::Values(chans))) = vd.get("m_Channels") else {
        return None;
    };
    let mut strides = [0usize; 4];
    for c in chans {
        let dim = usize::try_from(c.get("dimension")?.as_i64()? & 0xF).ok()?;
        if dim == 0 {
            continue;
        }
        let stream = usize::try_from(c.get("stream")?.as_i64()?).ok()?;
        let size = component_size(c.get("format")?.as_i64()?)?;
        *strides.get_mut(stream)? += dim * size;
    }
    let used = strides.iter().rposition(|&s| s > 0).map_or(0, |s| s + 1);
    let mut out = Vec::with_capacity(used);
    let mut offset = 0usize;
    for &stride in &strides[..used] {
        let len = vertices.checked_mul(stride)?;
        out.push((offset, len));
        offset = offset.checked_add(len)?.checked_next_multiple_of(16)?;
    }
    Some(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(data), Some(package)) = (args.next(), args.next()) else {
        eprintln!("usage: mesh_keys <the game's *_Data folder> <type tree package (.tpk)> > keys.tsv");
        return Ok(());
    };
    let package = TypeTreePackage::parse(&std::fs::read(package)?)?;
    let player = Player::open(Path::new(&data), package)?;
    let meshes = list(&player, Some(&["Mesh"]))?;
    let (mut written, mut failed, mut unlaid) = (0usize, 0usize, 0usize);
    let mut line = |bytes: &[u8], name: String| {
        if !bytes.is_empty() {
            println!("{:032x}\t{}\t{name}", xxh3_128(bytes), bytes.len());
            written += 1;
        }
    };
    for entry in &meshes.entries {
        let r = &entry.object;
        let m = match player.decode(r) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("{}/{}: {e}", r.file, r.path_id);
                failed += 1;
                continue;
            }
        };
        let name = format!(
            "{}/{}@{}",
            r.file,
            m.get("m_Name").and_then(Value::as_str).unwrap_or(""),
            r.path_id
        );
        if let Some(ib) = m.get("m_IndexBuffer").and_then(Value::as_bytes) {
            line(ib, format!("{name}#faces"));
        }
        let Some(vd) = m.get("m_VertexData") else {
            continue;
        };
        let streamed = m
            .get("m_StreamData")
            .map(StreamingInfo::of)
            .transpose()?
            .flatten();
        let bytes = match streamed {
            Some(s) => match player.read_stream(&r.file, &s) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("{name}: {e}");
                    failed += 1;
                    continue;
                }
            },
            None => vd
                .get("m_DataSize")
                .and_then(Value::as_bytes)
                .unwrap_or_default()
                .to_vec(),
        };
        if bytes.is_empty() {
            continue;
        }
        let vertices = vd
            .get("m_VertexCount")
            .and_then(Value::as_i64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        // the layout is right when its streams end where the data does (the last one padded to 16 bytes at most)
        let layout = streams(vd, vertices).filter(|s| {
            s.last()
                .is_some_and(|&(o, l)| o + l <= bytes.len() && bytes.len() - (o + l) < 16)
        });
        let Some(layout) = layout else {
            eprintln!(
                "{name}: its channels do not lay out its {} bytes of vertex data",
                bytes.len()
            );
            unlaid += 1;
            line(&bytes, format!("{name}#vertices"));
            continue;
        };
        let mut whole_is_a_stream = false;
        for (s, &(o, l)) in layout.iter().enumerate() {
            whole_is_a_stream |= o == 0 && l == bytes.len();
            line(&bytes[o..o + l], format!("{name}#vb{s}"));
        }
        if !whole_is_a_stream {
            line(&bytes, format!("{name}#vertices"));
        }
    }
    eprintln!(
        "{} meshes: {written} keys written; {failed} could not be read; {unlaid} whose streams could not be laid out",
        meshes.entries.len()
    );
    Ok(())
}
