//! Unity meshes: vertex channels in interleaved streams (decoded through checked vertex layouts), or a compressed
//! mesh (bit-packed quantised floats); an index buffer of 16- or 32-bit indices; submeshes.
//!
//! Vertex data layout (Unity 5 and later): the channels name a stream, an offset in it, a format and a dimension; a
//! stream's stride is the sum of its channels' sizes, and the streams follow each other, each starting 16-byte
//! aligned. The bytes are in the mesh, or streamed from a resource file (`m_StreamData`).
//!
//! A compressed mesh's floats are `PackedBitVector`s: `n` items of `bits` bits each, little-endian bit order, each
//! `start + item * (range / (2^bits - 1))` in f32; normals and tangents keep x and y, z is rebuilt from the unit
//! length with a stored sign; its indices are packed integers (`m_Triangles`).

use half::f16;
use primase_foundations::vertex::{
    Element, Format, Stream as VertexStream, decode_stream_raw, resolve_layout,
};

use crate::player::{ObjRef, Player, SkinWeights};
use crate::resource::StreamingInfo;
use crate::value::{Array, Value};
use crate::{Error, Result, invalid};

/// Unity's shader channels, in the order of `m_Channels`.
pub const CHANNELS: [&str; 14] = [
    "Vertex",
    "Normal",
    "Tangent",
    "Color",
    "TexCoord0",
    "TexCoord1",
    "TexCoord2",
    "TexCoord3",
    "TexCoord4",
    "TexCoord5",
    "TexCoord6",
    "TexCoord7",
    "BlendWeight",
    "BlendIndices",
];

/// One decoded channel: `dim` components per vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    /// Components per vertex.
    pub dim: usize,
    /// `dim` floats for each vertex, vertex after vertex.
    pub values: Vec<f32>,
}

impl Channel {
    /// Vertex `i`'s components; `None` when `i` is not a vertex of the channel.
    pub fn vertex(&self, i: usize) -> Option<&[f32]> {
        let start = i.checked_mul(self.dim)?;
        self.values.get(start..start.checked_add(self.dim)?)
    }
}

/// A submesh: a run of the index buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubMesh {
    /// Into the index buffer, in bytes.
    pub first_byte: u32,
    /// How many indices the run holds.
    pub index_count: u32,
    /// Unity's MeshTopology (0: triangles).
    pub topology: i64,
    /// `baseVertex` as stored: the vertex the run's indices count from.
    pub base_vertex: u32,
}

/// Bone influences per vertex, as a mesh stores them: vertex `v`'s weights and bone slots are
/// `weights[starts[v]..starts[v + 1]]` and `bones[..]`, heaviest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blend {
    /// Where each vertex's influences start in `weights` and `bones`: one entry more than there are vertices, the
    /// last the total.
    pub starts: Vec<u32>,
    /// The influences' weights, vertex after vertex.
    pub weights: Vec<f32>,
    /// The influences' bone slots, parallel to `weights`.
    pub bones: Vec<u32>,
}

impl Blend {
    /// `influences` (at most four) to a vertex from lists padded to four with zero weights.
    pub fn from_padded(influences: usize, weights: &[[f32; 4]], bones: &[[u32; 4]]) -> Blend {
        let influences = influences.min(4);
        Blend {
            starts: (0..=weights.len())
                .map(|v| u32::try_from(v.saturating_mul(influences)).unwrap_or(u32::MAX))
                .collect(),
            weights: weights
                .iter()
                .flat_map(|w| w.iter().take(influences).copied())
                .collect(),
            bones: bones
                .iter()
                .flat_map(|b| b.iter().take(influences).copied())
                .collect(),
        }
    }

    /// How many vertices the blend covers.
    pub fn vertices(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    /// Vertex `v`'s weights and bone slots; `None` when `v` is not a vertex of the blend or its range is not inside
    /// the lists.
    pub fn vertex(&self, v: usize) -> Option<(&[f32], &[u32])> {
        let start = usize::try_from(*self.starts.get(v)?).ok()?;
        let end = usize::try_from(*self.starts.get(v.checked_add(1)?)?).ok()?;
        Some((self.weights.get(start..end)?, self.bones.get(start..end)?))
    }

    /// The most influences any vertex has.
    pub fn max_influences(&self) -> usize {
        self.starts
            .iter()
            .zip(self.starts.iter().skip(1))
            .map(|(&a, &b)| usize::try_from(b.saturating_sub(a)).unwrap_or(usize::MAX))
            .max()
            .unwrap_or(0)
    }
}

/// A skin's influences as sets of four per vertex (`[set][vertex]`): the weights, the bone slots and the number of
/// vertices cut to a limit ([`MeshSkin::padded`]).
pub type PaddedSets = (Vec<Vec<[f32; 4]>>, Vec<Vec<[u32; 4]>>, u64);

/// A mesh's skin as stored, in Unity's frame: the blend channels (`BlendWeight` and `BlendIndices`: up to four
/// influences a vertex, as Unity draws them at four or fewer), the full influences of a mesh that stores them
/// (`m_VariableBoneCountWeights`, which Unity draws only at unlimited skin weights; the channels then hold its first
/// four renormalised), and per bone the bind pose, the matrix taking the renderer's local space to the bone's at bind
/// time.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSkin {
    /// Bone slots are slots of the renderer's bone list (`m_Bones`); a mesh with indices but no weights channel binds
    /// each vertex to its one bone wholly.
    pub channel: Blend,
    /// The full influences of a mesh that stores more than four a vertex (`m_VariableBoneCountWeights`); `None` for a
    /// mesh that does not.
    pub unlimited: Option<Blend>,
    /// `m_BindPose`, row-major (`[row][column]`, the translation in column 3).
    pub bind_poses: Vec<[[f64; 4]; 4]>,
}

impl MeshSkin {
    /// The blend `limit` draws and the most influences a vertex keeps.
    fn drawn(&self, limit: SkinWeights) -> (&Blend, usize) {
        match limit {
            SkinWeights::Unlimited => (self.unlimited.as_ref().unwrap_or(&self.channel), usize::MAX),
            SkinWeights::Bones(n) => (&self.channel, usize::try_from(n).unwrap_or(usize::MAX)),
        }
    }

    /// What `limit` makes of this skin: whether the full influences are drawn, and the most influences a vertex keeps.
    /// Two limits with the same outcome draw the same mesh.
    pub fn outcome(&self, limit: SkinWeights) -> (bool, usize) {
        let (blend, keep) = self.drawn(limit);
        (
            std::ptr::eq(blend, &self.channel),
            blend.max_influences().min(keep),
        )
    }

    /// How many sets of four influences a vertex has under `limit`: at least one.
    pub fn sets(&self, limit: SkinWeights) -> usize {
        self.outcome(limit).1.div_ceil(4).max(1)
    }

    /// The influences `limit` draws as sets of four per vertex (weights and bone slots, zero-padded); a vertex with
    /// more influences than the limit keeps its first ones renormalised to sum to 1. Also the vertices cut so.
    pub fn padded(&self, limit: SkinWeights) -> PaddedSets {
        let (blend, keep) = self.drawn(limit);
        let n = blend.vertices();
        let sets = self.sets(limit);
        let mut weights = vec![vec![[0.0f32; 4]; n]; sets];
        let mut bones = vec![vec![[0u32; 4]; n]; sets];
        let mut cut = 0u64;
        for v in 0..n {
            let Some((w, b)) = blend.vertex(v) else { continue };
            let kept = w.len().min(keep);
            let divisor = if kept < w.len() {
                cut = cut.saturating_add(1);
                Some(w.iter().take(kept).sum::<f32>()).filter(|sum| *sum > 0.0)
            } else {
                None
            };
            for (i, (&weight, &bone)) in w.iter().zip(b).take(kept).enumerate() {
                let (set, slot) = (i.div_euclid(4), i.rem_euclid(4));
                if let Some(x) = weights
                    .get_mut(set)
                    .and_then(|s| s.get_mut(v))
                    .and_then(|q| q.get_mut(slot))
                {
                    *x = divisor.map_or(weight, |sum| weight / sum);
                }
                if let Some(x) = bones
                    .get_mut(set)
                    .and_then(|s| s.get_mut(v))
                    .and_then(|q| q.get_mut(slot))
                {
                    *x = bone;
                }
            }
        }
        (weights, bones, cut)
    }
}

/// The blend channels of an uncompressed mesh: the influences stored a vertex and each vertex's weights and bone
/// slots, padded to four.
type BlendChannels = (usize, Vec<[f32; 4]>, Vec<[u32; 4]>);

/// A mesh as stored, in Unity's frame.
#[derive(Clone, Debug, PartialEq)]
pub struct UnityMeshData {
    /// `m_Name`.
    pub name: String,
    /// How many vertices the mesh has.
    pub vertex_count: usize,
    /// By shader channel (see [`CHANNELS`]); the blend weights and indices are in [`UnityMeshData::skin`].
    pub channels: [Option<Channel>; 12],
    /// The mesh's skin: its blend weights and indices with the bind poses; `None` for a mesh without blend data.
    pub skin: Option<MeshSkin>,
    /// Whether the vertices were stored as a `CompressedMesh` (bit-packed) rather than as vertex streams.
    pub compressed: bool,
    /// The mesh's blend shapes; `None` for a mesh without any.
    pub shapes: Option<MeshShapes>,
    /// The index buffer, decoded.
    pub indices: Vec<u32>,
    /// The width in bytes of an index in the stored index buffer (2 or 4), which a submesh's `first_byte` counts in;
    /// `indices` holds them widened to `u32`.
    pub index_size: u32,
    /// The submeshes, in order.
    pub submeshes: Vec<SubMesh>,
}

/// One blend shape channel (`m_Shapes.channels`): a name, the CRC-32 of it that an animation curve names it by, and its
/// frames (`frames` of them from `first_frame` in [`MeshShapes::frames`]).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeChannel {
    /// The channel's name.
    pub name: String,
    /// The channel name's CRC-32, which an animation curve names the channel by.
    pub name_hash: u32,
    /// Index of the channel's first frame in [`MeshShapes::frames`].
    pub first_frame: usize,
    /// How many frames the channel has.
    pub frames: usize,
}

/// One frame of a blend shape (`m_Shapes.shapes`, its `fullWeights` entry): the weight it is at, its entries
/// (`count` from `first` in [`MeshShapes::vertices`]) and whether they carry normal and tangent deltas.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeFrame {
    /// The weight the frame is at (`fullWeights`).
    pub weight: f32,
    /// Index of the frame's first entry in [`MeshShapes::vertices`].
    pub first: usize,
    /// How many entries the frame has.
    pub count: usize,
    /// Whether the entries carry normal deltas (`hasNormals`).
    pub normals: bool,
    /// Whether the entries carry tangent deltas (`hasTangents`).
    pub tangents: bool,
}

/// A blend shape entry (`m_Shapes.vertices`): the delta of vertex `index` of the mesh, in Unity's frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeVertex {
    /// The vertex of the mesh the delta applies to.
    pub index: u32,
    /// The position delta.
    pub position: [f32; 3],
    /// The normal delta (zero unless the frame has normals).
    pub normal: [f32; 3],
    /// The tangent delta (zero unless the frame has tangents).
    pub tangent: [f32; 3],
}

/// A mesh's blend shapes as stored (`m_Shapes`): per channel its frames in increasing weight (Unity's weights run 0 to
/// 100), per frame sparse deltas of the mesh's vertices, applied to the mesh as stored before any skin.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshShapes {
    /// The channels (`m_Shapes.channels`).
    pub channels: Vec<ShapeChannel>,
    /// Every channel's frames, one after another (`m_Shapes.shapes` with `fullWeights`).
    pub frames: Vec<ShapeFrame>,
    /// The frames' entries, one frame after another (`m_Shapes.vertices`).
    pub vertices: Vec<ShapeVertex>,
}

fn vec3(v: &Value, k: &str, what: &str) -> Result<[f32; 3]> {
    let c = v.get(k).ok_or_else(|| invalid(format!("{what}: no {k}")))?;
    let f = |a: &str| {
        c.get(a)
            .and_then(Value::as_f32)
            .ok_or_else(|| invalid(format!("{what}: {k} without {a}")))
    };
    Ok([f("x")?, f("y")?, f("z")?])
}

/// The blend shapes of a mesh (`m_Shapes`) with `vertex_count` vertices; `None` where it has no channel.
fn read_shapes(m: &Value, name: &str, vertex_count: usize) -> Result<Option<MeshShapes>> {
    let Some(s) = m.get("m_Shapes") else {
        return Ok(None);
    };
    let list = |k: &str| -> Result<&[Value]> {
        match s.get(k) {
            Some(Value::Array(Array::Values(l))) => Ok(l.as_slice()),
            Some(Value::Array(a)) if a.is_empty() => Ok(&[]),
            _ => Err(invalid(format!("{name}: no m_Shapes.{k}"))),
        }
    };
    let channels = list("channels")?;
    if channels.is_empty() {
        return Ok(None);
    }
    let full_weights: &[f32] = match s.get("fullWeights") {
        Some(Value::Array(Array::F32(w))) => w,
        Some(Value::Array(a)) if a.is_empty() => &[],
        _ => return Err(invalid(format!("{name}: no m_Shapes.fullWeights"))),
    };
    let stored = list("vertices")?;
    let shapes = list("shapes")?;
    if full_weights.len() != shapes.len() {
        return Err(invalid(format!(
            "{name}: {} frame weights for {} blend shape frames",
            full_weights.len(),
            shapes.len()
        )));
    }
    let what = format!("{name}: blend shapes");
    let vertices = stored
        .iter()
        .map(|v| {
            let index = int(v, "index")?;
            let Some(slot) = usize::try_from(index).ok().filter(|&i| i < vertex_count) else {
                return Err(invalid(format!("{what}: vertex {index} of {vertex_count}")));
            };
            let entry = ShapeVertex {
                index: u32::try_from(slot)
                    .map_err(|_| invalid(format!("{what}: vertex {index} of {vertex_count}")))?,
                position: vec3(v, "vertex", &what)?,
                normal: vec3(v, "normal", &what)?,
                tangent: vec3(v, "tangent", &what)?,
            };
            if [entry.position, entry.normal, entry.tangent]
                .iter()
                .flatten()
                .any(|x| !x.is_finite())
            {
                return Err(Error::Unsupported(format!(
                    "{what}: a delta that is not finite (vertex {index})"
                )));
            }
            Ok(entry)
        })
        .collect::<Result<Vec<_>>>()?;
    let frames = shapes
        .iter()
        .zip(full_weights)
        .map(|(f, &weight)| {
            let range = run(
                int(f, "firstVertex")?,
                int(f, "vertexCount")?,
                vertices.len(),
                &what,
            )?;
            let flag = |k: &str| {
                f.get(k)
                    .and_then(Value::as_bool)
                    .ok_or_else(|| invalid(format!("{what}: no {k}")))
            };
            Ok(ShapeFrame {
                weight,
                first: range.start,
                count: range.len(),
                normals: flag("hasNormals")?,
                tangents: flag("hasTangents")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let channels = channels
        .iter()
        .map(|c| {
            let range = run(int(c, "frameIndex")?, int(c, "frameCount")?, frames.len(), &what)?;
            let own = frames
                .get(range.clone())
                .filter(|own| !own.is_empty())
                .ok_or_else(|| invalid(format!("{what}: a channel of {} frames", range.len())))?;
            if !own
                .iter()
                .zip(own.iter().skip(1))
                .all(|(a, b)| a.weight < b.weight)
                || own.iter().any(|f| !f.weight.is_finite() || f.weight == 0.0)
            {
                return Err(Error::Unsupported(format!(
                    "{what}: frame weights not increasing or zero"
                )));
            }
            Ok(ShapeChannel {
                name: c.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                name_hash: uint(c, "nameHash", &what)?,
                first_frame: range.start,
                frames: range.len(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(MeshShapes {
        channels,
        frames,
        vertices,
    }))
}

fn int(v: &Value, k: &str) -> Result<i64> {
    v.get(k)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("mesh: no integer {k}")))
}

/// The integer field `k` of `v` as a `u32`: a stored value outside 0..=2^32-1 is refused, not wrapped.
fn uint(v: &Value, k: &str, what: &str) -> Result<u32> {
    let n = int(v, k)?;
    u32::try_from(n).map_err(|_| invalid(format!("{what}: {k} {n} is not a 32-bit unsigned number")))
}

/// The run `first..first + count` of a list of `len` entries, from stored integers: negative numbers, a sum that
/// overflows and a run past the list are refused.
fn run(first: i64, count: i64, len: usize, what: &str) -> Result<std::ops::Range<usize>> {
    first
        .checked_add(count)
        .and_then(|end| Some(usize::try_from(first).ok()?..usize::try_from(end).ok()?))
        .filter(|r| r.end <= len && count >= 0)
        .ok_or_else(|| invalid(format!("{what}: entries {first}+{count} of {len}")))
}

fn bytes<'a>(v: &'a Value, k: &str) -> Result<&'a [u8]> {
    match v.get(k) {
        Some(Value::Array(Array::U8(b))) => Ok(b),
        _ => Err(invalid(format!("mesh: no bytes {k}"))),
    }
}

/// Unity's VertexFormat component size.
fn component_size(format: i64) -> Result<usize> {
    Ok(match format {
        0 | 10 | 11 => 4,
        1 | 4 | 5 | 8 | 9 => 2,
        2 | 3 | 6 | 7 => 1,
        f => return Err(invalid(format!("vertex format {f}"))),
    })
}

/// A Unity (format, dimension) as a vertex layout format.
fn vertex_format(format: i64, dim: usize) -> Option<Format> {
    Some(match (format, dim) {
        (0, 2) => Format::Float2,
        (0, 3) => Format::Float3,
        (0, 4) => Format::Float4,
        (1, 2) => Format::Half2,
        (1, 4) => Format::Half4,
        (2, 4) => Format::Unorm8x4,
        (4, 2) => Format::Unorm16x2,
        (5, 2) => Format::Snorm16x2,
        (5, 4) => Format::Snorm16x4,
        _ => return None,
    })
}

/// A PackedBitVector's items as integers.
fn unpack_ints(v: &Value, start: usize, count: usize) -> Result<Vec<u32>> {
    let bits = int(v, "m_BitSize")?;
    let bits = usize::try_from(bits)
        .ok()
        .filter(|&b| b <= 32)
        .ok_or_else(|| invalid(format!("packed bit size {bits}")))?;
    let data = bytes(v, "m_Data")?;
    if bits == 0 {
        if count > MAX_PACKED_ITEMS {
            return Err(invalid(format!(
                "{count} packed items of no bits (at most {MAX_PACKED_ITEMS})"
            )));
        }
        return Ok(vec![0; count]);
    }
    let end_bit = start
        .checked_add(count)
        .and_then(|n| n.checked_mul(bits))
        .ok_or_else(|| invalid(format!("packed items {start}+{count} of {bits} bits")))?;
    if end_bit.div_ceil(8) > data.len() {
        return Err(invalid(format!(
            "packed items {start}+{count} of {bits} bits past {} bytes",
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(count);
    // an item starts at `bit` and, with its offset in the first byte, spans at most five bytes
    let mask = shl(1, bits).saturating_sub(1);
    let mut bit = start.saturating_mul(bits);
    for _ in 0..count {
        let (first, offset) = (bit / 8, bit % 8);
        let span = offset.saturating_add(bits).div_ceil(8);
        let window = first
            .checked_add(span)
            .and_then(|end| data.get(first..end))
            .ok_or_else(|| {
                invalid(format!(
                    "packed item at bit {bit} of {bits} bits past {} bytes",
                    data.len()
                ))
            })?;
        let word = window
            .iter()
            .enumerate()
            .fold(0u64, |w, (i, &b)| w | shl(u64::from(b), i.saturating_mul(8)));
        let value = shr(word, offset) & mask;
        out.push(u32::try_from(value).unwrap_or(u32::MAX));
        bit = bit.saturating_add(bits);
    }
    Ok(out)
}

/// `x << n`, 0 when `n` shifts every bit out.
fn shl(x: u64, n: usize) -> u64 {
    u32::try_from(n).ok().and_then(|n| x.checked_shl(n)).unwrap_or(0)
}

/// `x >> n`, 0 when `n` shifts every bit out.
fn shr(x: u64, n: usize) -> u64 {
    u32::try_from(n).ok().and_then(|n| x.checked_shr(n)).unwrap_or(0)
}

/// A PackedBitVector's items as floats: `start + item * (range / (2^bits - 1))`, in f32.
fn unpack_floats(v: &Value, start: usize, count: usize) -> Result<Vec<f32>> {
    let bits = int(v, "m_BitSize")?;
    let bits = u32::try_from(bits)
        .ok()
        .filter(|&b| b <= 32)
        .ok_or_else(|| invalid(format!("packed bit size {bits}")))?;
    let range = v
        .get("m_Range")
        .and_then(Value::as_f32)
        .ok_or_else(|| invalid("packed floats: no m_Range"))?;
    let first = v
        .get("m_Start")
        .and_then(Value::as_f32)
        .ok_or_else(|| invalid("packed floats: no m_Start"))?;
    if bits == 0 {
        if count > MAX_PACKED_ITEMS {
            return Err(invalid(format!(
                "{count} packed items of no bits (at most {MAX_PACKED_ITEMS})"
            )));
        }
        return Ok(vec![first; count]);
    }
    let scale = range / shl(1, usize::try_from(bits).unwrap_or(0)).saturating_sub(1) as f32;
    Ok(unpack_ints(v, start, count)?
        .into_iter()
        .map(|x| x as f32 * scale + first)
        .collect())
}

/// Channel `i` (0 to 11) of a mesh set to `channel`.
fn set_channel(channels: &mut [Option<Channel>; 12], i: usize, channel: Channel) -> Result<()> {
    *channels
        .get_mut(i)
        .ok_or_else(|| invalid(format!("mesh: channel {i} of 12")))? = Some(channel);
    Ok(())
}

fn items(v: &Value) -> Result<usize> {
    let n = int(v, "m_NumItems")?;
    usize::try_from(n).map_err(|_| invalid(format!("mesh: m_NumItems of {n}")))
}

/// The most items one packed vector may state when it needs no bytes (a bit size of 0 stores none): 2^26, which is 4
/// floats of each of 16 million vertices. The counts of vectors that do store bits are bounded by their bytes.
const MAX_PACKED_ITEMS: usize = 1 << 26;

fn check_vertices(name: &str, n: usize) -> Result<()> {
    if n > MAX_VERTICES {
        return Err(Error::Unsupported(format!(
            "{name}: {n} vertices (at most {MAX_VERTICES})"
        )));
    }
    Ok(())
}

/// The most vertices a mesh may state. A mesh of this many vertices has 16 million positions at least 12 bytes each;
/// the bound keeps a state of 2^31 from allocating a vertex-sized array before any data is looked at.
const MAX_VERTICES: usize = 1 << 24;

/// x, y and a stored z sign to a unit vector: z = sqrt(1 - x^2 - y^2), or (x, y, 0) normalised when that is
/// negative.
fn rebuild_z(x: f32, y: f32, positive: bool) -> [f32; 3] {
    let zsq = 1.0 - x * x - y * y;
    let [x, y, z] = if zsq >= 0.0 {
        [x, y, zsq.sqrt()]
    } else {
        let l = (x * x + y * y).sqrt();
        [x / l, y / l, 0.0]
    };
    [x, y, if positive { z } else { -z }]
}

/// A mesh's `m_VariableBoneCountWeights` data for `n` vertices: `n + 1` start offsets (word indexes into the data, the
/// first being `n + 1`, the last the data's length), then each vertex's influences as words, the bone slot in the low
/// 16 bits and the weight in the high 16 (normalised), heaviest first.
fn read_variable_weights(name: &str, data: &[u32], n: usize) -> Result<Blend> {
    let bad = |why: String| invalid(format!("{name}: variable bone weights: {why}"));
    let table = n.checked_add(1).ok_or_else(|| bad(format!("{n} vertices")))?;
    let Some(starts) = data.get(..table) else {
        return Err(bad(format!("{} words for {n} vertices", data.len())));
    };
    let (Some(&first), Some(&last)) = (starts.first(), starts.last()) else {
        return Err(bad("no start offsets".into()));
    };
    if usize::try_from(first) != Ok(table)
        || usize::try_from(last) != Ok(data.len())
        || starts.iter().zip(starts.iter().skip(1)).any(|(a, b)| b <= a)
    {
        return Err(bad(
            "start offsets not increasing from the end of the table to the end of the data".into(),
        ));
    }
    let words = data.get(table..).unwrap_or_default();
    Ok(Blend {
        starts: starts.iter().map(|o| o.saturating_sub(first)).collect(),
        weights: words
            .iter()
            .map(|w| f32::from(u16::try_from(w >> 16).unwrap_or(u16::MAX)) / 65535.0)
            .collect(),
        bones: words.iter().map(|w| w & 0xFFFF).collect(),
    })
}

/// The bind poses of mesh `m` (`m_BindPose`, Matrix4x4f `eRC` fields), row-major.
fn bind_poses(m: &Value, name: &str) -> Result<Vec<[[f64; 4]; 4]>> {
    let list = match m.get("m_BindPose") {
        Some(Value::Array(Array::Values(l))) => l.as_slice(),
        Some(Value::Array(a)) if a.is_empty() => &[],
        _ => return Err(invalid(format!("{name}: no m_BindPose"))),
    };
    list.iter()
        .map(|p| {
            let mut out = [[0.0; 4]; 4];
            for (r, row) in out.iter_mut().enumerate() {
                for (c, x) in row.iter_mut().enumerate() {
                    let k = format!("e{r}{c}");
                    *x = f64::from(
                        p.get(&k)
                            .and_then(Value::as_f32)
                            .ok_or_else(|| invalid(format!("{name}: bind pose without {k}")))?,
                    );
                }
            }
            Ok(out)
        })
        .collect()
}

/// A blend weight component by Unity's `VertexFormat`: `Float` (0), `Float16` (1), `UNorm8` (2) or `UNorm16` (4).
fn blend_weight(bytes: &[u8], format: i64) -> Result<f32> {
    Ok(match (format, bytes) {
        (0, &[a, b, c, d, ..]) => f32::from_le_bytes([a, b, c, d]),
        (1, &[a, b, ..]) => f16::from_le_bytes([a, b]).to_f32(),
        (2, &[a, ..]) => f32::from(a) / 255.0,
        (0..=2, _) => return Err(invalid(format!("a blend weight of format {format} cut short"))),
        (_, &[a, b, ..]) => f32::from(u16::from_le_bytes([a, b])) / 65535.0,
        _ => return Err(invalid("a blend weight cut short")),
    })
}

/// The `BlendWeight` and `BlendIndices` channels (12 and 13) of an uncompressed mesh: the influences stored per
/// vertex and each vertex's weights and bone slots, padded to four with zeros. An indices channel without a weights
/// channel is one influence of weight 1 (a rigid bind); a weights channel without indices, or of another dimension,
/// is an error. `None` for a mesh without either channel. Indices are `UInt8` (6), `UInt16` (8) or `UInt32` (10).
fn read_skin_channels(
    name: &str,
    info: &[(usize, usize, i64, usize)],
    stream_at: &[(usize, usize)],
    data: &[u8],
    n: usize,
) -> Result<Option<BlendChannels>> {
    let channel = |ch: usize| info.get(ch).copied().filter(|c| c.3 > 0);
    let (w, i) = (channel(12), channel(13));
    let Some((is, io, ifmt, id)) = i else {
        if w.is_some() {
            return Err(invalid(format!("{name}: blend weights without blend indices")));
        }
        return Ok(None);
    };
    if id > 4 {
        return Err(Error::Unsupported(format!(
            "{name}: {id} bone influences a vertex"
        )));
    }
    if !matches!(ifmt, 6 | 8 | 10) {
        return Err(Error::Unsupported(format!(
            "{name}: blend indices of format {ifmt}"
        )));
    }
    if let Some((_, _, wfmt, wd)) = w {
        if wd != id {
            return Err(invalid(format!(
                "{name}: {wd} blend weights and {id} blend indices a vertex"
            )));
        }
        if !matches!(wfmt, 0 | 1 | 2 | 4) {
            return Err(Error::Unsupported(format!(
                "{name}: blend weights of format {wfmt}"
            )));
        }
    } else if id != 1 {
        return Err(invalid(format!(
            "{name}: {id} blend indices a vertex and no blend weights"
        )));
    }
    let at = |stream: usize, off: usize, row: usize, k: usize, size: usize| -> Result<&[u8]> {
        let past = || {
            invalid(format!(
                "{name}: blend data past the vertex data ({} bytes)",
                data.len()
            ))
        };
        let &(base, stride) = stream_at.get(stream).ok_or_else(past)?;
        let from = row
            .checked_mul(stride)
            .and_then(|r| {
                r.checked_add(base)?
                    .checked_add(off)?
                    .checked_add(k.checked_mul(size)?)
            })
            .ok_or_else(past)?;
        data.get(from..from.checked_add(size).ok_or_else(past)?)
            .ok_or_else(past)
    };
    let index_size = component_size(ifmt)?;
    let mut weights = vec![[0.0f32; 4]; n];
    let mut bones = vec![[0u32; 4]; n];
    for (v, (weight_row, bone_row)) in weights.iter_mut().zip(bones.iter_mut()).enumerate() {
        for k in 0..id {
            let cut = || invalid(format!("{name}: a blend index of vertex {v} cut short"));
            let bone = match (ifmt, at(is, io, v, k, index_size)?) {
                (6, &[a, ..]) => u32::from(a),
                (8, &[a, b, ..]) => u32::from(u16::from_le_bytes([a, b])),
                (_, &[a, b, c, d, ..]) => u32::from_le_bytes([a, b, c, d]),
                _ => return Err(cut()),
            };
            let weight = match w {
                Some((ws, wo, wfmt, _)) => blend_weight(at(ws, wo, v, k, component_size(wfmt)?)?, wfmt)?,
                None => 1.0,
            };
            *bone_row.get_mut(k).ok_or_else(cut)? = bone;
            *weight_row.get_mut(k).ok_or_else(cut)? = weight;
        }
    }
    Ok(Some((id, weights, bones)))
}

/// The submeshes of mesh `m` (`m_SubMeshes`), their stored numbers checked to fit the 32 bits they are kept in.
fn read_submeshes(m: &Value, name: &str) -> Result<Vec<SubMesh>> {
    let Some(Value::Array(Array::Values(list))) = m.get("m_SubMeshes") else {
        return Err(invalid(format!("{name}: no m_SubMeshes")));
    };
    list.iter()
        .enumerate()
        .map(|(i, s)| {
            let what = format!("{name}: submesh {i}");
            Ok(SubMesh {
                first_byte: uint(s, "firstByte", &what)?,
                index_count: uint(s, "indexCount", &what)?,
                topology: int(s, "topology")?,
                base_vertex: uint(s, "baseVertex", &what)?,
            })
        })
        .collect()
}

impl UnityMeshData {
    /// Decode the Mesh object `r` of `player` (its vertex data in the mesh or streamed beside its file).
    pub fn read(player: &Player, r: &ObjRef) -> Result<Self> {
        let m = player.decode(r)?;
        let name = m.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned();
        let vd = m
            .get("m_VertexData")
            .ok_or_else(|| invalid("mesh: no m_VertexData"))?;
        let cm = m
            .get("m_CompressedMesh")
            .ok_or_else(|| invalid("mesh: no m_CompressedMesh"))?;
        let compressed = items(cm.get("m_Vertices").ok_or_else(|| invalid("no m_Vertices"))?)? > 0;
        if compressed && cm.get("m_Weights").map(items).transpose()?.unwrap_or(0) > 0 {
            return Err(Error::Unsupported(format!(
                "{name}: compressed mesh skin weights"
            )));
        }
        let variable = match m.at(&["m_VariableBoneCountWeights", "m_Data"]) {
            Some(Value::Array(Array::U32(d))) if !d.is_empty() => Some(d.as_slice()),
            Some(Value::Array(a)) if !a.is_empty() => {
                return Err(Error::Unsupported(format!(
                    "{name}: variable bone weights not stored as 32-bit words"
                )));
            }
            _ => None,
        };
        let mut channels: [Option<Channel>; 12] = Default::default();
        let mut skin_channels = None;
        let vertex_count;
        if compressed {
            let get = |k: &str| {
                cm.get(k)
                    .ok_or_else(|| invalid(format!("compressed mesh: no {k}")))
            };
            let pv = get("m_Vertices")?;
            vertex_count = items(pv)? / 3;
            check_vertices(&name, vertex_count)?;
            // vertex_count is at most MAX_VERTICES: the products below stay far from overflow
            let times = |n: usize| vertex_count.saturating_mul(n);
            set_channel(
                &mut channels,
                0,
                Channel {
                    dim: 3,
                    values: unpack_floats(pv, 0, times(3))?,
                },
            )?;
            let uv = get("m_UV")?;
            if items(uv)? > 0 {
                let info = uint(cm, "m_UVInfo", "compressed mesh")?;
                if info == 0 {
                    return Err(Error::Unsupported("compressed mesh UVs without UV info".into()));
                }
                let mut at = 0usize;
                for ch in 0..8u32 {
                    let b = info.checked_shr(ch.saturating_mul(4)).unwrap_or(0) & 0xF;
                    if b & 4 != 0 {
                        let dim = usize::try_from(b & 3).unwrap_or(0).saturating_add(1);
                        set_channel(
                            &mut channels,
                            usize::try_from(ch).unwrap_or(0).saturating_add(4),
                            Channel {
                                dim,
                                values: unpack_floats(uv, at, times(dim))?,
                            },
                        )?;
                        at = at.saturating_add(times(dim));
                    }
                }
            }
            let n = get("m_Normals")?;
            if items(n)? > 0 {
                let xy = unpack_floats(n, 0, times(2))?;
                let signs = unpack_ints(get("m_NormalSigns")?, 0, vertex_count)?;
                let values = xy
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(&signs)
                    .flat_map(|(&[x, y], &sign)| rebuild_z(x, y, sign != 0))
                    .collect();
                set_channel(&mut channels, 1, Channel { dim: 3, values })?;
            }
            let t = get("m_Tangents")?;
            if items(t)? > 0 {
                let xy = unpack_floats(t, 0, times(2))?;
                let signs = unpack_ints(get("m_TangentSigns")?, 0, times(2))?;
                let values = xy
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .zip(signs.as_chunks::<2>().0)
                    .flat_map(|(&[x, y], &[z_sign, w_sign])| {
                        let [x, y, z] = rebuild_z(x, y, z_sign != 0);
                        [x, y, z, if w_sign > 0 { 1.0 } else { -1.0 }]
                    })
                    .collect();
                set_channel(&mut channels, 2, Channel { dim: 4, values })?;
            }
            let c = get("m_FloatColors")?;
            if items(c)? > 0 {
                set_channel(
                    &mut channels,
                    3,
                    Channel {
                        dim: 4,
                        values: unpack_floats(c, 0, times(4))?,
                    },
                )?;
            }
        } else {
            let stated = int(vd, "m_VertexCount")?;
            vertex_count =
                usize::try_from(stated).map_err(|_| invalid(format!("{name}: {stated} vertices")))?;
            check_vertices(&name, vertex_count)?;
            let chans = match vd.get("m_Channels") {
                Some(Value::Array(Array::Values(c))) => c,
                _ => return Err(invalid("mesh: no m_Channels")),
            };
            // stream offsets and strides
            let info: Vec<(usize, usize, i64, usize)> = chans
                .iter()
                .map(|c| {
                    let non_negative = |k: &str| {
                        let n = int(c, k)?;
                        usize::try_from(n).map_err(|_| invalid(format!("{name}: channel {k} of {n}")))
                    };
                    let stream = non_negative("stream")?;
                    // Unity's vertex data has at most four streams
                    if stream >= 4 {
                        return Err(invalid(format!("{name}: a channel in stream {stream}")));
                    }
                    Ok((
                        stream,
                        non_negative("offset")?,
                        int(c, "format")?,
                        usize::try_from(int(c, "dimension")? & 0xF).unwrap_or(0),
                    ))
                })
                .collect::<Result<_>>()?;
            let streams = info
                .iter()
                .filter(|c| c.3 > 0)
                .map(|c| c.0)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let mut stream_at = Vec::with_capacity(streams);
            let mut offset = 0usize;
            for s in 0..streams {
                let mut stride = 0;
                for &(cs, _, f, d) in &info {
                    if cs == s && d > 0 {
                        stride = d.saturating_mul(component_size(f)?).saturating_add(stride);
                    }
                }
                stream_at.push((offset, stride));
                // vertex_count is at most 2^24 and a stride 15 x 4 x 16 channels: no overflow; were there one, the
                // saturated size fails the check against the data below
                offset = offset.saturating_add(vertex_count.saturating_mul(stride));
                offset = offset.checked_next_multiple_of(16).unwrap_or(usize::MAX);
            }
            let stream_data = m
                .get("m_StreamData")
                .map(StreamingInfo::of)
                .transpose()?
                .flatten();
            let data: Vec<u8> = match stream_data {
                Some(s) => {
                    let dir = player
                        .folders
                        .get(&r.file)
                        .ok_or_else(|| invalid(format!("no folder for {}", r.file)))?;
                    s.read(dir)?
                }
                None => bytes(vd, "m_DataSize")?.to_vec(),
            };
            // every stream holds its vertices, so what is allocated per vertex below is backed by the data
            for (s, &(base, stride)) in stream_at.iter().enumerate() {
                if stride > 0 && base.saturating_add(vertex_count.saturating_mul(stride)) > data.len() {
                    return Err(invalid(format!(
                        "{name}: stream {s} of {vertex_count} vertices of {stride} bytes past the {} bytes of vertex data",
                        data.len()
                    )));
                }
            }
            skin_channels = read_skin_channels(&name, &info, &stream_at, &data, vertex_count)?;
            for (ch, &(s, off, f, d)) in info.iter().enumerate().take(12) {
                if d == 0 {
                    continue;
                }
                let semantic = CHANNELS
                    .get(ch)
                    .copied()
                    .ok_or_else(|| invalid(format!("{name}: channel {ch}")))?;
                let format = vertex_format(f, d)
                    .ok_or_else(|| Error::Unsupported(format!("{semantic} channel format {f} x {d}")))?;
                let layout = resolve_layout(&[Element {
                    semantic: semantic.to_owned(),
                    index: 0,
                    format,
                    slot: u32::try_from(s).map_err(|_| invalid(format!("{name}: stream {s}")))?,
                    offset: Some(off),
                }])
                .map_err(|e| invalid(format!("{semantic}: {e}")))?;
                let &(base, stride) = stream_at.get(s).ok_or_else(|| {
                    invalid(format!(
                        "{name}: {semantic} names stream {s} of {}",
                        stream_at.len()
                    ))
                })?;
                let element = layout
                    .first()
                    .ok_or_else(|| invalid(format!("{name}: no layout for {semantic}")))?;
                // non-finite values kept as stored: `make_finite` decides by what the stream is
                let v = decode_stream_raw(
                    element,
                    VertexStream {
                        bytes: &data,
                        stride,
                        offset: base,
                    },
                    0,
                    vertex_count,
                )
                .map_err(|e| invalid(format!("{name} {semantic}: {e}")))?;
                set_channel(
                    &mut channels,
                    ch,
                    Channel {
                        dim: d,
                        values: v.iter().flat_map(|x| x.iter().take(d).copied()).collect(),
                    },
                )?;
            }
        }
        // indices
        let (index_size, index_bytes): (u32, usize) = match int(&m, "m_IndexFormat")? {
            0 => (2, 2),
            1 => (4, 4),
            f => return Err(invalid(format!("index format {f}"))),
        };
        let ib = bytes(&m, "m_IndexBuffer")?;
        if ib.len().checked_rem(index_bytes) != Some(0) {
            return Err(invalid(format!("index buffer of {} bytes", ib.len())));
        }
        // a compressed mesh keeps its indices bit-packed (`m_Triangles`); its submeshes still count index bytes
        let packed = cm.get("m_Triangles").map(items).transpose()?.unwrap_or(0);
        let indices: Vec<u32> = if compressed && packed > 0 {
            unpack_ints(cm.get("m_Triangles").expect("checked"), 0, packed)?
        } else if index_size == 2 {
            ib.as_chunks::<2>()
                .0
                .iter()
                .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
                .collect()
        } else {
            ib.as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        };
        let submeshes = read_submeshes(&m, &name)?;
        if variable.is_some() && compressed {
            return Err(Error::Unsupported(format!(
                "{name}: compressed mesh with variable bone weights"
            )));
        }
        let skin = match skin_channels {
            Some((influences, weights, bones)) => {
                let bind_poses = bind_poses(&m, &name)?;
                let channel = Blend::from_padded(influences, &weights, &bones);
                let unlimited = variable
                    .map(|d| read_variable_weights(&name, d, vertex_count))
                    .transpose()?;
                for blend in std::iter::once(&channel).chain(&unlimited) {
                    let bad = blend.weights.iter().zip(&blend.bones).position(|(w, b)| {
                        *w > 0.0 && usize::try_from(*b).map_or(true, |b| b >= bind_poses.len())
                    });
                    if let Some(i) = bad {
                        return Err(invalid(format!(
                            "{name}: an influence binds bone {} of {} bind poses",
                            blend.bones.get(i).copied().unwrap_or_default(),
                            bind_poses.len()
                        )));
                    }
                }
                if bind_poses.len() > usize::from(u16::MAX) {
                    return Err(Error::Unsupported(format!(
                        "{name}: {} bind poses (bone slots are 16 bits)",
                        bind_poses.len()
                    )));
                }
                Some(MeshSkin {
                    channel,
                    unlimited,
                    bind_poses,
                })
            }
            None if variable.is_some() => {
                return Err(invalid(format!(
                    "{name}: variable bone weights without blend channels"
                )));
            }
            None => None,
        };
        let shapes = read_shapes(&m, &name, vertex_count)?;
        Ok(UnityMeshData {
            name,
            vertex_count,
            channels,
            skin,
            compressed,
            shapes,
            indices,
            index_size,
            submeshes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packed vector states its own item count: negative or enormous counts of no-bit items are refused before the
    /// vector is made, and counts of stored bits are held to the bytes.
    #[test]
    fn packed_vectors_are_bounded_by_their_bytes_or_a_constant() {
        let packed = |items: i64, bits: i64, data: Vec<u8>| {
            Value::Struct(vec![
                ("m_NumItems".into(), Value::Int(items)),
                ("m_Range".into(), Value::Float(1.0)),
                ("m_Start".into(), Value::Float(0.5)),
                ("m_Data".into(), Value::Array(Array::U8(data))),
                ("m_BitSize".into(), Value::Int(bits)),
            ])
        };
        assert!(
            items(&packed(-1, 0, vec![]))
                .unwrap_err()
                .to_string()
                .contains("m_NumItems of -1")
        );
        assert_eq!(unpack_floats(&packed(3, 0, vec![]), 0, 3).unwrap(), [0.5; 3]);
        let huge = MAX_PACKED_ITEMS + 1;
        assert!(unpack_ints(&packed(0, 0, vec![]), 0, huge).is_err());
        assert!(unpack_floats(&packed(0, 0, vec![]), 0, huge).is_err());
        // bits stored: the bytes bound the count, even when start + count wraps
        assert!(unpack_ints(&packed(0, 8, vec![1, 2]), 0, 3).is_err());
        assert!(unpack_ints(&packed(0, 8, vec![1, 2]), usize::MAX, 2).is_err());
        assert_eq!(unpack_ints(&packed(0, 8, vec![1, 2]), 0, 2).unwrap(), [1, 2]);
        assert!(unpack_ints(&packed(0, -1, vec![]), 0, 1).is_err());
        assert!(unpack_floats(&packed(0, 33, vec![]), 0, 1).is_err());
    }

    /// The `info` rows of a mesh whose only channels are blend weights (12) and indices (13): (stream, offset, format,
    /// dimension).
    fn blend_info(
        weights: (usize, usize, i64, usize),
        indices: (usize, usize, i64, usize),
    ) -> Vec<(usize, usize, i64, usize)> {
        let mut info = vec![(0, 0, 0, 0); 14];
        info[12] = weights;
        info[13] = indices;
        info
    }

    /// Two influences a vertex: float weights then 32-bit indices in one stream of stride 16, a rigid bind (indices
    /// only), 16-bit normalised weights, and the refusals: weights without indices, other dimensions, five influences,
    /// an unsupported format and data short of the vertices.
    #[test]
    fn blend_channels_read_by_their_formats_and_refuse_the_rest() {
        let mut data = Vec::new();
        for (w, i) in [([0.75f32, 0.25], [3u32, 1]), ([1.0, 0.0], [2, 0])] {
            w.iter().for_each(|x| data.extend(x.to_le_bytes()));
            i.iter().for_each(|x| data.extend(x.to_le_bytes()));
        }
        let info = blend_info((0, 0, 0, 2), (0, 8, 10, 2));
        let (k, weights, bones) = read_skin_channels("m", &info, &[(0, 16)], &data, 2)
            .unwrap()
            .unwrap();
        assert_eq!(k, 2);
        assert_eq!(weights, vec![[0.75, 0.25, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]);
        assert_eq!(bones, vec![[3, 1, 0, 0], [2, 0, 0, 0]]);

        // no weights channel: each vertex wholly on its one bone (8-bit indices here)
        let (k, weights, bones) = read_skin_channels(
            "m",
            &blend_info((0, 0, 0, 0), (0, 0, 6, 1)),
            &[(0, 1)],
            &[5, 7],
            2,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            (k, weights, bones),
            (1, vec![[1.0, 0.0, 0.0, 0.0]; 2], vec![[5, 0, 0, 0], [7, 0, 0, 0]])
        );

        // 16-bit normalised weights with 16-bit indices
        let data = [0xFF, 0xFF, 0x00, 0x00, 4, 0, 9, 0];
        let (_, weights, bones) =
            read_skin_channels("m", &blend_info((0, 0, 4, 2), (0, 4, 8, 2)), &[(0, 8)], &data, 1)
                .unwrap()
                .unwrap();
        assert_eq!((weights, bones), (vec![[1.0, 0.0, 0.0, 0.0]], vec![[4, 9, 0, 0]]));

        assert!(
            read_skin_channels("m", &blend_info((0, 0, 0, 0), (0, 0, 0, 0)), &[(0, 0)], &[], 0)
                .unwrap()
                .is_none()
        );
        let refused = |info: Vec<(usize, usize, i64, usize)>, stride: usize, len: usize| {
            read_skin_channels("m", &info, &[(0, stride)], &vec![0u8; len], 2)
                .unwrap_err()
                .to_string()
        };
        assert!(refused(blend_info((0, 0, 0, 2), (0, 0, 0, 0)), 8, 16).contains("without blend indices"));
        assert!(
            refused(blend_info((0, 0, 0, 2), (0, 8, 10, 3)), 20, 40)
                .contains("2 blend weights and 3 blend indices")
        );
        assert!(refused(blend_info((0, 0, 0, 0), (0, 0, 10, 2)), 8, 16).contains("no blend weights"));
        assert!(refused(blend_info((0, 0, 0, 4), (0, 16, 10, 5)), 36, 72).contains("5 bone influences"));
        assert!(
            refused(blend_info((0, 0, 3, 2), (0, 8, 10, 2)), 16, 32).contains("blend weights of format 3")
        );
        assert!(
            refused(blend_info((0, 0, 0, 2), (0, 8, 5, 2)), 12, 24).contains("blend indices of format 5")
        );
        assert!(refused(blend_info((0, 0, 0, 2), (0, 8, 10, 2)), 16, 20).contains("past the vertex data"));
    }

    /// The variable-bone-count data: start offsets, then a word per influence (bone in the low 16 bits, weight in the
    /// high 16); its refusals.
    #[test]
    fn variable_bone_weights_read_by_their_offsets() {
        let word = |weight: u16, bone: u16| (u32::from(weight) << 16) | u32::from(bone);
        // two vertices: five influences summing to 65535, then three (the last of weight 0)
        let data = [
            3,
            8,
            11,
            word(32768, 6),
            word(16384, 7),
            word(8192, 5),
            word(4096, 15),
            word(4095, 3),
            word(43690, 2),
            word(21845, 1),
            word(0, 0),
        ];
        let b = read_variable_weights("m", &data, 2).unwrap();
        assert_eq!(b.starts, vec![0, 5, 8]);
        assert_eq!(b.bones, vec![6, 7, 5, 15, 3, 2, 1, 0]);
        assert_eq!(b.weights[0], 32768.0 / 65535.0);
        assert_eq!(b.weights[7], 0.0);
        assert_eq!(b.max_influences(), 5);
        assert_eq!(b.vertex(1).unwrap().1, &[2, 1, 0]);
        assert_eq!(b.vertex(9), None);
        let refused = |data: &[u32], n: usize| read_variable_weights("m", data, n).unwrap_err().to_string();
        assert!(refused(&data[..2], 2).contains("words for 2 vertices"));
        let mut bad = data;
        bad[0] = 4;
        assert!(refused(&bad, 2).contains("start offsets"));
        let mut bad = data;
        bad[2] = 12;
        assert!(refused(&bad, 2).contains("start offsets"));
        let mut bad = data;
        bad[1] = 3;
        assert!(refused(&bad, 2).contains("start offsets"));
    }

    /// What a quality level makes of a skin: the channel's four influences under 4 or fewer bones (cut and renormalised
    /// below the stored count), the full influences under unlimited; levels with one outcome draw one mesh.
    #[test]
    fn the_skin_weights_limit_decides_the_influences_drawn() {
        let channel = Blend::from_padded(4, &[[0.5, 0.25, 0.125, 0.125]], &[[3, 2, 1, 0]]);
        let unlimited = Blend {
            starts: vec![0, 5],
            weights: vec![0.4, 0.25, 0.125, 0.125, 0.1],
            bones: vec![3, 2, 1, 0, 4],
        };
        let skin = MeshSkin {
            channel: channel.clone(),
            unlimited: Some(unlimited),
            bind_poses: vec![[[0.0; 4]; 4]; 5],
        };
        assert_eq!(skin.outcome(SkinWeights::Bones(4)), (true, 4));
        assert_eq!(skin.outcome(SkinWeights::Bones(2)), (true, 2));
        assert_eq!(skin.outcome(SkinWeights::Bones(1)), (true, 1));
        assert_eq!(skin.outcome(SkinWeights::Unlimited), (false, 5));
        assert_eq!(skin.sets(SkinWeights::Bones(4)), 1);
        assert_eq!(skin.sets(SkinWeights::Unlimited), 2);
        let (w, b, cut) = skin.padded(SkinWeights::Bones(4));
        assert_eq!(
            (w, b, cut),
            (vec![vec![[0.5, 0.25, 0.125, 0.125]]], vec![vec![[3, 2, 1, 0]]], 0)
        );
        let (w, b, cut) = skin.padded(SkinWeights::Bones(2));
        assert_eq!(w, vec![vec![[0.5 / 0.75, 0.25 / 0.75, 0.0, 0.0]]]);
        assert_eq!((b, cut), (vec![vec![[3, 2, 0, 0]]], 1));
        let (w, b, cut) = skin.padded(SkinWeights::Unlimited);
        assert_eq!(
            w,
            vec![vec![[0.4, 0.25, 0.125, 0.125]], vec![[0.1, 0.0, 0.0, 0.0]]]
        );
        assert_eq!((b, cut), (vec![vec![[3, 2, 1, 0]], vec![[4, 0, 0, 0]]], 0));
        // a skin with no full influences draws its channel at unlimited too
        let plain = MeshSkin {
            unlimited: None,
            ..skin
        };
        assert_eq!(plain.outcome(SkinWeights::Unlimited), (true, 4));
        assert_eq!(
            plain.outcome(SkinWeights::Unlimited),
            plain.outcome(SkinWeights::Bones(4))
        );
        assert_eq!(SkinWeights::from_setting(255).unwrap(), SkinWeights::Unlimited);
        assert_eq!(SkinWeights::from_setting(2).unwrap(), SkinWeights::Bones(2));
        assert!(SkinWeights::from_setting(3).is_err());
    }

    fn field(name: &str, v: Value) -> (std::sync::Arc<str>, Value) {
        (name.into(), v)
    }

    fn list(items: Vec<Value>) -> Value {
        Value::Array(Array::Values(items))
    }

    /// A `m_Shapes` of one channel of one frame of one vertex, with the stored numbers given.
    fn shapes(first_vertex: i64, vertex_count: i64, frame_index: i64, name_hash: i64) -> Value {
        let v3 = |x: f32| {
            Value::Struct(vec![
                field("x", Value::Float(x)),
                field("y", Value::Float(0.0)),
                field("z", Value::Float(0.0)),
            ])
        };
        let m_shapes = Value::Struct(vec![
            field(
                "channels",
                list(vec![Value::Struct(vec![
                    field("name", Value::String("smile".into())),
                    field("nameHash", Value::Int(name_hash)),
                    field("frameIndex", Value::Int(frame_index)),
                    field("frameCount", Value::Int(1)),
                ])]),
            ),
            field("fullWeights", Value::Array(Array::F32(vec![100.0]))),
            field(
                "vertices",
                list(vec![Value::Struct(vec![
                    field("index", Value::Int(0)),
                    field("vertex", v3(1.0)),
                    field("normal", v3(0.0)),
                    field("tangent", v3(0.0)),
                ])]),
            ),
            field(
                "shapes",
                list(vec![Value::Struct(vec![
                    field("firstVertex", Value::Int(first_vertex)),
                    field("vertexCount", Value::Int(vertex_count)),
                    field("hasNormals", Value::Bool(false)),
                    field("hasTangents", Value::Bool(false)),
                ])]),
            ),
        ]);
        Value::Struct(vec![field("m_Shapes", m_shapes)])
    }

    /// Stored runs and hashes that overflow, are negative or do not fit 32 bits are refused where the old casts and
    /// sums panicked or wrapped.
    #[test]
    fn blend_shape_numbers_are_checked_not_cast() {
        let ok = read_shapes(&shapes(0, 1, 0, 0xdead_beef), "m", 1)
            .unwrap()
            .unwrap();
        assert_eq!((ok.channels[0].name_hash, ok.frames[0].count), (0xdead_beef, 1));
        for (first, count, frame, hash, wants) in [
            (i64::MAX, 1, 0, 0, "entries 9223372036854775807+1 of 1"),
            (0, i64::MAX, 0, 0, "entries 0+9223372036854775807 of 1"),
            (-1, 2, 0, 0, "entries -1+2 of 1"),
            (0, 1, i64::MAX, 0, "entries 9223372036854775807+1 of 1"),
            (0, 1, 0, 1 << 32, "nameHash 4294967296"),
            (0, 1, 0, -1, "nameHash -1"),
        ] {
            let e = read_shapes(&shapes(first, count, frame, hash), "m", 1)
                .unwrap_err()
                .to_string();
            assert!(e.contains(wants), "{e}");
        }
    }

    fn submesh(first_byte: i64, index_count: i64, base_vertex: i64) -> Value {
        Value::Struct(vec![
            field("firstByte", Value::Int(first_byte)),
            field("indexCount", Value::Int(index_count)),
            field("topology", Value::Int(0)),
            field("baseVertex", Value::Int(base_vertex)),
        ])
    }

    /// A submesh's numbers are 32-bit unsigned: a stored 2^32 + 6 is refused, not read as 6, and the error names the
    /// submesh and the field.
    #[test]
    fn submesh_numbers_that_do_not_fit_are_refused_not_wrapped() {
        let mesh = |subs| Value::Struct(vec![field("m_SubMeshes", list(subs))]);
        let ok = read_submeshes(&mesh(vec![submesh(6, 3, 0)]), "m").unwrap();
        assert_eq!((ok[0].first_byte, ok[0].index_count), (6, 3));
        for (sub, wants) in [
            (submesh((1 << 32) + 6, 3, 0), "firstByte 4294967302"),
            (submesh(0, -1, 0), "indexCount -1"),
            (submesh(0, 3, 1 << 33), "baseVertex 8589934592"),
        ] {
            let e = read_submeshes(&mesh(vec![submesh(0, 3, 0), sub]), "m")
                .unwrap_err()
                .to_string();
            assert!(e.contains("submesh 1") && e.contains(wants), "{e}");
        }
    }

    /// A channel offset near the top of `usize` is a read past the data, not an overflow.
    #[test]
    fn blend_offsets_that_overflow_are_refused() {
        let data = [0u8; 32];
        for offset in [usize::MAX, usize::MAX - 3] {
            let info = blend_info((0, 0, 0, 0), (0, offset, 10, 1));
            let e = read_skin_channels("m", &info, &[(0, 4)], &data, 2)
                .unwrap_err()
                .to_string();
            assert!(e.contains("past the vertex data (32 bytes)"), "{e}");
        }
    }

    /// Five influences asked of the padded constructor are cut to the four a padded list holds.
    #[test]
    fn padded_blends_hold_at_most_four_influences() {
        assert_eq!(Blend::from_padded(9, &[[0.25; 4]], &[[1; 4]]).bones, vec![1; 4]);
    }
}
