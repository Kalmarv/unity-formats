//! Crunch (`.crn`) as Unity stores a crunched texture (TextureFormat 28 and 29, Unity 2017.3 on): a header, Huffman
//! coded palettes of colour and alpha endpoints and selectors, and per mip level a stream of block references that
//! unpacks to DXT blocks. Only the DXT family is unpacked (DXT1, DXT5 and its swizzles, DXT5A, DXN); the ETC forms
//! (TextureFormat 64 and 65) and the original crunch format of Unity before 2017.3 are refused.
//!
//! Written from the decoder of Unity's crunch fork (`crn_decomp.h`, Unity-Technologies/crunch, zlib licence) as
//! texture2ddecoder 0.1.2 ports it to Rust (`src/unitycrunch`, UniversalGameExtraction/texture2ddecoder, MIT or
//! Apache-2.0): the stream layout, the Huffman code-length coding, the palette deltas and the block reference scheme
//! are theirs. Every read here is checked, so a damaged stream is an error naming what broke rather than a panic, and
//! the bit reader gives zero bits past the end of its stream as `crn_decomp.h` does.

use crate::{Result, invalid};

/// The signature a crunch file starts with: `Hx`.
const SIGNATURE: u16 = 0x4878;
/// The bytes of the fixed header, before the level offsets.
const HEADER_BYTES: usize = 70;
/// The most levels a file holds.
const MAX_LEVELS: usize = 16;
/// The most symbols a Huffman model codes.
const MAX_SYMBOLS: usize = 8192;
/// The longest code.
const MAX_CODE_SIZE: usize = 16;
/// The code-length codes, in the order their sizes are sent.
const CODE_LENGTH_ORDER: [usize; 21] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15, 16,
];
/// A DXT5 alpha selector's value from its linear rank.
const DXT5_FROM_LINEAR: [u32; 8] = [0, 2, 3, 4, 5, 6, 7, 1];

/// The block format a crunch file unpacks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrunchFormat {
    /// DXT1 (BC1): 8 bytes a block.
    Dxt1,
    /// DXT5 (BC3) or one of its swizzles: 16 bytes a block.
    Dxt5,
    /// DXT5A (BC4): 8 bytes a block.
    Dxt5a,
    /// DXN (BC5): 16 bytes a block.
    Dxn,
}

impl CrunchFormat {
    /// The bytes of one 4x4 block.
    pub fn block_bytes(self) -> usize {
        match self {
            CrunchFormat::Dxt1 | CrunchFormat::Dxt5a => 8,
            CrunchFormat::Dxt5 | CrunchFormat::Dxn => 16,
        }
    }
}

/// Where a palette's stream is and how many entries it holds.
#[derive(Clone, Copy, Debug, Default)]
struct Palette {
    offset: usize,
    size: usize,
    count: usize,
}

/// A crunch file's header.
#[derive(Clone, Debug)]
pub struct CrunchHeader {
    /// Width in pixels of the largest level.
    pub width: u32,
    /// Height in pixels of the largest level.
    pub height: u32,
    /// The levels it holds.
    pub levels: usize,
    /// The faces (6 for a cube map).
    pub faces: usize,
    /// The block format it unpacks to.
    pub format: CrunchFormat,
    color_endpoints: Palette,
    color_selectors: Palette,
    alpha_endpoints: Palette,
    alpha_selectors: Palette,
    tables: Palette,
    level_offsets: Vec<usize>,
    data_size: usize,
}

fn be(data: &[u8], at: usize, len: usize) -> Result<usize> {
    let bytes = at
        .checked_add(len)
        .and_then(|end| data.get(at..end))
        .ok_or_else(|| {
            invalid(format!(
                "crunch: header field at {at} past the {} bytes",
                data.len()
            ))
        })?;
    Ok(bytes.iter().fold(0usize, |v, &b| (v << 8) | usize::from(b)))
}

impl CrunchHeader {
    /// The header at the start of `data`; its sizes must lie within `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if be(data, 0, 2)? != usize::from(SIGNATURE) {
            return Err(invalid("crunch: no Hx signature"));
        }
        let header_size = be(data, 2, 2)?;
        let data_size = be(data, 6, 4)?;
        if header_size < HEADER_BYTES || data_size > data.len() || header_size > data_size {
            return Err(invalid(format!(
                "crunch: header of {header_size} bytes, data of {data_size}, in {} bytes",
                data.len()
            )));
        }
        let levels = be(data, 16, 1)?;
        if levels == 0 || levels > MAX_LEVELS {
            return Err(invalid(format!("crunch: {levels} levels")));
        }
        let format = match be(data, 18, 1)? {
            0 => CrunchFormat::Dxt1,
            // DXT5 and its swizzles: luma-chroma, xGxR, xGBR, AGBR
            2..=6 => CrunchFormat::Dxt5,
            7 | 8 => CrunchFormat::Dxn,
            9 => CrunchFormat::Dxt5a,
            f @ 10..=14 => {
                return Err(crate::Error::Unsupported(format!("crunch: ETC format {f}")));
            }
            f => return Err(invalid(format!("crunch: format {f}"))),
        };
        let palette = |at: usize| -> Result<Palette> {
            let p = Palette {
                offset: be(data, at, 3)?,
                size: be(data, at.saturating_add(3), 3)?,
                count: be(data, at.saturating_add(6), 2)?,
            };
            if p.offset.checked_add(p.size).is_none_or(|end| end > data_size) {
                return Err(invalid(format!(
                    "crunch: a stream at {}+{} past the data",
                    p.offset, p.size
                )));
            }
            Ok(p)
        };
        let tables = Palette {
            offset: be(data, 67, 3)?,
            size: be(data, 65, 2)?,
            count: 0,
        };
        if tables
            .offset
            .checked_add(tables.size)
            .is_none_or(|end| end > data_size)
        {
            return Err(invalid("crunch: the tables past the data"));
        }
        let level_offsets = (0..levels)
            .map(|i| be(data, HEADER_BYTES.saturating_add(i.saturating_mul(4)), 4))
            .collect::<Result<Vec<_>>>()?;
        Ok(CrunchHeader {
            width: u32::try_from(be(data, 12, 2)?).map_err(|_| invalid("crunch: width"))?,
            height: u32::try_from(be(data, 14, 2)?).map_err(|_| invalid("crunch: height"))?,
            levels,
            faces: be(data, 17, 1)?.max(1),
            format,
            color_endpoints: palette(33)?,
            color_selectors: palette(41)?,
            alpha_endpoints: palette(49)?,
            alpha_selectors: palette(57)?,
            tables,
            level_offsets,
            data_size,
        })
    }
}

/// Bits of a stream, most significant first; past its end, zeros.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    buffer: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits {
            data,
            at: 0,
            buffer: 0,
            count: 0,
        }
    }

    fn of(file: &'a [u8], offset: usize, size: usize, what: &str) -> Result<Self> {
        let stream = offset
            .checked_add(size)
            .and_then(|end| file.get(offset..end))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid(format!("crunch: the {what} stream at {offset}+{size}")))?;
        Ok(Bits::new(stream))
    }

    /// The next `n` bits (at most 32).
    fn bits(&mut self, n: u32) -> u32 {
        while self.count < n {
            let byte = self.data.get(self.at).copied().unwrap_or(0);
            self.at = self.at.saturating_add(1);
            self.buffer = (self.buffer << 8) | u64::from(byte);
            self.count = self.count.saturating_add(8);
        }
        let rest = self.count.saturating_sub(n);
        let value = (self.buffer >> rest) & (1u64 << n).wrapping_sub(1);
        self.count = rest;
        self.buffer &= (1u64 << rest).wrapping_sub(1);
        u32::try_from(value).unwrap_or(u32::MAX)
    }

    /// The next symbol of `model`.
    fn symbol(&mut self, model: &Model) -> Result<u32> {
        let mut code = 0u32;
        for size in 1..=MAX_CODE_SIZE {
            code = (code << 1) | self.bits(1);
            let Some(&(first, start, count)) = model.by_size.get(size) else {
                break;
            };
            if let Some(i) = code.checked_sub(first).filter(|&i| i < count) {
                let at = usize::try_from(start.saturating_add(i)).unwrap_or(usize::MAX);
                return model
                    .sorted
                    .get(at)
                    .copied()
                    .ok_or_else(|| invalid("crunch: a Huffman code past its symbols"));
            }
        }
        Err(invalid("crunch: a bit string that is no Huffman code"))
    }

    /// A Huffman model as sent: how many symbols, the sizes of the code-length codes, then each symbol's code size
    /// coded by them with runs of zeros and repeats.
    fn model(&mut self) -> Result<Model> {
        // total_bits(8192): 14
        let symbols = usize::try_from(self.bits(14)).unwrap_or(usize::MAX);
        if symbols == 0 || symbols > MAX_SYMBOLS {
            return Err(invalid(format!("crunch: a Huffman model of {symbols} symbols")));
        }
        let sent = usize::try_from(self.bits(5)).unwrap_or(usize::MAX);
        if sent == 0 || sent > CODE_LENGTH_ORDER.len() {
            return Err(invalid(format!("crunch: {sent} code-length code sizes")));
        }
        let mut lengths = [0u8; 21];
        for &code in CODE_LENGTH_ORDER.iter().take(sent) {
            if let Some(slot) = lengths.get_mut(code) {
                *slot = u8::try_from(self.bits(3)).unwrap_or(0);
            }
        }
        let lengths = Model::new(&lengths)?;
        let mut sizes = vec![0u8; symbols];
        let mut at = 0usize;
        while at < symbols {
            let code = self.symbol(&lengths)?;
            let left = symbols.saturating_sub(at);
            let run = |bits: &mut Self, extra: u32, least: u32| -> Result<usize> {
                let n = usize::try_from(bits.bits(extra).saturating_add(least)).unwrap_or(usize::MAX);
                if n > left {
                    return Err(invalid("crunch: a code-size run past the symbols"));
                }
                Ok(n)
            };
            match code {
                0..=16 => {
                    if let Some(s) = sizes.get_mut(at) {
                        *s = u8::try_from(code).unwrap_or(0);
                    }
                    at = at.saturating_add(1);
                }
                17 => at = at.saturating_add(run(self, 3, 3)?),
                18 => at = at.saturating_add(run(self, 7, 11)?),
                19 | 20 => {
                    let n = if code == 19 {
                        run(self, 2, 3)?
                    } else {
                        run(self, 6, 7)?
                    };
                    let previous = at
                        .checked_sub(1)
                        .and_then(|p| sizes.get(p))
                        .copied()
                        .filter(|&p| p != 0)
                        .ok_or_else(|| invalid("crunch: a repeat of no code size"))?;
                    let end = at.saturating_add(n);
                    for s in sizes.get_mut(at..end).into_iter().flatten() {
                        *s = previous;
                    }
                    at = end;
                }
                c => return Err(invalid(format!("crunch: code-length code {c}"))),
            }
        }
        Model::new(&sizes)
    }
}

/// A canonical Huffman code: per code size the first code, where its symbols start in `sorted` and how many there are.
#[derive(Clone, Debug, Default)]
struct Model {
    by_size: Vec<(u32, u32, u32)>,
    sorted: Vec<u32>,
}

impl Model {
    /// The code of `sizes` (a symbol's code size, 0 for an unused symbol): codes counted up through the sizes, the
    /// symbols of a size in their order, as crunch assigns them.
    fn new(sizes: &[u8]) -> Result<Model> {
        let mut counts = [0u32; MAX_CODE_SIZE + 1];
        for &s in sizes {
            if usize::from(s) > MAX_CODE_SIZE {
                return Err(invalid(format!("crunch: a code of {s} bits")));
            }
            if let Some(c) = counts.get_mut(usize::from(s)) {
                *c = c.saturating_add(1);
            }
        }
        counts[0] = 0;
        let mut by_size = vec![(0u32, 0u32, 0u32); MAX_CODE_SIZE + 1];
        let (mut code, mut start) = (0u32, 0u32);
        for size in 1..=MAX_CODE_SIZE {
            let n = counts.get(size).copied().unwrap_or(0);
            if let Some(slot) = by_size.get_mut(size) {
                *slot = (code, start, n);
            }
            code = code.saturating_add(n);
            if code > 1u32 << size {
                return Err(invalid("crunch: an over-subscribed Huffman code"));
            }
            start = start.saturating_add(n);
            code <<= 1;
        }
        if start == 0 {
            return Err(invalid("crunch: a Huffman code of no symbols"));
        }
        let mut sorted = Vec::with_capacity(usize::try_from(start).unwrap_or(0));
        for size in 1..=MAX_CODE_SIZE {
            for (symbol, &s) in sizes.iter().enumerate() {
                if usize::from(s) == size {
                    sorted.push(u32::try_from(symbol).map_err(|_| invalid("crunch: symbol"))?);
                }
            }
        }
        Ok(Model { by_size, sorted })
    }
}

/// The decoded palettes and the models the level streams use.
struct Palettes {
    color_endpoints: Vec<u32>,
    color_selectors: Vec<u32>,
    alpha_endpoints: Vec<u16>,
    /// Three 16-bit words a selector.
    alpha_selectors: Vec<u16>,
    reference: Model,
    /// Colour, alpha.
    endpoint_delta: [Model; 2],
    selector_delta: [Model; 2],
}

fn palettes(data: &[u8], h: &CrunchHeader) -> Result<Palettes> {
    let mut tables = Bits::of(data, h.tables.offset, h.tables.size, "tables")?;
    let reference = tables.model()?;
    let (has_color, has_alpha) = (h.color_endpoints.count > 0, h.alpha_endpoints.count > 0);
    if !has_color && !has_alpha {
        return Err(invalid("crunch: neither colour nor alpha endpoints"));
    }
    let mut endpoint_delta = [Model::default(), Model::default()];
    let mut selector_delta = [Model::default(), Model::default()];
    for (which, present) in [(0, has_color), (1, has_alpha)] {
        if present
            && let (Some(endpoint), Some(selector)) =
                (endpoint_delta.get_mut(which), selector_delta.get_mut(which))
        {
            *endpoint = tables.model()?;
            *selector = tables.model()?;
        }
    }
    let mut p = Palettes {
        color_endpoints: Vec::new(),
        color_selectors: Vec::new(),
        alpha_endpoints: Vec::new(),
        alpha_selectors: Vec::new(),
        reference,
        endpoint_delta,
        selector_delta,
    };
    if has_color {
        // the endpoints: deltas of 5:6:5 colours, the 5-bit and 6-bit components each by their own model
        let e = h.color_endpoints;
        let mut bits = Bits::of(data, e.offset, e.size, "colour endpoint")?;
        let (five, six) = (bits.model()?, bits.model()?);
        let mut c = [0u32; 6];
        for _ in 0..e.count {
            for (i, (model, mask)) in [
                (&five, 31),
                (&six, 63),
                (&five, 31),
                (&five, 31),
                (&six, 63),
                (&five, 31),
            ]
            .into_iter()
            .enumerate()
            {
                if let Some(v) = c.get_mut(i) {
                    *v = v.wrapping_add(bits.symbol(model)?) & mask;
                }
            }
            let [a, b, cc, d, ee, f] = c;
            p.color_endpoints
                .push(cc | (b << 5) | (a << 11) | (f << 16) | (ee << 21) | (d << 27));
        }
        // the selectors: 2-bit linear ranks xor-chained, mapped to DXT1's order
        let s = h.color_selectors;
        let mut bits = Bits::of(data, s.offset, s.size, "colour selector")?;
        let model = bits.model()?;
        let mut linear = 0u32;
        for _ in 0..s.count {
            for shift in (0..32).step_by(4) {
                linear ^= bits.symbol(&model)? << shift;
            }
            p.color_selectors
                .push(((linear ^ (linear << 1)) & 0xAAAA_AAAA) | ((linear >> 1) & 0x5555_5555));
        }
    }
    if has_alpha {
        let e = h.alpha_endpoints;
        let mut bits = Bits::of(data, e.offset, e.size, "alpha endpoint")?;
        let model = bits.model()?;
        let (mut a, mut b) = (0u32, 0u32);
        for _ in 0..e.count {
            a = a.wrapping_add(bits.symbol(&model)?) & 0xFF;
            b = b.wrapping_add(bits.symbol(&model)?) & 0xFF;
            p.alpha_endpoints.push(u16::try_from(a | (b << 8)).unwrap_or(0));
        }
        // the selectors: 3-bit linear ranks, two to a symbol, xor-chained, mapped to DXT5's order
        let s = h.alpha_selectors;
        let mut bits = Bits::of(data, s.offset, s.size, "alpha selector")?;
        let model = bits.model()?;
        let from_linear = |six: u32| -> u32 {
            let (low, high) = (
                usize::try_from(six & 7).unwrap_or(0),
                usize::try_from((six >> 3) & 7).unwrap_or(0),
            );
            DXT5_FROM_LINEAR.get(low).copied().unwrap_or(0)
                | (DXT5_FROM_LINEAR.get(high).copied().unwrap_or(0) << 3)
        };
        let (mut l0, mut l1) = (0u32, 0u32);
        for _ in 0..s.count {
            let (mut s0, mut s1) = (0u32, 0u32);
            for shift in [0, 6, 12, 18] {
                l0 ^= bits.symbol(&model)? << shift;
                s0 |= from_linear((l0 >> shift) & 0x3F) << shift;
            }
            for shift in [0, 6, 12, 18] {
                l1 ^= bits.symbol(&model)? << shift;
                s1 |= from_linear((l1 >> shift) & 0x3F) << shift;
            }
            p.alpha_selectors.push((s0 & 0xFFFF) as u16);
            p.alpha_selectors.push((((s0 >> 16) | (s1 << 8)) & 0xFFFF) as u16);
            p.alpha_selectors.push(((s1 >> 8) & 0xFFFF) as u16);
        }
    }
    Ok(p)
}

/// The endpoint indices a block column carries to the block below it.
#[derive(Clone, Copy, Debug, Default)]
struct Column {
    reference: u32,
    color: usize,
    alpha0: usize,
    alpha1: usize,
}

/// Step an index through a palette of `n` entries by a coded delta.
fn step(index: usize, delta: u32, n: usize, what: &str) -> Result<usize> {
    let next = index.saturating_add(usize::try_from(delta).unwrap_or(usize::MAX));
    let next = if next >= n { next.saturating_sub(n) } else { next };
    if next >= n {
        return Err(invalid(format!(
            "crunch: a {what} endpoint past the {n} of the palette"
        )));
    }
    Ok(next)
}

/// Unpack level `level` of the crunch file `data` to its DXT blocks, rows of blocks top first as stored: blocks of
/// `header.format`, `ceil(w / 4) * ceil(h / 4)` of them for each face, the faces one after the other.
pub fn unpack_level(data: &[u8], header: &CrunchHeader, level: usize) -> Result<Vec<u8>> {
    let h = header;
    let start = *h
        .level_offsets
        .get(level)
        .ok_or_else(|| invalid(format!("crunch: level {level} of {}", h.levels)))?;
    let end = h
        .level_offsets
        .get(level.saturating_add(1))
        .copied()
        .unwrap_or(h.data_size);
    if end <= start || end > h.data_size {
        return Err(invalid(format!("crunch: level {level} at {start}..{end}")));
    }
    let p = palettes(data, h)?;
    let mut bits = Bits::of(data, start, end.saturating_sub(start), "level")?;
    let width = (h.width >> level).max(1);
    let height = (h.height >> level).max(1);
    let blocks_x = usize::try_from(width.div_ceil(4)).map_err(|_| invalid("crunch: width"))?;
    let blocks_y = usize::try_from(height.div_ceil(4)).map_err(|_| invalid("crunch: height"))?;
    let block = h.format.block_bytes();
    let face_bytes = blocks_x
        .checked_mul(blocks_y)
        .and_then(|n| n.checked_mul(block))
        .ok_or_else(|| invalid("crunch: a level too large"))?;
    let mut out = vec![
        0u8;
        face_bytes
            .checked_mul(h.faces)
            .ok_or_else(|| invalid("crunch: a level too large"))?
    ];
    // blocks come in 2x2 groups, so a level's grid is rounded up to even sides; the blocks past it are decoded and
    // dropped
    let (grid_x, grid_y) = (blocks_x.saturating_add(1) & !1, blocks_y.saturating_add(1) & !1);
    let n_color = p.color_endpoints.len();
    let n_alpha = p.alpha_endpoints.len();
    let (mut color, mut alpha0, mut alpha1) = (0usize, 0usize, 0usize);
    let mut columns = vec![Column::default(); grid_x];
    let mut group = 0u32;
    for face in 0..h.faces {
        for y in 0..grid_y {
            for x in 0..grid_x {
                if y & 1 == 0 && x & 1 == 0 {
                    group = bits.symbol(&p.reference)?;
                }
                let column = columns
                    .get_mut(x)
                    .ok_or_else(|| invalid("crunch: block column"))?;
                let reference = if y & 1 == 1 {
                    column.reference
                } else {
                    let r = group & 3;
                    group >>= 2;
                    column.reference = group & 3;
                    group >>= 2;
                    r
                };
                let (uses_color, uses_alpha0, uses_alpha1) = match h.format {
                    CrunchFormat::Dxt1 => (true, false, false),
                    CrunchFormat::Dxt5 => (true, true, false),
                    CrunchFormat::Dxt5a => (false, true, false),
                    CrunchFormat::Dxn => (false, true, true),
                };
                match reference {
                    0 => {
                        if uses_color {
                            color = step(color, bits.symbol(&p.endpoint_delta[0])?, n_color, "colour")?;
                            column.color = color;
                        }
                        if uses_alpha0 {
                            alpha0 = step(alpha0, bits.symbol(&p.endpoint_delta[1])?, n_alpha, "alpha")?;
                            column.alpha0 = alpha0;
                        }
                        if uses_alpha1 {
                            alpha1 = step(alpha1, bits.symbol(&p.endpoint_delta[1])?, n_alpha, "alpha")?;
                            column.alpha1 = alpha1;
                        }
                    }
                    1 => {
                        column.color = color;
                        column.alpha0 = alpha0;
                        column.alpha1 = alpha1;
                    }
                    _ => {
                        color = column.color;
                        alpha0 = column.alpha0;
                        alpha1 = column.alpha1;
                    }
                }
                let mut words: Vec<u32> = Vec::with_capacity(4);
                let alpha_words = |endpoint: usize, selector: u32, words: &mut Vec<u32>| -> Result<()> {
                    let at = usize::try_from(selector).unwrap_or(usize::MAX).saturating_mul(3);
                    let s = at
                        .checked_add(3)
                        .and_then(|end| p.alpha_selectors.get(at..end))
                        .ok_or_else(|| invalid("crunch: an alpha selector past the palette"))?;
                    let e = p
                        .alpha_endpoints
                        .get(endpoint)
                        .copied()
                        .ok_or_else(|| invalid("crunch: an alpha endpoint past the palette"))?;
                    let (s0, s1, s2) = (s.first(), s.get(1), s.get(2));
                    let (s0, s1, s2) = (
                        u32::from(*s0.unwrap_or(&0)),
                        u32::from(*s1.unwrap_or(&0)),
                        u32::from(*s2.unwrap_or(&0)),
                    );
                    words.push(u32::from(e) | (s0 << 16));
                    words.push(s1 | (s2 << 16));
                    Ok(())
                };
                let color_selector = if uses_color {
                    Some(bits.symbol(&p.selector_delta[0])?)
                } else {
                    None
                };
                let alpha0_selector = if uses_alpha0 {
                    Some(bits.symbol(&p.selector_delta[1])?)
                } else {
                    None
                };
                let alpha1_selector = if uses_alpha1 {
                    Some(bits.symbol(&p.selector_delta[1])?)
                } else {
                    None
                };
                if x >= blocks_x || y >= blocks_y {
                    continue;
                }
                if let Some(s) = alpha0_selector {
                    alpha_words(alpha0, s, &mut words)?;
                }
                if let Some(s) = alpha1_selector {
                    alpha_words(alpha1, s, &mut words)?;
                }
                if let Some(s) = color_selector {
                    let e = p
                        .color_endpoints
                        .get(color)
                        .copied()
                        .ok_or_else(|| invalid("crunch: a colour endpoint past the palette"))?;
                    let sel = usize::try_from(s)
                        .ok()
                        .and_then(|s| p.color_selectors.get(s))
                        .copied()
                        .ok_or_else(|| invalid("crunch: a colour selector past the palette"))?;
                    words.push(e);
                    words.push(sel);
                }
                let at = face
                    .checked_mul(face_bytes)
                    .and_then(|f| f.checked_add(y.checked_mul(blocks_x)?.checked_add(x)?.checked_mul(block)?))
                    .ok_or_else(|| invalid("crunch: block offset"))?;
                let dst = at
                    .checked_add(block)
                    .and_then(|end| out.get_mut(at..end))
                    .ok_or_else(|| invalid("crunch: block past the level"))?;
                for (chunk, word) in dst.as_chunks_mut::<4>().0.iter_mut().zip(&words) {
                    *chunk = word.to_le_bytes();
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written most significant first, as [`Bits`] reads them.
    #[derive(Default)]
    struct Writer {
        bytes: Vec<u8>,
        bits: u32,
    }

    impl Writer {
        fn put(&mut self, value: u32, n: u32) {
            for i in (0..n).rev() {
                if self.bits.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                let bit = ((value >> i) & 1) as u8;
                *self.bytes.last_mut().unwrap() |= bit << (7 - self.bits % 8);
                self.bits += 1;
            }
        }

        /// A model of two symbols of one bit each (`0` codes symbol 0, `1` symbol 1): the code-length code 1 alone
        /// has a size (one bit, so its code is `0`), and each symbol's size is that code.
        fn two_symbol_model(&mut self) {
            self.put(2, 14);
            // the code-length codes sent: up to symbol 1, the 19th of the order
            self.put(19, 5);
            for code in CODE_LENGTH_ORDER.iter().take(19) {
                self.put(u32::from(*code == 1), 3);
            }
            self.put(0, 1);
            self.put(0, 1);
        }
    }

    /// A one-level 4x4 DXT1 crunch file: one colour endpoint of component deltas `endpoint` (R, G, B of the first
    /// colour, then of the second), one selector of the eight 4-bit symbols `selector`, and the level's four blocks
    /// (a 2x2 group, three of them outside the 1x1 block grid) each referencing a new endpoint.
    fn dxt1_file(endpoint: [u32; 6], selector: [u32; 8]) -> Vec<u8> {
        let mut tables = Writer::default();
        for _ in 0..3 {
            tables.two_symbol_model();
        }
        let mut endpoints = Writer::default();
        endpoints.two_symbol_model();
        endpoints.two_symbol_model();
        for d in endpoint {
            endpoints.put(d, 1);
        }
        let mut selectors = Writer::default();
        selectors.two_symbol_model();
        for s in selector {
            selectors.put(s, 1);
        }
        let mut level = Writer::default();
        // the group's references (symbol 0: every reference is 0, a new endpoint), then per block its endpoint delta
        // (0) and selector (0)
        level.put(0, 1);
        for _ in 0..4 {
            level.put(0, 1);
            level.put(0, 1);
        }
        let mut file = vec![0u8; HEADER_BYTES + 4];
        let mut streams = Vec::new();
        let mut place = |bytes: &[u8]| -> (usize, usize) {
            let at = HEADER_BYTES + 4 + streams.len();
            streams.extend_from_slice(bytes);
            (at, bytes.len())
        };
        let tables = place(&tables.bytes);
        let endpoints = place(&endpoints.bytes);
        let selectors = place(&selectors.bytes);
        let level = place(&level.bytes);
        file.extend_from_slice(&streams);
        let len = file.len();
        let mut be = |at: usize, value: usize, n: usize| {
            for i in 0..n {
                file[at + i] = (value >> (8 * (n - 1 - i))) as u8;
            }
        };
        be(0, usize::from(SIGNATURE), 2);
        be(2, HEADER_BYTES + 4, 2);
        be(6, len, 4);
        be(12, 4, 2);
        be(14, 4, 2);
        be(16, 1, 1);
        be(17, 1, 1);
        for (at, (offset, size), count) in [(33, endpoints, 1), (41, selectors, 1)] {
            be(at, offset, 3);
            be(at + 3, size, 3);
            be(at + 6, count, 2);
        }
        be(65, tables.1, 2);
        be(67, tables.0, 3);
        be(70, level.0, 4);
        file
    }

    #[test]
    fn a_dxt1_level_unpacks_to_its_endpoint_and_selector() {
        let file = dxt1_file([1, 1, 0, 1, 0, 1], [1, 0, 1, 1, 0, 0, 1, 0]);
        let header = CrunchHeader::parse(&file).unwrap();
        assert_eq!(
            (header.width, header.height, header.levels, header.format),
            (4, 4, 1, CrunchFormat::Dxt1)
        );
        let block = unpack_level(&file, &header, 0).unwrap();
        // the endpoint: components a..f summed from zero, packed c | b << 5 | a << 11 | f << 16 | e << 21 | d << 27
        let endpoint: u32 = (1 << 5) | (1 << 11) | (1 << 16) | (1 << 27);
        // the selector: each symbol xored in at its nibble, the linear ranks mapped to DXT1's order
        let linear: u32 = 1 | (1 << 8) | (1 << 12) | (1 << 24);
        let selector = ((linear ^ (linear << 1)) & 0xAAAA_AAAA) | ((linear >> 1) & 0x5555_5555);
        let mut expected = endpoint.to_le_bytes().to_vec();
        expected.extend_from_slice(&selector.to_le_bytes());
        assert_eq!(block, expected);
    }

    #[test]
    fn damaged_files_are_refused_not_panicked_on() {
        let file = dxt1_file([1, 1, 0, 1, 0, 1], [1, 0, 1, 1, 0, 0, 1, 0]);
        let header = CrunchHeader::parse(&file).unwrap();
        // every truncation is refused at the header or by a stream past the data
        for len in 0..file.len() {
            if let Ok(h) = CrunchHeader::parse(&file[..len]) {
                let _ = unpack_level(&file[..len], &h, 0);
            }
        }
        // every single-byte change decodes or is refused, never panics
        for at in 0..file.len() {
            for flip in [0x01u8, 0x80, 0xFF] {
                let mut bad = file.clone();
                bad[at] ^= flip;
                if let Ok(h) = CrunchHeader::parse(&bad) {
                    let _ = unpack_level(&bad, &h, 0);
                }
            }
        }
        assert!(unpack_level(&file, &header, 1).is_err());
        let mut etc = file.clone();
        etc[18] = 10;
        assert!(matches!(
            CrunchHeader::parse(&etc),
            Err(crate::Error::Unsupported(_))
        ));
    }

    /// Past the end of its stream a reader gives zeros, as crunch's decoder does.
    #[test]
    fn bits_past_the_end_are_zero() {
        let mut bits = Bits::new(&[0b1010_0001]);
        assert_eq!(bits.bits(3), 0b101);
        assert_eq!(bits.bits(5), 0b00001);
        assert_eq!(bits.bits(12), 0);
    }
}
