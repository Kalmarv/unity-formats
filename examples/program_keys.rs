//! Lists the key and names of every D3D11 program of a Unity player's Shaders, so a frame captured from the running
//! game can name the programs, constants and textures Unity's stripped bytecode does not.
//!
//! ```text
//! cargo run --release --example program_keys -- <the game's *_Data folder> <a type tree package (.tpk)> > programs.jsonl
//! ```
//!
//! Each line written is one variant's program as JSON: `key` (XXH3-128 of the DXBC container, as 32 lowercase hex
//! digits: the key locum names captured bytes by), `bytes`, `name` (`<shader> <subshader>.<pass> <stage>
//! [<keywords>]`), and its `constant_buffers` and `bindings` in the shape of locum's shader reflection;
//! `locum-cli index --programs programs.jsonl` reads it. A variable takes its type from Unity's record: a vector or
//! scalar as declared, a struct with its members, and a matrix as one `float4` per register it takes (`name[i]` is
//! register i), since Unity's records keep no packing order. A texture's sampler is named `sampler<texture>` at the
//! register its record gives; an inline sampler state (`sampler_linear_clamp`), which the records keep without its
//! name, is left out.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value as Json, json};
use unity_formats::TypeTreePackage;
use unity_formats::listing::list;
use unity_formats::player::Player;
use unity_formats::program::{
    D3D11_PROGRAM_TYPES, Field, Parameters, Segments, ShaderPrograms, Shape, Stage, dxbc,
};
use xxhash_rust::xxh3::xxh3_128;

/// Unity's `ShaderCompilerPlatform` of D3D11.
const D3D11: i64 = 4;

/// D3D_SHADER_VARIABLE_TYPE of Unity's `ShaderParamType` (0 float, 1 int, 2 bool, 3 half, 4 short, 5 uint): a half
/// and a short take a 32-bit lane in a constant buffer.
fn base(kind: i64) -> u16 {
    match kind {
        1 | 4 => 2,
        2 => 1,
        5 => 19,
        _ => 3,
    }
}

/// A field's type (locum's `rdef::Type`) and its size in bytes.
fn field_type(f: &Field) -> (Json, u32) {
    let ty = |class: u16, columns: u32, elements: u32| json!({"class": class, "base": base(f.kind), "rows": 1, "columns": columns, "elements": elements});
    match f.shape {
        Shape::Vector { columns } => {
            let class = if columns == 1 { 0 } else { 1 };
            // an array's elements each start a register; the last takes only its lanes
            let size = f.array_size.saturating_sub(1) * 16 + columns * 4;
            (ty(class, columns, f.array_size), size)
        }
        Shape::Matrix { rows } => {
            let registers = rows * f.array_size.max(1);
            (ty(1, 4, registers), registers * 16)
        }
    }
}

fn stage_name(s: Stage) -> &'static str {
    match s {
        Stage::Vertex => "vertex",
        Stage::Fragment => "fragment",
        Stage::Geometry => "geometry",
        Stage::Hull => "hull",
        Stage::Domain => "domain",
    }
}

/// The constant buffers and bindings of a variant's parameters, as locum's reflection has them; and how many fields
/// overlap the next one (a sign the sizes here are wrong).
fn reflection(p: &Parameters) -> (Json, Json, usize) {
    let mut overlaps = 0;
    let mut buffers = Vec::new();
    let mut bindings = Vec::new();
    for cb in &p.constant_buffers {
        let mut variables: Vec<(u32, u32, Json)> = cb
            .fields
            .iter()
            .map(|f| {
                let (ty, size) = field_type(f);
                (
                    f.offset,
                    size,
                    json!({"name": f.name, "offset": f.offset, "size": size, "used": true, "type": ty}),
                )
            })
            .collect();
        for s in &cb.structs {
            let members: Vec<Json> = s
                .members
                .iter()
                .map(|m| json!({"name": m.name, "offset": m.offset, "type": field_type(m).0}))
                .collect();
            let size = s.size * s.array_size.max(1);
            let ty = json!({"class": 5, "base": 0, "rows": 1, "columns": 1, "elements": s.array_size, "members": members});
            variables.push((
                s.offset,
                size,
                json!({"name": s.name, "offset": s.offset, "size": size, "used": true, "type": ty}),
            ));
        }
        variables.sort_by_key(|v| v.0);
        overlaps += variables.windows(2).filter(|w| w[0].0 + w[0].1 > w[1].0).count();
        buffers.push(json!({
            "name": cb.name,
            "slot": cb.slot,
            "space": 0,
            "size": cb.size,
            "variables": variables.into_iter().map(|v| v.2).collect::<Vec<_>>(),
        }));
        if let Some(slot) = cb.slot {
            bindings.push(json!({"name": cb.name, "kind": 0, "slot": slot, "count": 1, "space": 0}));
        }
    }
    for t in &p.textures {
        bindings.push(json!({"name": t.name, "kind": 2, "slot": t.slot, "count": 1, "space": 0}));
    }
    // a texture's own sampler, at the register its record gives (the inline sampler states `samplers` lists have
    // no name)
    let mut named = HashSet::new();
    for t in &p.textures {
        if let Some(slot) = t.sampler.filter(|s| named.insert(*s)) {
            bindings.push(json!({"name": format!("sampler{}", t.name), "kind": 3, "slot": slot, "count": 1, "space": 0}));
        }
    }
    (Json::Array(buffers), Json::Array(bindings), overlaps)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(data), Some(package)) = (args.next(), args.next()) else {
        eprintln!(
            "usage: program_keys <the game's *_Data folder> <type tree package (.tpk)> > programs.jsonl"
        );
        return Ok(());
    };
    let package = TypeTreePackage::parse(&std::fs::read(package)?)?;
    let player = Player::open(Path::new(&data), package)?;
    let shaders = list(&player, Some(&["Shader"]))?;
    let (mut written, mut failed, mut no_d3d11, mut overlapping) = (0usize, 0usize, 0usize, 0usize);
    let mut seen = HashSet::new();
    for entry in &shaders.entries {
        let r = &entry.object;
        let programs = match ShaderPrograms::read(&player, r) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{}/{}: {e}", r.file, r.path_id);
                failed += 1;
                continue;
            }
        };
        if !programs.platforms.contains(&D3D11) {
            no_d3d11 += 1;
            continue;
        }
        let mut cache = Segments::default();
        for (sub, passes) in programs.passes.iter().enumerate() {
            for (n, pass) in passes.iter().enumerate() {
                for stage in Stage::ALL {
                    let program = pass.stage(stage);
                    for v in &program.variants {
                        if !D3D11_PROGRAM_TYPES.contains(&v.gpu_program_type) {
                            continue;
                        }
                        let code = match programs.code_with(&mut cache, D3D11, v.blob_index) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("{}: {e}", programs.name);
                                failed += 1;
                                continue;
                            }
                        };
                        let Some(container) = dxbc(&code) else {
                            continue;
                        };
                        let mut name = format!("{} {sub}.{n} {}", programs.name, stage_name(stage));
                        if !v.keywords.is_empty() {
                            name += &format!(" [{}]", v.keywords.join(" "));
                        }
                        let key = format!("{:032x}", xxh3_128(container));
                        if !seen.insert((key.clone(), name.clone())) {
                            continue;
                        }
                        let parameters = match programs.variant_parameters(&mut cache, D3D11, program, v) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("{name}: {e}");
                                failed += 1;
                                continue;
                            }
                        };
                        let (constant_buffers, bindings, overlaps) = reflection(&parameters);
                        overlapping += overlaps;
                        println!(
                            "{}",
                            json!({
                                "key": key,
                                "bytes": container.len(),
                                "name": name,
                                "constant_buffers": constant_buffers,
                                "bindings": bindings,
                            })
                        );
                        written += 1;
                    }
                }
            }
        }
    }
    eprintln!(
        "{} shaders: {written} programs written; {failed} could not be read; {no_d3d11} without D3D11 programs; \
         {overlapping} variables overlap the next",
        shaders.entries.len()
    );
    Ok(())
}
