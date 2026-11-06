//! Unity textures (`Texture2D`): the pixel format, the mip chain from the object or its streamed resource, colour
//! space, how the texture is used, and its sampler state.
//!
//! Unity stores rows bottom first (row 0 is v = 0, its texture origin being bottom-left); the bytes are returned as
//! stored.

use std::borrow::Cow;

use crate::crunch::{CrunchFormat, CrunchHeader, unpack_level};
use crate::player::{ObjRef, Player};
use crate::resource::StreamingInfo;
use crate::value::Value;
use crate::version::UnityVersion;
use crate::{Error, Result, invalid};

/// Unity's TextureUsageMode for a normal map stored as DXT5nm: X in alpha, Y in green.
pub const USAGE_NORMAL_DXT5NM: i64 = 3;
/// Unity's TextureUsageMode for a normal map stored as RGB.
pub const USAGE_NORMAL_PLAIN: i64 = 4;

/// A `Texture2D` as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct UnityTexture {
    /// `m_Name`.
    pub name: String,
    /// Width in pixels of the largest level.
    pub width: u32,
    /// Height in pixels of the largest level.
    pub height: u32,
    /// Unity's TextureFormat id.
    pub format: i64,
    /// `m_MipCount`: how many levels `data` holds.
    pub mips: u32,
    /// `m_ColorSpace` 1: sampled as sRGB.
    pub srgb: bool,
    /// Unity's TextureUsageMode (`m_LightmapFormat`).
    pub usage: i64,
    /// TextureWrapMode along u and v (0 repeat, 1 clamp, 2 mirror, 3 mirror once).
    pub wrap: [i64; 2],
    /// FilterMode (0 point, 1 bilinear, 2 trilinear).
    pub filter: i64,
    /// `m_MipBias`: added to the level of detail the texture is sampled at.
    pub mip_bias: f32,
    /// `m_Aniso`: the anisotropic filtering level (above 1: anisotropic).
    pub aniso: i64,
    /// All levels, largest first, bottom row first; for a crunched format, the crunch file that holds them.
    pub data: Vec<u8>,
    /// The Unity version of the file the texture is in (crunched textures before 2017.3 are of crunch's original
    /// format).
    pub unity: UnityVersion,
}

/// The first Unity version whose crunched textures are of Unity's crunch format.
pub const UNITY_CRUNCH: UnityVersion = UnityVersion::new(2017, 3, 0, 0, 0);

/// A non-negative 32-bit integer of the texture (a negative one is no size).
fn dimension(v: &Value, k: &str) -> Result<u32> {
    let n = int(v, k)?;
    u32::try_from(n).map_err(|_| invalid(format!("texture: {k} of {n}")))
}

fn int(v: &Value, k: &str) -> Result<i64> {
    v.get(k)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("texture: no integer {k}")))
}

fn float(v: &Value, k: &str) -> Result<f32> {
    v.get(k)
        .and_then(Value::as_f32)
        .ok_or_else(|| invalid(format!("texture: no float {k}")))
}

/// The pixel formats of Unity's `TextureFormat` this reader names, each with how its bytes are laid out. Multi-byte
/// channels are little-endian. A crunched format's data is a crunch file ([`crate::crunch`]) that
/// [`UnityTexture::levels`] unpacks to the blocks of [`TextureFormat::unpacked`]. Refused: PVRTC (30 to 33: levels have
/// a minimum size this reader does not know), the 3DS forms of ETC (60, 61), and the ids Unity's enum no longer has
/// (6, 8, 11, 23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextureFormat {
    /// `Alpha8` (1): one 8-bit channel.
    Alpha8,
    /// `ARGB4444` (2): one little-endian 16-bit value, A in the top 4 bits, then R, G, B.
    Argb4444,
    /// `RGB24` (3): three bytes a pixel, R, G, B.
    Rgb24,
    /// `RGBA32` (4): four bytes a pixel, R, G, B, A.
    Rgba32,
    /// `ARGB32` (5): four bytes a pixel, A, R, G, B.
    Argb32,
    /// `RGB565` (7): one 16-bit value, R in the top 5 bits, then G (6), B (5).
    Rgb565,
    /// `R16` (9): one 16-bit unsigned normalised channel.
    R16,
    /// `DXT1` (10): BC1, 4x4 blocks of 8 bytes.
    Dxt1,
    /// `DXT5` (12): BC3, 4x4 blocks of 16 bytes.
    Dxt5,
    /// `RGBA4444` (13): one 16-bit value, R in the top 4 bits, then G, B, A.
    Rgba4444,
    /// `BGRA32` (14): four bytes a pixel, B, G, R, A.
    Bgra32,
    /// `RHalf` (15): one half float.
    RHalf,
    /// `RGHalf` (16): two half floats.
    RgHalf,
    /// `RGBAHalf` (17): four half floats.
    RgbaHalf,
    /// `RFloat` (18): one 32-bit float.
    RFloat,
    /// `RGFloat` (19): two 32-bit floats.
    RgFloat,
    /// `RGBAFloat` (20): four 32-bit floats.
    RgbaFloat,
    /// `YUY2` (21): two pixels in four bytes, Y0, U, Y1, V.
    Yuy2,
    /// `RGB9e5Float` (22): one 32-bit value, R, G, B 9-bit mantissas (R lowest) and a shared 5-bit exponent on top.
    Rgb9e5Float,
    /// `BC6H` (24): 4x4 blocks of 16 bytes, unsigned half floats.
    Bc6h,
    /// `BC7` (25): 4x4 blocks of 16 bytes.
    Bc7,
    /// `BC4` (26): 4x4 blocks of 8 bytes.
    Bc4,
    /// `BC5` (27): 4x4 blocks of 16 bytes.
    Bc5,
    /// `DXT1Crunched` (28): DXT1 blocks in a crunch file.
    Dxt1Crunched,
    /// `DXT5Crunched` (29): DXT5 blocks in a crunch file.
    Dxt5Crunched,
    /// `ETC_RGB4Crunched` (64): ETC1 blocks in a crunch file (not unpacked: [`UnityTexture::levels`] refuses it).
    EtcRgb4Crunched,
    /// `ETC2_RGBA8Crunched` (65): ETC2 RGBA8 blocks in a crunch file (not unpacked).
    Etc2Rgba8Crunched,
    /// `ETC_RGB4` (34): ETC1, 4x4 blocks of 8 bytes.
    EtcRgb4,
    /// `ATC_RGB4` (35): 4x4 blocks of 8 bytes.
    AtcRgb4,
    /// `ATC_RGBA8` (36): 4x4 blocks of 16 bytes.
    AtcRgba8,
    /// `EAC_R` (41): 4x4 blocks of 8 bytes, one unsigned channel.
    EacR,
    /// `EAC_R_SIGNED` (42): 4x4 blocks of 8 bytes, one signed channel.
    EacRSigned,
    /// `EAC_RG` (43): 4x4 blocks of 16 bytes, two unsigned channels.
    EacRg,
    /// `EAC_RG_SIGNED` (44): 4x4 blocks of 16 bytes, two signed channels.
    EacRgSigned,
    /// `ETC2_RGB` (45): 4x4 blocks of 8 bytes.
    Etc2Rgb,
    /// `ETC2_RGBA1` (46): 4x4 blocks of 8 bytes with punch-through alpha.
    Etc2Rgba1,
    /// `ETC2_RGBA8` (47): 4x4 blocks of 16 bytes.
    Etc2Rgba8,
    /// ASTC (48 to 59 `ASTC_RGB_*`/`ASTC_RGBA_*`, 66 to 71 `ASTC_HDR_*`): blocks of `side` x `side` pixels in 16 bytes
    /// (4, 5, 6, 8, 10 or 12), HDR or not.
    Astc {
        /// The block's side in pixels.
        side: u32,
        /// Whether the blocks are of the HDR profile (66 to 71).
        hdr: bool,
    },
    /// `RG16` (62): two 8-bit channels.
    Rg16,
    /// `R8` (63): one 8-bit channel.
    R8,
    /// `RG32` (72): two 16-bit unsigned normalised channels.
    Rg32,
    /// `RGB48` (73): three 16-bit unsigned normalised channels.
    Rgb48,
    /// `RGBA64` (74): four 16-bit unsigned normalised channels.
    Rgba64,
    /// `R8_SIGNED` (75): one 8-bit signed normalised channel.
    R8Signed,
    /// `RG16_SIGNED` (76): two 8-bit signed normalised channels.
    Rg16Signed,
    /// `RGB24_SIGNED` (77): three 8-bit signed normalised channels.
    Rgb24Signed,
    /// `RGBA32_SIGNED` (78): four 8-bit signed normalised channels.
    Rgba32Signed,
    /// `R16_SIGNED` (79): one 16-bit signed normalised channel.
    R16Signed,
    /// `RG32_SIGNED` (80): two 16-bit signed normalised channels.
    Rg32Signed,
    /// `RGB48_SIGNED` (81): three 16-bit signed normalised channels.
    Rgb48Signed,
    /// `RGBA64_SIGNED` (82): four 16-bit signed normalised channels.
    Rgba64Signed,
}

/// The ASTC block sides in the order of Unity's ids (48 + i for RGB, 54 + i for RGBA, 66 + i for HDR).
const ASTC_SIDES: [u32; 6] = [4, 5, 6, 8, 10, 12];

impl TextureFormat {
    /// The format Unity's `TextureFormat` id `id` names.
    pub fn from_id(id: i64) -> Result<TextureFormat> {
        use TextureFormat::*;
        let astc = |first: i64, hdr: bool| {
            let side = id
                .checked_sub(first)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| ASTC_SIDES.get(i).copied())
                .expect("an id of the range");
            Astc { side, hdr }
        };
        Ok(match id {
            1 => Alpha8,
            2 => Argb4444,
            3 => Rgb24,
            4 => Rgba32,
            5 => Argb32,
            7 => Rgb565,
            9 => R16,
            10 => Dxt1,
            12 => Dxt5,
            13 => Rgba4444,
            14 => Bgra32,
            15 => RHalf,
            16 => RgHalf,
            17 => RgbaHalf,
            18 => RFloat,
            19 => RgFloat,
            20 => RgbaFloat,
            21 => Yuy2,
            22 => Rgb9e5Float,
            24 => Bc6h,
            25 => Bc7,
            26 => Bc4,
            27 => Bc5,
            34 => EtcRgb4,
            35 => AtcRgb4,
            36 => AtcRgba8,
            41 => EacR,
            42 => EacRSigned,
            43 => EacRg,
            44 => EacRgSigned,
            45 => Etc2Rgb,
            46 => Etc2Rgba1,
            47 => Etc2Rgba8,
            48..=53 => astc(48, false),
            54..=59 => astc(54, false),
            62 => Rg16,
            63 => R8,
            66..=71 => astc(66, true),
            72 => Rg32,
            73 => Rgb48,
            74 => Rgba64,
            75 => R8Signed,
            76 => Rg16Signed,
            77 => Rgb24Signed,
            78 => Rgba32Signed,
            79 => R16Signed,
            80 => Rg32Signed,
            81 => Rgb48Signed,
            82 => Rgba64Signed,
            28 => Dxt1Crunched,
            29 => Dxt5Crunched,
            64 => EtcRgb4Crunched,
            65 => Etc2Rgba8Crunched,
            30..=33 => return Err(Error::Unsupported(format!("PVRTC texture format {id}"))),
            id => return Err(Error::Unsupported(format!("texture format {id}"))),
        })
    }

    /// The format of the blocks a crunched format unpacks to; another format itself.
    pub fn unpacked(self) -> TextureFormat {
        match self {
            TextureFormat::Dxt1Crunched => TextureFormat::Dxt1,
            TextureFormat::Dxt5Crunched => TextureFormat::Dxt5,
            TextureFormat::EtcRgb4Crunched => TextureFormat::EtcRgb4,
            TextureFormat::Etc2Rgba8Crunched => TextureFormat::Etc2Rgba8,
            other => other,
        }
    }

    /// Whether the data is a crunch file.
    pub fn is_crunched(self) -> bool {
        self.unpacked() != self
    }

    /// The width and height in pixels of a block and its size in bytes (one pixel for the uncompressed formats; a
    /// crunched format's are those of the blocks it unpacks to).
    fn block(self) -> (u32, u32, usize) {
        use TextureFormat::*;
        match self {
            Alpha8 | R8 | R8Signed => (1, 1, 1),
            Argb4444 | Rgb565 | R16 | Rgba4444 | RHalf | Rg16 | Rg16Signed | R16Signed => (1, 1, 2),
            Rgb24 | Rgb24Signed => (1, 1, 3),
            Rgba32 | Argb32 | Bgra32 | RgHalf | RFloat | Rgb9e5Float | Rg32 | Rgba32Signed | Rg32Signed => {
                (1, 1, 4)
            }
            Rgb48 | Rgb48Signed => (1, 1, 6),
            RgbaHalf | RgFloat | Rgba64 | Rgba64Signed => (1, 1, 8),
            RgbaFloat => (1, 1, 16),
            Yuy2 => (2, 1, 4),
            Dxt1 | Bc4 | EtcRgb4 | AtcRgb4 | EacR | EacRSigned | Etc2Rgb | Etc2Rgba1 => (4, 4, 8),
            Dxt5 | Bc6h | Bc7 | Bc5 | AtcRgba8 | EacRg | EacRgSigned | Etc2Rgba8 => (4, 4, 16),
            Astc { side, .. } => (side, side, 16),
            Dxt1Crunched | EtcRgb4Crunched => (4, 4, 8),
            Dxt5Crunched | Etc2Rgba8Crunched => (4, 4, 16),
        }
    }

    /// The bytes of a `width` x `height` level as [`UnityTexture::levels`] gives it, whole blocks rounded up; `None`
    /// when the size does not fit in a `usize` (the dimensions are the file's to state).
    pub fn level_size(self, width: u32, height: u32) -> Option<usize> {
        let (block_width, block_height, bytes) = self.block();
        let blocks_x = usize::try_from(width.div_ceil(block_width).max(1)).ok()?;
        let blocks_y = usize::try_from(height.div_ceil(block_height).max(1)).ok()?;
        blocks_x.checked_mul(blocks_y)?.checked_mul(bytes)
    }
}

impl UnityTexture {
    /// Read the Texture2D `r` of `player` (its pixels in the object or streamed beside its file).
    pub fn read(player: &Player, r: &ObjRef) -> Result<Self> {
        let t = player.decode(r)?;
        let settings = t
            .get("m_TextureSettings")
            .ok_or_else(|| invalid("texture: no m_TextureSettings"))?;
        let stream = t
            .get("m_StreamData")
            .map(StreamingInfo::of)
            .transpose()?
            .flatten();
        let data = match stream {
            Some(s) => player.read_stream(&r.file, &s)?,
            None => t
                .get("image data")
                .and_then(Value::as_bytes)
                .ok_or_else(|| invalid("texture: no image data"))?
                .to_vec(),
        };
        Ok(UnityTexture {
            name: t.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned(),
            width: dimension(&t, "m_Width")?,
            height: dimension(&t, "m_Height")?,
            format: int(&t, "m_TextureFormat")?,
            mips: dimension(&t, "m_MipCount")?,
            srgb: int(&t, "m_ColorSpace")? == 1,
            usage: int(&t, "m_LightmapFormat")?,
            wrap: [int(settings, "m_WrapU")?, int(settings, "m_WrapV")?],
            filter: int(settings, "m_FilterMode")?,
            mip_bias: float(settings, "m_MipBias")?,
            aniso: int(settings, "m_Aniso")?,
            data,
            unity: player.file(&r.file)?.unity,
        })
    }

    /// The mip chain split into its levels, largest first, each as stored (bottom row first), with the format of their
    /// bytes: the one the texture id names, or for a crunched format the one its crunch file unpacks to, each level
    /// unpacked. Every size is checked: no more levels than halvings of a 32-bit side, no level whose size overflows or
    /// runs past the data, no data left over; a crunch file of the texture's size and levels. Refused: a crunched
    /// texture of Unity before 2017.3 (crunch's original format) or of an ETC format. An empty texture (a side of 0 or
    /// no levels: a dynamic font's `Font Texture` before the game draws glyphs into it) has no levels and must hold no
    /// bytes.
    pub fn levels(&self) -> Result<(TextureFormat, Vec<Cow<'_, [u8]>>)> {
        let format = TextureFormat::from_id(self.format)?;
        if self.width == 0 || self.height == 0 || self.mips == 0 {
            if !self.data.is_empty() {
                return Err(invalid(format!(
                    "{}: {} bytes of pixels for an empty texture ({}x{}, {} levels)",
                    self.name,
                    self.data.len(),
                    self.width,
                    self.height,
                    self.mips
                )));
            }
            return Ok((format.unpacked(), Vec::new()));
        }
        // no more levels than halvings of a 32-bit side
        if self.mips > 32 {
            return Err(invalid(format!("{}: {} mip levels", self.name, self.mips)));
        }
        if format.is_crunched() {
            return self.crunched(format);
        }
        let mut levels = Vec::with_capacity(self.mips as usize);
        let mut at = 0usize;
        for i in 0..self.mips {
            let (w, h) = ((self.width >> i).max(1), (self.height >> i).max(1));
            let size = format.level_size(w, h).ok_or_else(|| {
                invalid(format!(
                    "{}: level {i} of {w}x{h} does not fit in memory",
                    self.name
                ))
            })?;
            let bytes = at
                .checked_add(size)
                .and_then(|end| self.data.get(at..end))
                .ok_or_else(|| {
                    invalid(format!(
                        "{}: level {i} past the {} bytes of pixels",
                        self.name,
                        self.data.len()
                    ))
                })?;
            levels.push(Cow::Borrowed(bytes));
            at = at.saturating_add(size);
        }
        if at != self.data.len() {
            return Err(invalid(format!(
                "{}: {} levels use {at} of {} bytes",
                self.name,
                self.mips,
                self.data.len()
            )));
        }
        Ok((format, levels))
    }

    /// The levels of a crunched texture, unpacked from its crunch file.
    fn crunched(&self, format: TextureFormat) -> Result<(TextureFormat, Vec<Cow<'_, [u8]>>)> {
        let name = &self.name;
        if self.unity < UNITY_CRUNCH {
            return Err(Error::Unsupported(format!(
                "{name}: a crunched texture of Unity {} (crunch's original format, before 2017.3)",
                self.unity
            )));
        }
        let header = CrunchHeader::parse(&self.data).map_err(|e| invalid(format!("{name}: {e}")))?;
        let expected = match format {
            TextureFormat::Dxt1Crunched => CrunchFormat::Dxt1,
            TextureFormat::Dxt5Crunched => CrunchFormat::Dxt5,
            _ => {
                return Err(Error::Unsupported(format!(
                    "{name}: crunched texture format {} (ETC)",
                    self.format
                )));
            }
        };
        if header.format != expected
            || (header.width, header.height) != (self.width, self.height)
            || header.levels != self.mips as usize
            || header.faces != 1
        {
            return Err(invalid(format!(
                "{name}: a crunch file of {:?} {}x{}, {} levels and {} faces for a {}x{} texture of {} levels",
                header.format,
                header.width,
                header.height,
                header.levels,
                header.faces,
                self.width,
                self.height,
                self.mips
            )));
        }
        let unpacked = format.unpacked();
        let mut levels = Vec::with_capacity(header.levels);
        for level in 0..header.levels {
            let bytes =
                unpack_level(&self.data, &header, level).map_err(|e| invalid(format!("{name}: {e}")))?;
            let shift = u32::try_from(level).unwrap_or(u32::MAX);
            let (w, h) = (
                self.width.checked_shr(shift).unwrap_or(0).max(1),
                self.height.checked_shr(shift).unwrap_or(0).max(1),
            );
            if unpacked.level_size(w, h) != Some(bytes.len()) {
                return Err(invalid(format!(
                    "{name}: crunch level {level} unpacked to {} bytes",
                    bytes.len()
                )));
            }
            levels.push(Cow::Owned(bytes));
        }
        Ok((unpacked, levels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every id names the format whose bytes Unity's documentation gives it, sized by its blocks: the per-pixel formats
    /// by their pixel, YUY2 by pixel pairs, the 4x4 families and ASTC by their blocks, whole blocks rounded up.
    #[test]
    fn each_format_is_sized_by_its_blocks() {
        let size = |id: i64, w: u32, h: u32| TextureFormat::from_id(id).unwrap().level_size(w, h).unwrap();
        for (id, bytes) in [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 4),
            (7, 2),
            (9, 2),
            (13, 2),
            (14, 4),
            (15, 2),
            (16, 4),
            (17, 8),
            (18, 4),
            (19, 8),
            (20, 16),
            (22, 4),
            (62, 2),
            (63, 1),
            (72, 4),
            (73, 6),
            (74, 8),
            (75, 1),
            (76, 2),
            (77, 3),
            (78, 4),
            (79, 2),
            (80, 4),
            (81, 6),
            (82, 8),
        ] {
            assert_eq!(size(id, 3, 5), 15 * bytes, "format {id}");
        }
        // YUY2: a pair of pixels in four bytes, an odd width rounded up to a pair
        assert_eq!(size(21, 3, 5), 2 * 5 * 4);
        for id in [10, 26, 34, 35, 41, 42, 45, 46] {
            assert_eq!(size(id, 5, 9), 2 * 3 * 8, "format {id}");
        }
        for id in [12, 24, 25, 27, 36, 43, 44, 47] {
            assert_eq!(size(id, 5, 9), 2 * 3 * 16, "format {id}");
        }
        for (i, side) in ASTC_SIDES.into_iter().enumerate() {
            for (first, hdr) in [(48, false), (54, false), (66, true)] {
                let id = first + i as i64;
                assert_eq!(
                    TextureFormat::from_id(id).unwrap(),
                    TextureFormat::Astc { side, hdr }
                );
                // a 13 x 1 level: whole blocks across, one down
                assert_eq!(
                    size(id, 13, 1),
                    13_usize.div_ceil(side as usize) * 16,
                    "format {id}"
                );
            }
        }
    }

    #[test]
    fn pvrtc_and_retired_ids_are_refused() {
        for (id, why) in [
            (30, "PVRTC"),
            (33, "PVRTC"),
            (6, "texture format 6"),
            (11, "texture format 11"),
            (60, "texture format 60"),
            (83, "texture format 83"),
        ] {
            let e = TextureFormat::from_id(id).unwrap_err();
            assert!(
                matches!(e, Error::Unsupported(_)) && e.to_string().contains(why),
                "{id}: {e}"
            );
        }
    }

    /// The crunched ids name formats sized as the blocks they unpack to; a crunched texture of Unity before 2017.3
    /// (crunch's original format) and an ETC one are refused by their levels.
    #[test]
    fn crunched_formats_unpack_to_their_blocks() {
        for (id, unpacked) in [
            (28, TextureFormat::Dxt1),
            (29, TextureFormat::Dxt5),
            (64, TextureFormat::EtcRgb4),
            (65, TextureFormat::Etc2Rgba8),
        ] {
            let f = TextureFormat::from_id(id).unwrap();
            assert!(f.is_crunched() && f.unpacked() == unpacked, "{id}");
            assert_eq!(f.level_size(5, 9), unpacked.level_size(5, 9), "{id}");
        }
        let texture = |format: i64, unity: &str| UnityTexture {
            name: "t".into(),
            width: 4,
            height: 4,
            format,
            mips: 1,
            srgb: false,
            usage: 0,
            wrap: [0, 0],
            filter: 1,
            mip_bias: 0.0,
            aniso: 1,
            data: vec![0; 74],
            unity: UnityVersion::parse(unity).unwrap(),
        };
        let e = texture(29, "2017.2.5f1").levels().unwrap_err();
        assert!(
            matches!(e, Error::Unsupported(_)) && e.to_string().contains("original format"),
            "{e}"
        );
        // a crunch file that is no crunch file: refused as damaged
        let e = texture(29, "2019.4.20f1").levels().unwrap_err();
        assert!(
            matches!(e, Error::Invalid(_)) || e.to_string().contains("signature"),
            "{e}"
        );
    }

    /// A texture of no pixels (a dynamic font's 0x0 `Font Texture`, one level stated) has no levels; one that holds
    /// bytes anyway is damaged.
    #[test]
    fn an_empty_texture_has_no_levels() {
        let empty = |data: Vec<u8>| UnityTexture {
            name: "Font Texture".into(),
            width: 0,
            height: 0,
            format: 1,
            mips: 1,
            srgb: false,
            usage: 0,
            wrap: [1, 1],
            filter: 1,
            mip_bias: 0.0,
            aniso: 1,
            data,
            unity: UnityVersion::parse("2021.3.45f2").unwrap(),
        };
        let t = empty(Vec::new());
        let (format, levels) = t.levels().unwrap();
        assert_eq!((format, levels.len()), (TextureFormat::Alpha8, 0));
        let e = empty(vec![0; 4]).levels().unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e}");
    }
}
