//! Unity terrains as stored: `TerrainData` (the heightmap, holes, splat layers and their alphamaps, tree prototypes
//! and instances) and `TerrainLayer`.
//!
//! The heightmap is `resolution` x `resolution` samples, row `z` then column `x`, each 0..=32766 for 0..1 of the
//! terrain's height (`scale.y` metres; `TerrainData.GetHeight` is `h / 32766 * size.y`); samples are `scale.x` and
//! `scale.z` metres apart from the terrain's position, so the terrain is `(resolution - 1) * scale` wide. Holes are
//! one byte per cell, 0 a hole (none stored: no holes). Alphamaps hold four layer weights per texel (RGBA), texel
//! row 0 at z = 0 as Unity stores rows.

use crate::player::{ObjRef, Player};
use crate::value::{Array, Value};
use crate::{Result, invalid};

/// The highest stored height sample: 0..=32766 is 0..1 of the terrain's height (`TerrainData.GetHeight`).
pub const MAX_HEIGHT: i16 = 32766;

/// Why [`check_heightmap`] refuses a heightmap.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HeightmapError {
    /// The resolution leaves no cell.
    #[error("resolution {resolution}: a heightmap has at least 2 x 2 samples")]
    Resolution {
        /// The stated resolution.
        resolution: u32,
    },
    /// The resolution squared does not fit the address space.
    #[error("resolution {resolution}: the heightmap's sample count overflows")]
    SampleCount {
        /// The stated resolution.
        resolution: u32,
    },
    /// The number of height samples is not the resolution squared.
    #[error("{found} heights for resolution {resolution} ({expected} expected)")]
    Heights {
        /// The stated resolution.
        resolution: u32,
        /// The samples stored.
        found: usize,
        /// The samples the resolution needs.
        expected: usize,
    },
    /// A sample is outside 0..=[`MAX_HEIGHT`].
    #[error("height sample {index} (row {row}, column {column}) is {value}, outside 0..={MAX_HEIGHT}")]
    HeightRange {
        /// The sample's place in the array.
        index: usize,
        /// Its row.
        row: usize,
        /// Its column.
        column: usize,
        /// Its value.
        value: i16,
    },
    /// The number of holes is not the number of cells.
    #[error("{found} holes for {cells} cells")]
    Holes {
        /// The holes stored.
        found: usize,
        /// The cells of the heightmap.
        cells: usize,
    },
}

impl From<HeightmapError> for crate::Error {
    fn from(e: HeightmapError) -> Self {
        invalid(format!("terrain: {e}"))
    }
}

/// What a heightmap must be to be read: `resolution` x `resolution` samples (at least 2 x 2: one cell), each
/// 0..=[`MAX_HEIGHT`], and, where it has holes, one per cell ((resolution - 1)^2).
pub fn check_heightmap(resolution: u32, heights: &[i16], holes: Option<&[u8]>) -> Result<(), HeightmapError> {
    if resolution < 2 {
        return Err(HeightmapError::Resolution { resolution });
    }
    let n = usize::try_from(resolution).map_err(|_| HeightmapError::SampleCount { resolution })?;
    let samples = n
        .checked_mul(n)
        .ok_or(HeightmapError::SampleCount { resolution })?;
    if heights.len() != samples {
        return Err(HeightmapError::Heights {
            resolution,
            found: heights.len(),
            expected: samples,
        });
    }
    if let Some((index, &value)) = heights
        .iter()
        .enumerate()
        .find(|(_, h)| !(0..=MAX_HEIGHT).contains(*h))
    {
        return Err(HeightmapError::HeightRange {
            index,
            row: index.div_euclid(n),
            column: index.rem_euclid(n),
            value,
        });
    }
    // n >= 2 and n * n did not overflow, so (n - 1)^2 does not either
    let cells = n.saturating_sub(1).saturating_mul(n.saturating_sub(1));
    match holes {
        Some(h) if h.len() != cells => Err(HeightmapError::Holes {
            found: h.len(),
            cells,
        }),
        _ => Ok(()),
    }
}

/// A tree placed on a terrain: position as 0..1 of the terrain's size, scales, rotation about y (radians), colour,
/// and its prototype's index.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeInstance {
    /// Position as 0 to 1 of the terrain's size along each axis.
    pub position: [f32; 3],
    /// `widthScale`.
    pub width_scale: f32,
    /// `heightScale`.
    pub height_scale: f32,
    /// Rotation about the y axis, in radians.
    pub rotation: f32,
    /// The tint, packed as stored (`color.rgba`).
    pub color: u32,
    /// Index into [`TerrainData::tree_prototypes`].
    pub prototype: usize,
}

/// A `TerrainData` as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainData {
    /// `m_Name`.
    pub name: String,
    /// Samples along each side of the heightmap.
    pub resolution: u32,
    /// Metres between samples along x, the height in metres along y, metres between samples along z (`m_Scale`).
    pub scale: [f32; 3],
    /// The height samples, row `z` then column `x`, each 0 to [`MAX_HEIGHT`].
    pub heights: Vec<i16>,
    /// One byte per cell, 0 a hole; `None` when none are stored.
    pub holes: Option<Vec<u8>>,
    /// The splat layers (`m_TerrainLayers`), read with [`TerrainLayerData::read`]; `None` for a null pointer.
    pub layers: Vec<Option<ObjRef>>,
    /// The splat weight textures (`m_AlphaTextures`), four layers' weights each as RGBA.
    pub alphamaps: Vec<Option<ObjRef>>,
    /// `m_AlphamapResolution`: the side of an alphamap in texels.
    pub alphamap_resolution: u32,
    /// Each tree prototype's prefab.
    pub tree_prototypes: Vec<Option<ObjRef>>,
    /// The tree instances.
    pub trees: Vec<TreeInstance>,
}

/// A `TerrainLayer` as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainLayerData {
    /// `m_Name`.
    pub name: String,
    /// The diffuse texture.
    pub diffuse: Option<ObjRef>,
    /// The normal map.
    pub normal: Option<ObjRef>,
    /// The mask map.
    pub mask: Option<ObjRef>,
    /// `m_TileSize`: how many metres one repeat covers.
    pub tile_size: [f32; 2],
    /// `m_TileOffset`.
    pub tile_offset: [f32; 2],
    /// `m_Specular`, RGBA.
    pub specular: [f32; 4],
    /// `m_Metallic`.
    pub metallic: f32,
    /// `m_Smoothness`.
    pub smoothness: f32,
    /// `m_NormalScale`.
    pub normal_scale: f32,
    /// Min and max, RGBA.
    pub diffuse_remap: [[f32; 4]; 2],
    /// `m_MaskMapRemapMin` and `m_MaskMapRemapMax`: min and max, RGBA.
    pub mask_remap: [[f32; 4]; 2],
}

fn get<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k).ok_or_else(|| invalid(format!("terrain: no {k}")))
}

fn float(v: &Value, k: &str) -> Result<f32> {
    get(v, k)?
        .as_f32()
        .ok_or_else(|| invalid(format!("terrain: {k} is not a float")))
}

fn int(v: &Value, k: &str) -> Result<i64> {
    get(v, k)?
        .as_i64()
        .ok_or_else(|| invalid(format!("terrain: {k} is not an integer")))
}

fn vec<const N: usize>(v: &Value, k: &str, names: [&str; N]) -> Result<[f32; N]> {
    let x = get(v, k)?;
    let mut out = [0.0; N];
    for (o, n) in out.iter_mut().zip(names) {
        *o = float(x, n)?;
    }
    Ok(out)
}

fn list<'a>(v: &'a Value, k: &str) -> Result<&'a [Value]> {
    match get(v, k)? {
        Value::Array(Array::Values(items)) => Ok(items),
        Value::Array(a) if a.is_empty() => Ok(&[]),
        _ => Err(invalid(format!("terrain: {k} is not a list"))),
    }
}

fn pptrs(player: &Player, file: &str, items: &[Value], field: Option<&str>) -> Result<Vec<Option<ObjRef>>> {
    items
        .iter()
        .map(|p| {
            let p = match field {
                Some(f) => get(p, f)?,
                None => p,
            };
            player.resolve(file, p)
        })
        .collect()
}

/// The tree instances `items` states, each naming one of `prototypes` prototypes and a colour that fits 32 bits.
fn read_trees(items: &[Value], prototypes: usize) -> Result<Vec<TreeInstance>> {
    items
        .iter()
        .enumerate()
        .map(|(n, i)| {
            let index = int(i, "index")?;
            let prototype = usize::try_from(index)
                .ok()
                .filter(|&p| p < prototypes)
                .ok_or_else(|| {
                    invalid(format!(
                        "terrain: tree {n} uses prototype {index} of {prototypes}"
                    ))
                })?;
            let rgba = int(get(i, "color")?, "rgba")?;
            let color = u32::try_from(rgba).map_err(|_| {
                invalid(format!(
                    "terrain: tree {n}: colour {rgba} is not a 32-bit unsigned number"
                ))
            })?;
            Ok(TreeInstance {
                position: vec(i, "position", ["x", "y", "z"])?,
                width_scale: float(i, "widthScale")?,
                height_scale: float(i, "heightScale")?,
                rotation: float(i, "rotation")?,
                color,
                prototype,
            })
        })
        .collect()
}

impl TerrainData {
    /// Read the TerrainData `r` of `player`, its heightmap as [`check_heightmap`] requires.
    pub fn read(player: &Player, r: &ObjRef) -> Result<Self> {
        let t = player.decode(r)?;
        let hm = get(&t, "m_Heightmap")?;
        let resolution = int(hm, "m_Resolution")?;
        let resolution = u32::try_from(resolution)
            .map_err(|_| invalid(format!("terrain: resolution {resolution} is not a sample count")))?;
        let heights = match get(hm, "m_Heights")? {
            Value::Array(Array::I16(h)) => h.clone(),
            _ => return Err(invalid("terrain: m_Heights is not 16-bit")),
        };
        let holes: Option<Vec<u8>> = match hm.get("m_Holes") {
            Some(Value::Array(Array::U8(h))) if !h.is_empty() => Some(h.clone()),
            Some(Value::Array(Array::Bool(h))) if !h.is_empty() => {
                Some(h.iter().map(|&b| u8::from(b)).collect())
            }
            _ => None,
        };
        check_heightmap(resolution, &heights, holes.as_deref())?;
        let splat = get(&t, "m_SplatDatabase")?;
        let detail = get(&t, "m_DetailDatabase")?;
        let prototypes = list(detail, "m_TreePrototypes")?;
        let trees = read_trees(list(detail, "m_TreeInstances")?, prototypes.len())?;
        let alphamap_resolution = int(splat, "m_AlphamapResolution")?;
        let alphamap_resolution = u32::try_from(alphamap_resolution).map_err(|_| {
            invalid(format!(
                "terrain: m_AlphamapResolution {alphamap_resolution} is not a 32-bit unsigned number"
            ))
        })?;
        Ok(TerrainData {
            name: t.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned(),
            resolution,
            scale: vec(hm, "m_Scale", ["x", "y", "z"])?,
            heights,
            holes,
            layers: pptrs(player, &r.file, list(splat, "m_TerrainLayers")?, None)?,
            alphamaps: pptrs(player, &r.file, list(splat, "m_AlphaTextures")?, None)?,
            alphamap_resolution,
            tree_prototypes: pptrs(player, &r.file, prototypes, Some("prefab"))?,
            trees,
        })
    }

    /// The height of sample (x, z) in metres above the terrain's position; `None` when (x, z) is not a sample of the
    /// heightmap.
    pub fn height(&self, x: usize, z: usize) -> Option<f64> {
        let side = usize::try_from(self.resolution).ok()?;
        if x >= side || z >= side {
            return None;
        }
        let sample = self.heights.get(z.checked_mul(side)?.checked_add(x)?)?;
        Some(f64::from(*sample) / f64::from(MAX_HEIGHT) * f64::from(self.scale[1]))
    }

    /// The terrain's size in metres: (resolution - 1) * scale along x and z, scale.y high.
    pub fn size(&self) -> [f64; 3] {
        let cells = f64::from(self.resolution.saturating_sub(1));
        [
            cells * f64::from(self.scale[0]),
            f64::from(self.scale[1]),
            cells * f64::from(self.scale[2]),
        ]
    }
}

impl TerrainLayerData {
    /// Read the TerrainLayer `r` of `player`.
    pub fn read(player: &Player, r: &ObjRef) -> Result<Self> {
        let l = player.decode(r)?;
        let tex = |k: &str| -> Result<Option<ObjRef>> {
            match l.get(k) {
                Some(p) => player.resolve(&r.file, p),
                None => Ok(None),
            }
        };
        let rgba = ["x", "y", "z", "w"];
        Ok(TerrainLayerData {
            name: l.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned(),
            diffuse: tex("m_DiffuseTexture")?,
            normal: tex("m_NormalMapTexture")?,
            mask: tex("m_MaskMapTexture")?,
            tile_size: vec(&l, "m_TileSize", ["x", "y"])?,
            tile_offset: vec(&l, "m_TileOffset", ["x", "y"])?,
            specular: vec(&l, "m_Specular", ["r", "g", "b", "a"])?,
            metallic: float(&l, "m_Metallic")?,
            smoothness: float(&l, "m_Smoothness")?,
            normal_scale: float(&l, "m_NormalScale")?,
            diffuse_remap: [
                vec(&l, "m_DiffuseRemapMin", rgba)?,
                vec(&l, "m_DiffuseRemapMax", rgba)?,
            ],
            mask_remap: [
                vec(&l, "m_MaskMapRemapMin", rgba)?,
                vec(&l, "m_MaskMapRemapMax", rgba)?,
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A heightmap has at least one cell, its samples within 0..=32766, and one hole per cell where it has holes;
    /// resolution 0 and 1 are refused before any cell count is taken.
    #[test]
    fn a_heightmap_is_checked_before_its_cells_are_counted() {
        assert!(check_heightmap(2, &[0, 1, 2, MAX_HEIGHT], None).is_ok());
        assert!(check_heightmap(2, &[0; 4], Some(&[1])).is_ok());
        for resolution in [0, 1] {
            assert_eq!(
                check_heightmap(resolution, &[], Some(&[])).unwrap_err(),
                HeightmapError::Resolution { resolution }
            );
        }
        assert_eq!(
            check_heightmap(2, &[0; 3], None).unwrap_err(),
            HeightmapError::Heights {
                resolution: 2,
                found: 3,
                expected: 4
            }
        );
        let range = check_heightmap(2, &[0, 0, -1, 0], None).unwrap_err();
        assert_eq!(
            range,
            HeightmapError::HeightRange {
                index: 2,
                row: 1,
                column: 0,
                value: -1
            }
        );
        assert_eq!(
            range.to_string(),
            "height sample 2 (row 1, column 0) is -1, outside 0..=32766"
        );
        assert!(check_heightmap(2, &[0, 0, 0, i16::MAX], None).is_err());
        let holes = check_heightmap(3, &[0; 9], Some(&[1; 3])).unwrap_err();
        assert_eq!(holes, HeightmapError::Holes { found: 3, cells: 4 });
        assert_eq!(holes.to_string(), "3 holes for 4 cells");
        // the typed error becomes the crate's refusal
        let e = crate::Error::from(holes);
        assert!(matches!(e, crate::Error::Invalid(m) if m == "terrain: 3 holes for 4 cells"));
    }

    fn terrain(resolution: u32, heights: Vec<i16>, scale_y: f32) -> TerrainData {
        TerrainData {
            name: String::new(),
            resolution,
            scale: [1.0, scale_y, 1.0],
            heights,
            holes: None,
            layers: vec![],
            alphamaps: vec![],
            alphamap_resolution: 0,
            tree_prototypes: vec![],
            trees: vec![],
        }
    }

    /// A sample is the stored height over [`MAX_HEIGHT`] of the terrain's height; a sample outside the heightmap is
    /// `None`, also where `z * resolution + x` would land in the next row.
    #[test]
    fn heights_are_scaled_by_max_height_and_bounded() {
        let t = terrain(2, vec![0, MAX_HEIGHT, 16383, 0], 100.0);
        assert_eq!(t.height(1, 0), Some(100.0));
        assert_eq!(
            t.height(0, 1),
            Some(f64::from(16383) / f64::from(MAX_HEIGHT) * 100.0)
        );
        assert_eq!(t.height(2, 0), None);
        assert_eq!(t.height(0, 2), None);
        assert_eq!(t.height(usize::MAX, usize::MAX), None);
        assert_eq!(terrain(0, vec![], 1.0).size(), [0.0, 1.0, 0.0]);
    }

    fn tree(index: i64, rgba: i64) -> Value {
        let f = |k: &str, v: Value| (std::sync::Arc::<str>::from(k), v);
        let v3 = Value::Struct(vec![
            f("x", Value::Float(0.5)),
            f("y", Value::Float(0.0)),
            f("z", Value::Float(0.5)),
        ]);
        Value::Struct(vec![
            f("position", v3),
            f("widthScale", Value::Float(1.0)),
            f("heightScale", Value::Float(1.0)),
            f("rotation", Value::Float(0.0)),
            f("color", Value::Struct(vec![f("rgba", Value::Int(rgba))])),
            f("index", Value::Int(index)),
        ])
    }

    /// A tree's prototype must be one of the prototypes and its colour fit 32 bits: neither is cast to fit.
    #[test]
    fn trees_name_a_prototype_and_a_colour_that_exist() {
        let ok = read_trees(&[tree(1, 0xff00_ff00)], 2).unwrap();
        assert_eq!((ok[0].prototype, ok[0].color), (1, 0xff00_ff00));
        for (index, rgba, wants) in [
            (2, 0, "tree 1 uses prototype 2 of 2"),
            (-1, 0, "tree 1 uses prototype -1 of 2"),
            (i64::MAX, 0, "prototype 9223372036854775807 of 2"),
            (0, 1 << 32, "colour 4294967296"),
            (0, -1, "colour -1"),
        ] {
            let e = read_trees(&[tree(0, 0), tree(index, rgba)], 2)
                .unwrap_err()
                .to_string();
            assert!(e.contains(wants), "{e}");
        }
    }
}
