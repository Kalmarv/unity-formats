//! Unity AnimationClips: the curves of a Mecanim clip (`m_MuscleClip`) decoded to keys with tangents.
//!
//! A clip stores its curves in three blocks, numbered in this order: the **streamed** curves (`m_StreamedClip`), the
//! **dense** curves (`m_DenseClip`, one sample per frame at a fixed rate) and the **constant** curves
//! (`m_ConstantClip`, one value each). `m_ClipBindingConstant.genericBindings` lists what the curves drive, in curve
//! order: a binding takes one curve (a float property), or for a Transform the curves of its attribute, 1 position (3),
//! 2 rotation, a quaternion (4), 3 scale (3), 4 Euler angles (3). A binding names its target by `path`, the CRC-32 of the
//! path of GameObject names below the Animator's own (the Animator's GameObject itself is the empty path, hash 0).
//!
//! The streamed block is a sequence of frames, each `time: f32`, `count: i32` and `count` keys of `index: i32` and four
//! `f32` coefficients; a key opens a cubic segment of its curve that lasts until the curve's next key:
//! `value(dt) = ((c0 dt + c1) dt + c2) dt + c3` for `dt` seconds after the key, so `c3` is the key's value and `c2` its
//! outgoing slope; the incoming slope of the next key is the derivative at the segment's end. The first frame is at
//! `-f32::MAX` and gives every curve's initial value; the last is at `+inf` with no keys. A segment whose coefficients are
//! not finite is a step.

use crate::player::{ObjRef, Player};
use crate::value::{Array, Value};
use crate::{Error, Result, invalid};

/// One curve binding of a clip (`genericBindings`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericBinding {
    /// The CRC-32 of the target's path below the Animator's GameObject.
    pub path: u32,
    /// The attribute: for a Transform (`type_id` 4) 1 position, 2 rotation, 3 scale, 4 Euler angles; else the
    /// property's hash.
    pub attribute: u32,
    /// The target's class id (4 Transform, 95 Animator, 137 SkinnedMeshRenderer, ...).
    pub type_id: i64,
    /// Whether the curve drives an object reference (`isPPtrCurve`); such clips are refused by [`ClipData::read`].
    pub is_pptr: bool,
}

impl GenericBinding {
    /// How many curves this binding takes.
    pub fn width(&self) -> usize {
        match (self.type_id, self.attribute) {
            (4, 1 | 3 | 4) => 3,
            (4, 2) => 4,
            _ => 1,
        }
    }
}

/// Where a curve was stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// The streamed block (`m_StreamedClip`).
    Streamed,
    /// The dense block (`m_DenseClip`).
    Dense,
    /// The constant block (`m_ConstantClip`).
    Constant,
}

/// A key of a curve: its time (seconds), value and the slopes (per second) into and out of it. An infinite outgoing
/// slope is a step to the next key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Seconds from the clip's start.
    pub time: f32,
    /// The curve's value at `time`.
    pub value: f32,
    /// The slope arriving at the key, per second.
    pub in_slope: f32,
    /// The slope leaving the key, per second; infinite for a step.
    pub out_slope: f32,
}

/// One scalar curve: keys in increasing time, evaluated as a cubic Hermite spline between them (a dense curve's slopes
/// are its secants, so it is piecewise linear).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    /// Which block the curve was stored in.
    pub storage: Storage,
    /// The keys, in increasing time.
    pub keys: Vec<Key>,
}

impl Curve {
    /// The value at `t` and the slopes into and out of it: a key's own at its time, a segment's derivative inside it,
    /// the end values held with no slope outside. A step segment holds its key's value. A curve without keys is 0; a
    /// `t` that is NaN has the value NaN.
    pub fn sample(&self, t: f32) -> (f32, f32, f32) {
        let k = &self.keys;
        let (Some(first), Some(last)) = (k.first(), k.last()) else {
            return (0.0, 0.0, 0.0);
        };
        if t.is_nan() {
            return (f32::NAN, 0.0, 0.0);
        }
        if t <= first.time {
            return (
                first.value,
                0.0,
                if t == first.time { first.out_slope } else { 0.0 },
            );
        }
        if t >= last.time {
            return (last.value, if t == last.time { last.in_slope } else { 0.0 }, 0.0);
        }
        let i = k.partition_point(|key| key.time <= t) - 1;
        let (a, b) = (&k[i], &k[i + 1]);
        if t == a.time {
            return (a.value, a.in_slope, a.out_slope);
        }
        if !a.out_slope.is_finite() {
            return (a.value, 0.0, 0.0);
        }
        let (dt, s) = (
            f64::from(b.time - a.time),
            f64::from(t - a.time) / f64::from(b.time - a.time),
        );
        let (p0, p1) = (f64::from(a.value), f64::from(b.value));
        let (m0, m1) = (f64::from(a.out_slope) * dt, f64::from(b.in_slope) * dt);
        let (s2, s3) = (s * s, s * s * s);
        let value = (2.0 * s3 - 3.0 * s2 + 1.0) * p0
            + (s3 - 2.0 * s2 + s) * m0
            + (-2.0 * s3 + 3.0 * s2) * p1
            + (s3 - s2) * m1;
        let slope = ((6.0 * s2 - 6.0 * s) * p0
            + (3.0 * s2 - 4.0 * s + 1.0) * m0
            + (-6.0 * s2 + 6.0 * s) * p1
            + (3.0 * s2 - 2.0 * s) * m1)
            / dt;
        (value as f32, slope as f32, slope as f32)
    }

    /// Whether any slope is a step.
    pub fn has_steps(&self) -> bool {
        self.keys.iter().any(|k| !k.out_slope.is_finite())
    }

    /// Whether the curve has a single key.
    pub fn is_constant(&self) -> bool {
        self.keys.len() == 1
    }
}

/// A binding with the curves it takes, in the order of its attribute's components.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// What the curves drive.
    pub binding: GenericBinding,
    /// [`GenericBinding::width`] curves.
    pub curves: Vec<Curve>,
}

/// An AnimationClip's curves as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipData {
    /// `m_Name`.
    pub name: String,
    /// Seconds, `m_StopTime - m_StartTime`.
    pub duration: f32,
    /// `m_LoopTime`.
    pub looping: bool,
    /// The bindings with their curves, in binding order.
    pub tracks: Vec<Track>,
    /// Animation events (`m_Events`): not read.
    pub events: usize,
    /// How far a streamed segment's end was from the next key's value (relative), the worst over the segments of a
    /// millisecond or more: the decode's own consistency (the segment is the cubic the next key continues).
    pub worst_continuity_error: f32,
}

/// Words a key takes in the streamed block: its curve index and four coefficients.
const KEY_WORDS: usize = 5;

fn int(v: &Value, k: &str) -> Result<i64> {
    v.get(k)
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid(format!("clip: no integer {k}")))
}

fn float(v: &Value, k: &str) -> Result<f32> {
    v.get(k)
        .and_then(Value::as_f32)
        .ok_or_else(|| invalid(format!("clip: no float {k}")))
}

fn list<'a>(v: &'a Value, k: &str) -> Result<&'a [Value]> {
    match v.get(k) {
        Some(Value::Array(Array::Values(items))) => Ok(items),
        Some(Value::Array(a)) if a.is_empty() => Ok(&[]),
        _ => Err(invalid(format!("clip: no list {k}"))),
    }
}

fn floats(v: &Value, k: &str) -> Result<Vec<f32>> {
    match v.get(k) {
        Some(Value::Array(Array::F32(f))) => Ok(f.clone()),
        Some(Value::Array(a)) if a.is_empty() => Ok(Vec::new()),
        _ => Err(invalid(format!("clip: no float list {k}"))),
    }
}

/// The streamed block's curves: `curve_count` curves from the frames in `words`. Every curve stores at least its
/// initial key, five words, so a count the data cannot hold is refused before anything is allocated for it.
fn read_streamed(name: &str, words: &[u32], curve_count: usize) -> Result<(Vec<Curve>, f32)> {
    let bad = |why: String| invalid(format!("{name}: streamed clip: {why}"));
    let most = words.len() / KEY_WORDS;
    if curve_count > most {
        return Err(bad(format!(
            "{curve_count} curves, but {} bytes of frames hold at most {most} keys, and every curve has one",
            words.len().saturating_mul(4)
        )));
    }
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    // each curve's entries: the time and the four coefficients of the cubic that starts there
    let mut entries: Vec<Vec<(f32, [f32; 4])>> = vec![Vec::new(); curve_count];
    let mut at = 0;
    let take = |at: &mut usize, n: usize| -> Result<&[u8]> {
        let part = at
            .checked_add(n)
            .and_then(|end| bytes.get(*at..end))
            .ok_or_else(|| {
                bad(format!(
                    "frames past the data at byte {at} of {}",
                    bytes.len(),
                    at = *at
                ))
            })?;
        *at += n;
        Ok(part)
    };
    let mut last_time = f32::NEG_INFINITY;
    let mut ended = false;
    while at < bytes.len() {
        let frame_at = at;
        let time = f32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes"));
        let count = i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes"));
        if time.is_nan() || time < last_time {
            return Err(bad(format!(
                "frame time {time} after {last_time} (the frame at byte {frame_at}; times must not decrease)"
            )));
        }
        last_time = time;
        let count = usize::try_from(count).map_err(|_| {
            bad(format!(
                "{count} keys in the frame at byte {frame_at} (at least 0)"
            ))
        })?;
        for _ in 0..count {
            let key_at = at;
            let index = i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes"));
            let mut c = [0.0f32; 4];
            for x in &mut c {
                *x = f32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes"));
            }
            usize::try_from(index)
                .ok()
                .and_then(|i| entries.get_mut(i))
                .ok_or_else(|| {
                    bad(format!(
                        "a key of curve {index} of {curve_count} at byte {key_at}"
                    ))
                })?
                .push((time, c));
        }
        if time == f32::INFINITY {
            ended = true;
            break;
        }
    }
    if !ended || at != bytes.len() {
        return Err(bad(format!(
            "the frames do not end with the +inf frame at the end of the data (frames end at byte {at} of {})",
            bytes.len()
        )));
    }
    let mut worst = 0.0f32;
    let mut out = Vec::with_capacity(curve_count);
    for (i, list) in entries.iter().enumerate() {
        // the initial entry (at -f32::MAX) is the value before the first key, which the first key repeats
        let real: Vec<&(f32, [f32; 4])> = list
            .iter()
            .filter(|(t, _)| *t > f32::MIN / 2.0 && t.is_finite())
            .collect();
        let mut keys: Vec<Key> = Vec::new();
        if real.is_empty() {
            let (_, c) = list.first().ok_or_else(|| bad(format!("curve {i} has no key")))?;
            keys.push(Key {
                time: 0.0,
                value: c[3],
                in_slope: 0.0,
                out_slope: 0.0,
            });
        }
        let mut previous: Option<(f32, [f32; 4])> = None;
        for &&(time, c) in &real {
            let in_slope = match previous {
                Some((t0, p)) => {
                    let dt = time - t0;
                    // the cubic of the previous key at this key; a segment under a millisecond (a step made of two
                    // keys) is not checked, its large coefficients cancel past f32 (17 of 135,385 in SkateBIRD's clips)
                    let end = ((p[0] * dt + p[1]) * dt + p[2]) * dt + p[3];
                    if dt >= 1e-3 && end.is_finite() && c[3].is_finite() {
                        worst = worst.max((end - c[3]).abs() / (1.0 + c[3].abs()));
                    }
                    (3.0 * p[0] * dt + 2.0 * p[1]) * dt + p[2]
                }
                None => 0.0,
            };
            keys.push(Key {
                time,
                value: c[3],
                in_slope,
                out_slope: c[2],
            });
            previous = Some((time, c));
        }
        // a key whose segment is a step has non-finite coefficients: its slopes are a step
        for k in &mut keys {
            if !k.in_slope.is_finite() {
                k.in_slope = f32::INFINITY;
            }
        }
        out.push(Curve {
            storage: Storage::Streamed,
            keys,
        });
    }
    Ok((out, worst))
}

/// The dense block's curves (`m_DenseClip`): frame `f` of curve `c` is `m_SampleArray[f * curves + c]`, at
/// `m_BeginTime + f / m_SampleRate`. The sample array must hold exactly `frames * curves` values, and a block that
/// states curves needs frames: a count nothing stored backs is refused before a curve is made for it.
fn read_dense(name: &str, dense: &Value) -> Result<Vec<Curve>> {
    let bad = |why: String| invalid(format!("{name}: dense clip: {why}"));
    let dense_curves =
        usize::try_from(int(dense, "m_CurveCount")?).map_err(|_| bad("a negative curve count".into()))?;
    let frames =
        usize::try_from(int(dense, "m_FrameCount")?).map_err(|_| bad("a negative frame count".into()))?;
    let samples = floats(dense, "m_SampleArray")?;
    if frames.checked_mul(dense_curves) != Some(samples.len()) {
        return Err(bad(format!(
            "{} samples for {frames} frames of {dense_curves} curves",
            samples.len()
        )));
    }
    if frames == 0 && dense_curves > 0 {
        return Err(bad(format!("{dense_curves} curves of no frames")));
    }
    let (rate, begin) = (float(dense, "m_SampleRate")?, float(dense, "m_BeginTime")?);
    if frames > 1 && !(rate.is_finite() && rate > 0.0) {
        return Err(bad(format!(
            "sample rate {rate} for {frames} frames (a positive rate is needed)"
        )));
    }
    let times: Vec<f32> = (0..frames).map(|f| begin + f as f32 / rate).collect();
    let mut curves = Vec::with_capacity(dense_curves);
    for c in 0..dense_curves {
        let values: Vec<f32> = samples.iter().skip(c).step_by(dense_curves).copied().collect();
        let slope = |a: usize, b: usize| match (values.get(a..=b), times.get(a..=b)) {
            (Some([v0, .., v1]), Some([t0, .., t1])) => (v1 - v0) / (t1 - t0),
            _ => 0.0,
        };
        curves.push(Curve {
            storage: Storage::Dense,
            keys: values
                .iter()
                .zip(&times)
                .enumerate()
                .map(|(f, (&value, &time))| Key {
                    time,
                    value,
                    in_slope: if f > 0 { slope(f - 1, f) } else { 0.0 },
                    out_slope: if f + 1 < frames { slope(f, f + 1) } else { 0.0 },
                })
                .collect(),
        });
    }
    Ok(curves)
}

impl ClipData {
    /// The clip object `r`. Legacy clips (`m_Legacy`, curves stored per kind) are not read.
    pub fn read(player: &Player, r: &ObjRef) -> Result<ClipData> {
        let c = player.decode(r)?;
        let name = c.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned();
        if c.get("m_Legacy")
            .is_some_and(|l| matches!(l, Value::Bool(true)) || l.as_i64().is_some_and(|v| v != 0))
        {
            return Err(Error::Unsupported(format!("{name}: legacy animation clip")));
        }
        let muscle = c
            .get("m_MuscleClip")
            .ok_or_else(|| invalid(format!("{name}: no m_MuscleClip")))?;
        let data = muscle
            .at(&["m_Clip", "data"])
            .ok_or_else(|| invalid(format!("{name}: no m_MuscleClip.m_Clip.data")))?;
        let streamed = data
            .get("m_StreamedClip")
            .ok_or_else(|| invalid(format!("{name}: no m_StreamedClip")))?;
        let dense = data
            .get("m_DenseClip")
            .ok_or_else(|| invalid(format!("{name}: no m_DenseClip")))?;
        let constant = data
            .get("m_ConstantClip")
            .ok_or_else(|| invalid(format!("{name}: no m_ConstantClip")))?;
        let start = float(muscle, "m_StartTime")?;
        let stop = float(muscle, "m_StopTime")?;
        if start != 0.0 {
            return Err(Error::Unsupported(format!(
                "{name}: a clip starting at {start} s"
            )));
        }
        let streamed_words = match streamed.get("data") {
            Some(Value::Array(Array::U32(w))) => w.as_slice(),
            Some(Value::Array(a)) if a.is_empty() => &[],
            _ => return Err(invalid(format!("{name}: streamed clip data is not 32-bit words"))),
        };
        let streamed_count = usize::try_from(int(streamed, "curveCount")?)
            .map_err(|_| invalid(format!("{name}: negative curve count")))?;
        let (mut curves, worst) = if streamed_count == 0 && streamed_words.is_empty() {
            (Vec::new(), 0.0)
        } else {
            read_streamed(&name, streamed_words, streamed_count)?
        };
        curves.extend(read_dense(&name, dense)?);
        for v in floats(constant, "data")? {
            curves.push(Curve {
                storage: Storage::Constant,
                keys: vec![Key {
                    time: 0.0,
                    value: v,
                    in_slope: 0.0,
                    out_slope: 0.0,
                }],
            });
        }
        // the bindings take the curves in order
        let mut bindings = Vec::new();
        let constant_bindings = c
            .get("m_ClipBindingConstant")
            .ok_or_else(|| invalid(format!("{name}: no m_ClipBindingConstant")))?;
        for b in list(constant_bindings, "genericBindings")? {
            bindings.push(GenericBinding {
                path: u32::try_from(int(b, "path")?)
                    .map_err(|_| invalid(format!("{name}: a path hash beyond 32 bits")))?,
                attribute: u32::try_from(int(b, "attribute")?)
                    .map_err(|_| invalid(format!("{name}: an attribute beyond 32 bits")))?,
                type_id: int(b, "typeID")?,
                is_pptr: int(b, "isPPtrCurve")? != 0,
            });
        }
        if let Some(b) = bindings.iter().find(|b| b.is_pptr) {
            return Err(Error::Unsupported(format!(
                "{name}: a PPtr curve (attribute {})",
                b.attribute
            )));
        }
        let wanted: usize = bindings.iter().map(GenericBinding::width).sum();
        if wanted != curves.len() {
            return Err(invalid(format!(
                "{name}: the bindings take {wanted} curves, the clip stores {}",
                curves.len()
            )));
        }
        let mut rest = curves.into_iter();
        let tracks = bindings
            .into_iter()
            .map(|binding| {
                let curves = rest.by_ref().take(binding.width()).collect();
                Track { binding, curves }
            })
            .collect();
        let looping = muscle
            .get("m_LoopTime")
            .is_some_and(|l| matches!(l, Value::Bool(true)) || l.as_i64().is_some_and(|v| v != 0));
        Ok(ClipData {
            name,
            duration: stop - start,
            looping,
            tracks,
            events: list(&c, "m_Events")?.len(),
            worst_continuity_error: worst,
        })
    }
}

/// The hash a clip's binding names a path by: the CRC-32 (the zlib one) of the path of GameObject names below the
/// Animator's GameObject, empty for the GameObject itself.
pub fn path_hash(path: &str) -> u32 {
    crate::animator::name_hash(path)
}

/// The clips an AnimatorController lists (`m_AnimationClips`), in order, null entries left out.
pub fn controller_clips(player: &Player, controller: &ObjRef) -> Result<Vec<ObjRef>> {
    let c = player.decode(controller)?;
    let mut out = Vec::new();
    for p in list(&c, "m_AnimationClips")? {
        if let Some(clip) = player.resolve(&controller.file, p)? {
            out.push(clip);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, in_slope: f32, out_slope: f32) -> Key {
        Key {
            time,
            value,
            in_slope,
            out_slope,
        }
    }

    fn frame(time: f32, keys: &[(i32, [f32; 4])]) -> Vec<u32> {
        let mut words = vec![time.to_bits(), keys.len() as u32];
        for (index, c) in keys {
            words.push(*index as u32);
            words.extend(c.iter().map(|x| x.to_bits()));
        }
        words
    }

    /// The initial frame, two keys of one curve and the end frame: the keys' values, outgoing slopes, the incoming
    /// slope of the second as the first cubic's derivative at its end, and the continuity of the decode.
    #[test]
    fn a_streamed_curve_decodes_to_keys_with_slopes() {
        // a cubic from 1 at t=0 over 2 s: v(dt) = 0.5 dt^3 - dt^2 + 3 dt + 1; its end value 1+6-4+4=7... at dt = 2
        let (c0, c1, c2, c3) = (0.5f32, -1.0, 3.0, 1.0);
        let end = ((c0 * 2.0 + c1) * 2.0 + c2) * 2.0 + c3;
        let mut words = frame(-f32::MAX, &[(0, [0.0, 0.0, 0.0, 1.0])]);
        words.extend(frame(0.0, &[(0, [c0, c1, c2, c3])]));
        words.extend(frame(2.0, &[(0, [0.0, 0.0, 0.5, end])]));
        words.extend(frame(f32::INFINITY, &[]));
        let (curves, worst) = read_streamed("t", &words, 1).unwrap();
        assert_eq!(worst, 0.0);
        let k = &curves[0].keys;
        assert_eq!(k.len(), 2);
        assert_eq!((k[0].time, k[0].value, k[0].out_slope), (0.0, 1.0, 3.0));
        // the derivative at the end of the first segment: 3 c0 dt^2 + 2 c1 dt + c2 = 6 - 4 + 3
        assert_eq!(
            (k[1].time, k[1].value, k[1].in_slope, k[1].out_slope),
            (2.0, end, 5.0, 0.5)
        );
        // the Hermite spline through these keys is the stored cubic
        let (v, s, _) = curves[0].sample(1.0);
        assert!((v - (0.5 - 1.0 + 3.0 + 1.0)).abs() < 1e-5, "{v}");
        assert!((s - (1.5 - 2.0 + 3.0)).abs() < 1e-5, "{s}");
        assert_eq!(curves[0].sample(0.0), (1.0, 0.0, 3.0));
        assert_eq!(curves[0].sample(2.0), (end, 5.0, 0.0));
    }

    /// A curve with only its initial entry is constant; frames that do not end at +inf, a key of a curve beyond the
    /// count and times going back are refused.
    #[test]
    fn streamed_clips_refuse_what_is_not_the_format() {
        let mut words = frame(-f32::MAX, &[(0, [0.0, 0.0, 0.0, 4.0])]);
        words.extend(frame(f32::INFINITY, &[]));
        let (curves, _) = read_streamed("t", &words, 1).unwrap();
        assert_eq!(curves[0].keys, vec![key(0.0, 4.0, 0.0, 0.0)]);
        let refused = |words: &[u32], n: usize| read_streamed("t", words, n).unwrap_err().to_string();
        assert!(refused(&words[..words.len() - 2], 1).contains("do not end"));
        assert!(refused(&words, 0).contains("curve 0 of 0"));
        let mut back = frame(1.0, &[]);
        back.extend(frame(0.5, &[]));
        back.extend(frame(f32::INFINITY, &[]));
        assert!(refused(&back, 1).contains("after"));
    }

    /// A curve count nothing in the data backs is refused before a vector is made for it, with the offset of the bad
    /// bytes where there is one.
    #[test]
    fn a_streamed_curve_count_is_bounded_by_the_data() {
        let mut words = frame(-f32::MAX, &[(0, [0.0, 0.0, 0.0, 4.0])]);
        words.extend(frame(f32::INFINITY, &[]));
        for n in [2, 1 << 40, usize::MAX] {
            let e = read_streamed("t", &words, n).unwrap_err().to_string();
            assert!(e.contains("hold at most 1 keys"), "{e}");
        }
        // a key naming curve 5 of 1, at the byte the key starts (the frame header takes 8)
        let mut words = frame(-f32::MAX, &[(5, [0.0; 4])]);
        words.extend(frame(f32::INFINITY, &[]));
        let e = read_streamed("t", &words, 1).unwrap_err().to_string();
        assert!(e.contains("curve 5 of 1 at byte 8"), "{e}");
        let mut back = frame(1.0, &[]);
        back.extend(frame(0.5, &[]));
        let e = read_streamed("t", &back, 0).unwrap_err().to_string();
        assert!(e.contains("byte 8"), "{e}");
    }

    fn dense(curves: i64, frames: i64, samples: Vec<f32>) -> Value {
        let f = |k: &str, v: Value| (std::sync::Arc::<str>::from(k), v);
        Value::Struct(vec![
            f("m_CurveCount", Value::Int(curves)),
            f("m_FrameCount", Value::Int(frames)),
            f("m_SampleRate", Value::Float(2.0)),
            f("m_BeginTime", Value::Float(0.0)),
            f("m_SampleArray", Value::Array(Array::F32(samples))),
        ])
    }

    /// The dense block's frame and curve counts are multiplied without overflow and must be backed by samples: a huge
    /// count with no samples makes no curves.
    #[test]
    fn dense_counts_are_checked_against_the_samples() {
        for (curves, frames) in [(i64::MAX, 3), (3, i64::MAX), (i64::MAX, i64::MAX), (1 << 40, 0)] {
            let e = read_dense("t", &dense(curves, frames, vec![]))
                .unwrap_err()
                .to_string();
            assert!(e.contains("dense clip"), "{e}");
        }
        let curves = read_dense("t", &dense(2, 2, vec![0.0, 10.0, 1.0, 20.0])).unwrap();
        assert_eq!(curves.len(), 2);
        assert_eq!(
            curves[1].keys,
            vec![key(0.0, 10.0, 0.0, 20.0), key(0.5, 20.0, 20.0, 0.0)]
        );
        assert_eq!(read_dense("t", &dense(0, 0, vec![])).unwrap(), vec![]);
    }

    /// A time that is not a number samples to NaN instead of indexing before the first key.
    #[test]
    fn sampling_at_nan_does_not_panic() {
        let curve = Curve {
            storage: Storage::Constant,
            keys: vec![key(0.0, 1.0, 0.0, 0.0), key(1.0, 2.0, 0.0, 0.0)],
        };
        assert!(curve.sample(f32::NAN).0.is_nan());
    }

    /// A Hermite curve samples to its keys, and a transform track of three curves with different key times becomes
    /// one Hermite channel at the union of the times whose pieces are the same cubics; constants give one key, dense
    /// curves a Linear channel.
    #[test]
    fn path_hashes_are_the_crc32_of_the_path() {
        assert_eq!(path_hash(""), 0);
        assert_eq!(path_hash("scarf"), 2_702_068_705);
        assert_eq!(path_hash("root/Lchain1/Lchain2/Lchain3"), 3_634_936_137);
        assert_eq!(path_hash("root/Rchain1/Rchain2"), 356_844_446);
    }
}
