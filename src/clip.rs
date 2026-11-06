//! Unity AnimationClips: the curves of a Mecanim clip (`m_MuscleClip`) decoded to keys with tangents, and the keyframe
//! curves of a legacy clip.
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
//!
//! A binding of an object property (`isPPtrCurve`: a renderer's material or sprite) takes one curve whose values are
//! indices into the objects `m_ClipBindingConstant.pptrCurveMapping` lists.
//!
//! A **legacy** clip (`m_Legacy`, played by the Animation component) has no Mecanim curves: its `m_MuscleClip` is a
//! default one (no curves, a stop time of 1 s) and its curves are Unity's own keyframes, stored by kind with the path of
//! their target: `m_RotationCurves` (quaternions), `m_EulerCurves`, `m_PositionCurves`, `m_ScaleCurves` (three
//! components each), `m_FloatCurves` (a property: its `attribute` name and the `classID` of its component) and
//! `m_PPtrCurves` (an object property, its keys objects). [`ClipData::read`] gives them as the tracks of a Mecanim clip,
//! in that order: the path and a float or PPtr curve's attribute hashed as a Mecanim binding hashes them (CRC-32), a
//! Transform's kinds as its attributes 1 to 4, a PPtr curve's keys as indices into the clip's objects. A keyframe has a
//! time, a value and its two slopes, and from Unity 2018.1 a `weightedMode` (bit 1 the in weight, bit 2 the out weight)
//! with the two weights: a weighted side of a segment is a Bezier handle that fraction of the segment from its key, an
//! unweighted one a third (where the Hermite spline's handles are).

use std::cmp::Ordering;

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
    /// Whether the curve drives an object reference (`isPPtrCurve`): its one curve's values are indices into
    /// [`ClipData::pptr_targets`], each held until the next key.
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
    /// A legacy clip's keyframe curve (`m_RotationCurves` and the others): Unity's own keys and tangents.
    Keyframes,
}

/// A key of a curve: its time (seconds), value and the slopes (per second) into and out of it. An infinite slope on
/// either side of a segment is a step: the segment holds its first key's value.
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
    /// The weight of the incoming tangent (a legacy key whose `weightedMode` has bit 1): its Bezier handle reaches
    /// back this fraction of the segment before the key. `None` is a third, the Hermite spline's.
    pub in_weight: Option<f32>,
    /// The weight of the outgoing tangent (`weightedMode` bit 2): its handle reaches this fraction of the segment
    /// after the key. `None` is a third.
    pub out_weight: Option<f32>,
}

impl Key {
    /// An unweighted key.
    pub fn new(time: f32, value: f32, in_slope: f32, out_slope: f32) -> Key {
        Key {
            time,
            value,
            in_slope,
            out_slope,
            in_weight: None,
            out_weight: None,
        }
    }
}

/// The weight of a tangent with none: a third of the segment, where a Hermite spline's Bezier handles are.
const THIRD: f64 = 1.0 / 3.0;

/// One scalar curve: keys in increasing time, evaluated as a cubic Hermite spline between them (a dense curve's slopes
/// are its secants, so it is piecewise linear); a segment with a weighted side is the cubic Bezier curve in time and
/// value whose handles the weights place.
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
    /// `t` that is NaN has the value NaN. A weight is clamped to 0 to 1.
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
        // the first key after `t`, and the one before it; `t` is strictly between the first and last keys
        let after = k.partition_point(|key| key.time <= t);
        let (Some(a), Some(b)) = (after.checked_sub(1).and_then(|i| k.get(i)), k.get(after)) else {
            return (last.value, 0.0, 0.0);
        };
        if t == a.time {
            return (a.value, a.in_slope, a.out_slope);
        }
        if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
            return (a.value, 0.0, 0.0);
        }
        if a.out_weight.is_some() || b.in_weight.is_some() {
            return weighted(a, b, t);
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
        (crate::narrow(value), crate::narrow(slope), crate::narrow(slope))
    }

    /// Whether any slope is a step.
    pub fn has_steps(&self) -> bool {
        self.keys
            .iter()
            .any(|k| !k.out_slope.is_finite() || !k.in_slope.is_finite())
    }

    /// Whether a segment is weighted (its value is not the Hermite spline of its keys).
    pub fn is_weighted(&self) -> bool {
        self.keys
            .windows(2)
            .any(|w| matches!(w, [a, b] if a.out_weight.is_some() || b.in_weight.is_some()))
    }

    /// Whether the curve has a single key.
    pub fn is_constant(&self) -> bool {
        self.keys.len() == 1
    }
}

/// The value and slope at `t` of the weighted segment from `a` to `b` (`a.time < t < b.time`): the cubic Bezier curve
/// from `(a.time, a.value)` to `(b.time, b.value)` whose handles are `w_out dt` after `a` along its slope and `w_in dt`
/// before `b` along its, `dt` the segment's span. With weights in 0 to 1 its time does not decrease, so the parameter
/// at `t` is found by bisection.
fn weighted(a: &Key, b: &Key, t: f32) -> (f32, f32, f32) {
    let dt = f64::from(b.time) - f64::from(a.time);
    let w = |weight: Option<f32>| weight.map_or(THIRD, |w| f64::from(w).clamp(0.0, 1.0));
    let (w0, w1) = (w(a.out_weight), w(b.in_weight));
    let (x1, x2) = (w0 * dt, dt - w1 * dt);
    let (y0, y3) = (f64::from(a.value), f64::from(b.value));
    let (y1, y2) = (
        y0 + w0 * dt * f64::from(a.out_slope),
        y3 - w1 * dt * f64::from(b.in_slope),
    );
    let bezier = |p: [f64; 4], u: f64| {
        let v = 1.0 - u;
        v * v * v * p[0] + 3.0 * v * v * u * p[1] + 3.0 * v * u * u * p[2] + u * u * u * p[3]
    };
    let derivative = |p: [f64; 4], u: f64| {
        let v = 1.0 - u;
        3.0 * (v * v * (p[1] - p[0]) + 2.0 * v * u * (p[2] - p[1]) + u * u * (p[3] - p[2]))
    };
    let (xs, ys) = ([0.0, x1, x2, dt], [y0, y1, y2, y3]);
    let x = f64::from(t) - f64::from(a.time);
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    // 60 halvings take the parameter to f64's precision
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if bezier(xs, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let u = 0.5 * (lo + hi);
    let dx = derivative(xs, u);
    let slope = if dx > 0.0 { derivative(ys, u) / dx } else { 0.0 };
    (
        crate::narrow(bezier(ys, u)),
        crate::narrow(slope),
        crate::narrow(slope),
    )
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
    /// Seconds, `m_StopTime - m_StartTime`; for a legacy clip, the time of its last key.
    pub duration: f32,
    /// `m_LoopTime`; for a legacy clip, a wrap mode (`m_WrapMode`) of 2, loop.
    pub looping: bool,
    /// Whether the clip plays forwards then backwards (a legacy clip's wrap mode 4, ping-pong).
    pub ping_pong: bool,
    /// Whether the clip is a legacy one (its curves [`Storage::Keyframes`]).
    pub legacy: bool,
    /// The bindings with their curves, in binding order.
    pub tracks: Vec<Track>,
    /// The objects the PPtr tracks' values index (`m_ClipBindingConstant.pptrCurveMapping`; for a legacy clip, the
    /// objects of its `m_PPtrCurves` keys in order); `None` for a null reference.
    pub pptr_targets: Vec<Option<ObjRef>>,
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
        let end = at.checked_add(n);
        let part = end.and_then(|end| bytes.get(*at..end)).ok_or_else(|| {
            bad(format!(
                "frames past the data at byte {at} of {}",
                bytes.len(),
                at = *at
            ))
        })?;
        *at = end.unwrap_or(*at);
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
            let slot = usize::try_from(index).ok().ok_or_else(|| {
                bad(format!(
                    "a key of curve {index} of {curve_count} at byte {key_at}"
                ))
            })?;
            entries
                .get_mut(slot)
                // frames that end exactly where the data does but name a curve the count does not include (PEAK's
                // 6000.3.15f1 clips have them, in the first frame and later) are a layout this reader does not know,
                // not damaged bytes
                .ok_or_else(|| {
                    Error::Unsupported(format!(
                        "{name}: streamed clip: a key of curve {index} of {curve_count} at byte {key_at}, a curve \
                         past the stated count"
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
            keys.push(Key::new(0.0, c[3], 0.0, 0.0));
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
            keys.push(Key::new(time, c[3], in_slope, c[2]));
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
                .map(|(f, (&value, &time))| {
                    Key::new(
                        time,
                        value,
                        f.checked_sub(1).map_or(0.0, |p| slope(p, f)),
                        if f < frames.saturating_sub(1) {
                            slope(f, f.saturating_add(1))
                        } else {
                            0.0
                        },
                    )
                })
                .collect(),
        });
    }
    Ok(curves)
}

/// Whether a flag field is set (stored as a bool or an integer).
fn flag(v: &Value, k: &str) -> bool {
    v.get(k)
        .is_some_and(|l| matches!(l, Value::Bool(true)) || l.as_i64().is_some_and(|v| v != 0))
}

/// The Transform class.
const TRANSFORM: i64 = 4;

/// The kinds of a legacy clip's curves: the field, the Transform attribute it is (none for a float curve) and the
/// components of a key's value (none for a float).
const LEGACY_KINDS: [(&str, Option<u32>, &[&str]); 5] = [
    ("m_RotationCurves", Some(2), &["x", "y", "z", "w"]),
    ("m_EulerCurves", Some(4), &["x", "y", "z"]),
    ("m_PositionCurves", Some(1), &["x", "y", "z"]),
    ("m_ScaleCurves", Some(3), &["x", "y", "z"]),
    ("m_FloatCurves", None, &[]),
];

/// The `RotationOrder` of the Euler angles Mecanim's Euler attribute takes: Z, then X, then Y (Unity's default).
const ROTATION_ORDER_ZXY: i64 = 4;

/// The legacy wrap modes that loop and that ping-pong (`WrapMode.Loop`, `WrapMode.PingPong`).
const WRAP_LOOP: i64 = 2;
const WRAP_PING_PONG: i64 = 4;

/// A component of a keyframe's `field`: the float itself, or the field `c` of a vector.
fn component(name: &str, key: &Value, field: &str, c: Option<&str>) -> Result<f32> {
    let v = key
        .get(field)
        .ok_or_else(|| invalid(format!("{name}: a keyframe without {field}")))?;
    match c {
        None => v.as_f32(),
        Some(c) => v.get(c).and_then(Value::as_f32),
    }
    .ok_or_else(|| {
        invalid(format!(
            "{name}: keyframe {field} is not a float or a vector with {c:?}"
        ))
    })
}

/// One component of a legacy curve's keys (`c` of a vector value, `None` for a float curve).
fn legacy_keys(name: &str, path: &str, keys: &[Value], c: Option<&str>) -> Result<Vec<Key>> {
    let mut out: Vec<Key> = Vec::with_capacity(keys.len());
    for k in keys {
        let time = float(k, "time")?;
        if let Some(p) = out.last()
            && !matches!(p.time.partial_cmp(&time), Some(Ordering::Less | Ordering::Equal))
        {
            return Err(invalid(format!(
                "{name}: a key of {path} at {time} s after one at {} s (times must not decrease)",
                p.time
            )));
        }
        let mode = k.get("weightedMode").and_then(Value::as_i64).unwrap_or(0);
        let weight = |bit: i64, field: &str| -> Result<Option<f32>> {
            if mode & bit == 0 {
                Ok(None)
            } else {
                component(name, k, field, c).map(Some)
            }
        };
        out.push(Key {
            time,
            value: component(name, k, "value", c)?,
            in_slope: component(name, k, "inSlope", c)?,
            out_slope: component(name, k, "outSlope", c)?,
            in_weight: weight(1, "inWeight")?,
            out_weight: weight(2, "outWeight")?,
        });
    }
    Ok(out)
}

/// Every key of a PPtr track names an entry of the clip's `targets` objects: a whole number from 0.
fn check_pptr_indices(name: &str, tracks: &[Track], targets: usize) -> Result<()> {
    for t in tracks.iter().filter(|t| t.binding.is_pptr) {
        for k in t.curves.iter().flat_map(|c| &c.keys) {
            let v = k.value;
            let below = u32::try_from(targets).map_or(true, |n| f64::from(v) < f64::from(n));
            if !(v >= 0.0 && v.fract() == 0.0 && below) {
                return Err(invalid(format!(
                    "{name}: a PPtr curve key of {v} at {} s, which is no index of the {targets} objects the clip's \
                     PPtr curves name",
                    k.time
                )));
            }
        }
    }
    Ok(())
}

/// A legacy clip's PPtr curve (`m_PPtrCurves`): a track of one stepped curve whose keys are indices into `targets`,
/// to which each key's object is added.
fn legacy_pptr_track(
    name: &str,
    entry: &Value,
    resolve: &dyn Fn(&Value) -> Result<Option<ObjRef>>,
    targets: &mut Vec<Option<ObjRef>>,
) -> Result<Track> {
    let text = |k: &str| {
        entry
            .get(k)
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(format!("{name}: a PPtr curve without its {k}")))
    };
    let path = text("path")?;
    let mut keys: Vec<Key> = Vec::new();
    for k in list(entry, "curve")? {
        let time = float(k, "time")?;
        if let Some(p) = keys.last()
            && !matches!(p.time.partial_cmp(&time), Some(Ordering::Less | Ordering::Equal))
        {
            return Err(invalid(format!(
                "{name}: a PPtr key of {path} at {time} s after one at {} s (times must not decrease)",
                p.time
            )));
        }
        let target = k
            .get("value")
            .ok_or_else(|| invalid(format!("{name}: a PPtr key of {path} without its value")))?;
        // an index into the targets (exact in an f32 below 2^24), held until the next key
        let index = u32::try_from(targets.len())
            .ok()
            .filter(|&n| n < 1 << 24)
            .map(|n| crate::narrow(f64::from(n)))
            .ok_or_else(|| invalid(format!("{name}: more PPtr keys than an f32 index counts")))?;
        targets.push(resolve(target)?);
        keys.push(Key::new(time, index, f32::INFINITY, f32::INFINITY));
    }
    Ok(Track {
        binding: GenericBinding {
            path: path_hash(path),
            attribute: path_hash(text("attribute")?),
            type_id: int(entry, "classID")?,
            is_pptr: true,
        },
        curves: vec![Curve {
            storage: Storage::Keyframes,
            keys,
        }],
    })
}

/// A legacy clip's curves (the module's documentation has the layout); `resolve` follows a PPtr of the clip's file.
/// Refused: compressed clips (`m_Compressed`, `m_CompressedRotationCurves`) and Euler curves of another rotation order
/// than Mecanim's.
fn read_legacy(
    name: &str,
    c: &Value,
    resolve: &dyn Fn(&Value) -> Result<Option<ObjRef>>,
) -> Result<ClipData> {
    if flag(c, "m_Compressed") || !list(c, "m_CompressedRotationCurves")?.is_empty() {
        return Err(Error::Unsupported(format!("{name}: a compressed legacy clip")));
    }
    let mut tracks = Vec::new();
    let mut last = 0.0f32;
    for (field, attribute, components) in LEGACY_KINDS {
        for entry in list(c, field)? {
            let path = entry
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(format!("{name}: a curve of {field} without a path")))?;
            let curve = entry
                .get("curve")
                .ok_or_else(|| invalid(format!("{name}: a curve of {field} without its keys")))?;
            let binding = match attribute {
                Some(attribute) => GenericBinding {
                    path: path_hash(path),
                    attribute,
                    type_id: TRANSFORM,
                    is_pptr: false,
                },
                None => {
                    let property = entry.get("attribute").and_then(Value::as_str).ok_or_else(|| {
                        invalid(format!("{name}: a float curve of {path} without its attribute"))
                    })?;
                    GenericBinding {
                        path: path_hash(path),
                        attribute: path_hash(property),
                        type_id: int(entry, "classID")?,
                        is_pptr: false,
                    }
                }
            };
            if attribute == Some(4) {
                let order = curve
                    .get("m_RotationOrder")
                    .and_then(Value::as_i64)
                    .unwrap_or(ROTATION_ORDER_ZXY);
                if order != ROTATION_ORDER_ZXY {
                    return Err(Error::Unsupported(format!(
                        "{name}: Euler curves of {path:?} in rotation order {order} (Mecanim's Euler attribute is \
                         order {ROTATION_ORDER_ZXY}, Z then X then Y)"
                    )));
                }
            }
            let keys = list(curve, "m_Curve")?;
            let curves = if components.is_empty() {
                vec![legacy_keys(name, path, keys, None)?]
            } else {
                components
                    .iter()
                    .map(|&part| legacy_keys(name, path, keys, Some(part)))
                    .collect::<Result<Vec<_>>>()?
            };
            if curves.len() != binding.width() {
                return Err(invalid(format!(
                    "{name}: the float curve of {path:?} has an attribute hash ({}) a Transform's attribute takes",
                    binding.attribute
                )));
            }
            for k in curves.iter().filter_map(|c| c.last()) {
                last = last.max(k.time);
            }
            tracks.push(Track {
                binding,
                curves: curves
                    .into_iter()
                    .map(|keys| Curve {
                        storage: Storage::Keyframes,
                        keys,
                    })
                    .collect(),
            });
        }
    }
    let mut pptr_targets = Vec::new();
    for entry in list(c, "m_PPtrCurves")? {
        let track = legacy_pptr_track(name, entry, resolve, &mut pptr_targets)?;
        for k in track.curves.iter().filter_map(|c| c.keys.last()) {
            last = last.max(k.time);
        }
        tracks.push(track);
    }
    let wrap = c.get("m_WrapMode").and_then(Value::as_i64).unwrap_or(0);
    Ok(ClipData {
        name: name.to_owned(),
        duration: last,
        looping: wrap == WRAP_LOOP,
        ping_pong: wrap == WRAP_PING_PONG,
        legacy: true,
        tracks,
        pptr_targets,
        events: list(c, "m_Events")?.len(),
        worst_continuity_error: 0.0,
    })
}

impl ClipData {
    /// The clip object `r`: a Mecanim clip's curves, or a legacy clip's keyframe curves. Refused: a Mecanim int curve
    /// (`isIntCurve`, whose values are not floats; none of the players read has one).
    pub fn read(player: &Player, r: &ObjRef) -> Result<ClipData> {
        let c = player.decode(r)?;
        let name = c.get("m_Name").and_then(Value::as_str).unwrap_or("").to_owned();
        if flag(&c, "m_Legacy") {
            return read_legacy(&name, &c, &|p| player.resolve(&r.file, p));
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
                keys: vec![Key::new(0.0, v, 0.0, 0.0)],
            });
        }
        // the bindings take the curves in order
        let mut bindings = Vec::new();
        let constant_bindings = c
            .get("m_ClipBindingConstant")
            .ok_or_else(|| invalid(format!("{name}: no m_ClipBindingConstant")))?;
        for b in list(constant_bindings, "genericBindings")? {
            if flag(b, "isIntCurve") {
                return Err(Error::Unsupported(format!(
                    "{name}: an int curve (isIntCurve, attribute {})",
                    int(b, "attribute")?
                )));
            }
            bindings.push(GenericBinding {
                path: u32::try_from(int(b, "path")?)
                    .map_err(|_| invalid(format!("{name}: a path hash beyond 32 bits")))?,
                attribute: u32::try_from(int(b, "attribute")?)
                    .map_err(|_| invalid(format!("{name}: an attribute beyond 32 bits")))?,
                type_id: int(b, "typeID")?,
                is_pptr: int(b, "isPPtrCurve")? != 0,
            });
        }
        let pptr_targets = list(constant_bindings, "pptrCurveMapping")?
            .iter()
            .map(|p| player.resolve(&r.file, p))
            .collect::<Result<Vec<_>>>()?;
        let wanted: usize = bindings.iter().map(GenericBinding::width).sum();
        if wanted != curves.len() {
            return Err(invalid(format!(
                "{name}: the bindings take {wanted} curves, the clip stores {}",
                curves.len()
            )));
        }
        let mut rest = curves.into_iter();
        let tracks: Vec<Track> = bindings
            .into_iter()
            .map(|binding| {
                let curves = rest.by_ref().take(binding.width()).collect();
                Track { binding, curves }
            })
            .collect();
        check_pptr_indices(&name, &tracks, pptr_targets.len())?;
        Ok(ClipData {
            name,
            duration: stop - start,
            looping: flag(muscle, "m_LoopTime"),
            ping_pong: false,
            legacy: false,
            tracks,
            pptr_targets,
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
        Key::new(time, value, in_slope, out_slope)
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
        let e = read_streamed("t", &words, 1).unwrap_err();
        // well-formed frames naming a curve past the count are a layout not read; a negative index is damaged bytes
        assert!(matches!(e, Error::Unsupported(_)), "{e}");
        assert!(e.to_string().contains("curve 5 of 1 at byte 8"), "{e}");
        let mut negative = frame(-f32::MAX, &[(-1, [0.0; 4])]);
        negative.extend(frame(f32::INFINITY, &[]));
        let e = read_streamed("t", &negative, 1).unwrap_err();
        assert!(matches!(e, Error::Invalid(_)), "{e}");
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

    /// A segment whose weights are a third is the Hermite spline of its keys; other weights move the Bezier handles,
    /// so the sample at the time of a point of that Bezier curve is the point's value.
    #[test]
    fn weighted_segments_are_bezier_curves_of_their_handles() {
        let (a, b) = (key(0.0, 0.0, 0.0, 2.0), key(2.0, 1.0, 0.5, 0.0));
        let hermite = Curve {
            storage: Storage::Keyframes,
            keys: vec![a, b],
        };
        let thirds = Curve {
            storage: Storage::Keyframes,
            keys: vec![
                Key {
                    out_weight: Some(1.0 / 3.0),
                    ..a
                },
                Key {
                    in_weight: Some(1.0 / 3.0),
                    ..b
                },
            ],
        };
        assert!(thirds.is_weighted() && !hermite.is_weighted());
        for t in [0.1, 0.5, 1.0, 1.7, 1.99] {
            let (h, w) = (hermite.sample(t), thirds.sample(t));
            assert!(
                (h.0 - w.0).abs() < 1e-5 && (h.1 - w.1).abs() < 1e-4,
                "{t}: {h:?} {w:?}"
            );
        }
        // out weight 0.9, in weight 0.1 over 2 s: the handles at (1.8, 0 + 1.8 * 2) and (1.8, 1 - 0.2 * 0.5)
        let bent = Curve {
            storage: Storage::Keyframes,
            keys: vec![
                Key {
                    out_weight: Some(0.9),
                    ..a
                },
                Key {
                    in_weight: Some(0.1),
                    ..b
                },
            ],
        };
        let (xs, ys) = ([0.0f64, 1.8, 1.8, 2.0], [0.0f64, 3.6, 0.9, 1.0]);
        let at = |p: [f64; 4], u: f64| {
            let v = 1.0 - u;
            v * v * v * p[0] + 3.0 * v * v * u * p[1] + 3.0 * v * u * u * p[2] + u * u * u * p[3]
        };
        for u in [0.1, 0.25, 0.5, 0.8] {
            let (t, want) = (at(xs, u) as f32, at(ys, u) as f32);
            let (got, ..) = bent.sample(t);
            assert!((got - want).abs() < 1e-4, "u {u}, t {t}: {got} vs {want}");
            assert!(
                (got - hermite.sample(t).0).abs() > 1e-2,
                "u {u}: the weights changed nothing"
            );
        }
        // a step on the far side of a segment holds the first key
        let step = Curve {
            storage: Storage::Keyframes,
            keys: vec![a, key(2.0, 1.0, f32::INFINITY, 0.0)],
        };
        assert_eq!(step.sample(1.0), (0.0, 0.0, 0.0));
        assert!(step.has_steps());
    }

    fn field(k: &str, v: Value) -> (std::sync::Arc<str>, Value) {
        (std::sync::Arc::<str>::from(k), v)
    }

    fn vector(names: &str, base: f32) -> Value {
        Value::Struct(
            names
                .chars()
                .enumerate()
                .map(|(i, c)| field(&c.to_string(), Value::Float(base + i as f32)))
                .collect(),
        )
    }

    fn keyframe(time: f32, value: Value, mode: Option<i64>) -> Value {
        let like = |v: &Value, x: f32| match v {
            Value::Struct(f) => Value::Struct(f.iter().map(|(k, _)| (k.clone(), Value::Float(x))).collect()),
            _ => Value::Float(x),
        };
        let mut fields = vec![
            field("time", Value::Float(time)),
            field("inSlope", like(&value, 0.5)),
            field("outSlope", like(&value, -0.5)),
        ];
        if let Some(mode) = mode {
            fields.push(field("weightedMode", Value::Int(mode)));
            fields.push(field("inWeight", like(&value, 0.25)));
            fields.push(field("outWeight", like(&value, 0.75)));
        }
        fields.insert(1, field("value", value));
        Value::Struct(fields)
    }

    fn values(v: Vec<Value>) -> Value {
        Value::Array(Array::Values(v))
    }

    fn legacy(rotation: Vec<Value>, floats: Vec<Value>, wrap: i64) -> Value {
        let curve = |path: &str, keys: Vec<Value>| {
            Value::Struct(vec![
                field("curve", Value::Struct(vec![field("m_Curve", values(keys))])),
                field("path", Value::String(path.into())),
            ])
        };
        let float_curve = Value::Struct(vec![
            field("curve", Value::Struct(vec![field("m_Curve", values(floats))])),
            field("attribute", Value::String("m_IsActive".into())),
            field("path", Value::String("lid".into())),
            field("classID", Value::Int(1)),
        ]);
        Value::Struct(vec![
            field("m_Name", Value::String("open".into())),
            field("m_Legacy", Value::Bool(true)),
            field("m_Compressed", Value::Bool(false)),
            field("m_RotationCurves", values(vec![curve("lid/hinge", rotation)])),
            field("m_CompressedRotationCurves", values(vec![])),
            field("m_EulerCurves", values(vec![])),
            field("m_PositionCurves", values(vec![])),
            field("m_ScaleCurves", values(vec![])),
            field("m_FloatCurves", values(vec![float_curve])),
            field("m_PPtrCurves", values(vec![])),
            field("m_WrapMode", Value::Int(wrap)),
            field("m_Events", values(vec![])),
        ])
    }

    /// A legacy clip's rotation curve is a Transform rotation track of four curves, one per component, and its float
    /// curve a track of its class with the CRC-32 of its attribute; weights are read where `weightedMode` sets them,
    /// the duration is the last key's time, and wrap mode 4 is ping-pong.
    #[test]
    fn a_legacy_clip_reads_its_curves_by_kind() {
        let rotation = vec![
            keyframe(0.0, vector("xyzw", 0.0), None),
            keyframe(1.5, vector("xyzw", 10.0), Some(0)),
        ];
        let floats = vec![
            keyframe(0.25, Value::Float(1.0), Some(2)),
            keyframe(2.5, Value::Float(0.0), Some(1)),
        ];
        let clip = read_legacy("open", &legacy(rotation.clone(), floats.clone(), 4), &no_pptr).unwrap();
        assert!(clip.legacy && clip.ping_pong && !clip.looping);
        assert_eq!(clip.duration, 2.5);
        assert_eq!(clip.tracks.len(), 2);
        let (r, f) = (&clip.tracks[0], &clip.tracks[1]);
        assert_eq!(
            r.binding,
            GenericBinding {
                path: path_hash("lid/hinge"),
                attribute: 2,
                type_id: 4,
                is_pptr: false
            }
        );
        assert_eq!(r.curves.len(), 4);
        assert_eq!(
            r.curves[3].keys,
            vec![key(0.0, 3.0, 0.5, -0.5), key(1.5, 13.0, 0.5, -0.5)]
        );
        assert!(r.curves.iter().all(|c| c.storage == Storage::Keyframes));
        assert_eq!(
            (f.binding.path, f.binding.attribute, f.binding.type_id),
            (path_hash("lid"), path_hash("m_IsActive"), 1)
        );
        assert_eq!(
            f.curves[0].keys,
            vec![
                Key {
                    out_weight: Some(0.75),
                    ..key(0.25, 1.0, 0.5, -0.5)
                },
                Key {
                    in_weight: Some(0.25),
                    ..key(2.5, 0.0, 0.5, -0.5)
                },
            ]
        );
        assert!(
            read_legacy("open", &legacy(rotation.clone(), floats.clone(), 2), &no_pptr)
                .unwrap()
                .looping
        );
        // keys going back in time are damaged bytes
        let back = vec![
            keyframe(1.0, vector("xyzw", 0.0), None),
            keyframe(0.5, vector("xyzw", 0.0), None),
        ];
        let e = read_legacy("open", &legacy(back, floats.clone(), 0), &no_pptr).unwrap_err();
        assert!(
            matches!(e, Error::Invalid(_)) && e.to_string().contains("must not decrease"),
            "{e}"
        );
        // a PPtr curve: a stepped track of indices into the objects its keys name, in key order
        let pptr_key = |time: f32, path_id: i64| {
            Value::Struct(vec![
                field("time", Value::Float(time)),
                field(
                    "value",
                    Value::Struct(vec![
                        field("m_FileID", Value::Int(0)),
                        field("m_PathID", Value::Int(path_id)),
                    ]),
                ),
            ])
        };
        let swap = Value::Struct(vec![
            field("curve", values(vec![pptr_key(0.0, 7), pptr_key(3.0, 0)])),
            field("attribute", Value::String("m_Sprite".into())),
            field("path", Value::String("lid".into())),
            field("classID", Value::Int(212)),
        ]);
        let mut pptr = legacy(rotation, floats, 0);
        if let Value::Struct(fields) = &mut pptr {
            for (k, v) in fields.iter_mut() {
                if &**k == "m_PPtrCurves" {
                    *v = values(vec![swap.clone()]);
                }
            }
        }
        let resolve = |p: &Value| -> Result<Option<ObjRef>> {
            let id = p.get("m_PathID").and_then(Value::as_i64).unwrap();
            Ok((id != 0).then(|| ObjRef {
                file: "f".into(),
                path_id: id,
            }))
        };
        let clip = read_legacy("open", &pptr, &resolve).unwrap();
        assert_eq!(clip.duration, 3.0);
        let t = clip.tracks.last().unwrap();
        assert_eq!(
            t.binding,
            GenericBinding {
                path: path_hash("lid"),
                attribute: path_hash("m_Sprite"),
                type_id: 212,
                is_pptr: true
            }
        );
        let inf = f32::INFINITY;
        assert_eq!(
            t.curves[0].keys,
            vec![key(0.0, 0.0, inf, inf), key(3.0, 1.0, inf, inf)]
        );
        assert_eq!(
            clip.pptr_targets,
            vec![
                Some(ObjRef {
                    file: "f".into(),
                    path_id: 7
                }),
                None
            ]
        );
        assert_eq!(t.curves[0].sample(2.0).0, 0.0);
        assert!(check_pptr_indices("open", &clip.tracks, 2).is_ok());
        let e = check_pptr_indices("open", &clip.tracks, 1).unwrap_err();
        assert!(
            matches!(e, Error::Invalid(_)) && e.to_string().contains("no index"),
            "{e}"
        );
    }

    fn no_pptr(_: &Value) -> Result<Option<ObjRef>> {
        unreachable!("no PPtr curves")
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
