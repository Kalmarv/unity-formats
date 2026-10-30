//! Lists a Unity player's scenes, reads a few of its meshes and, for a Universal Render Pipeline player, its default
//! renderer.
//!
//! `cargo run --example inspect_player -- <the game's *_Data folder> <a type tree package (.tpk)> [<a bundle>...]`
//!
//! Each bundle's serialized files are read as well (a bundle that holds the player's own files, like Big Walk's
//! `data.unity3d`, or Addressables bundles); a bundle of many levels would better be read one level at a time with
//! `NodeSelection::Closure`.

use std::path::{Path, PathBuf};

use unity_formats::TypeTreePackage;
use unity_formats::listing::list;
use unity_formats::mesh::UnityMeshData;
use unity_formats::pipeline::{read_urp_feature_active, read_urp_renderer};
use unity_formats::player::{BundleSpec, NodeSelection, Player};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(data), Some(package)) = (args.next(), args.next()) else {
        eprintln!(
            "usage: inspect_player <the game's *_Data folder> <type tree package (.tpk)> [<a bundle>...]"
        );
        return Ok(());
    };
    let bundles: Vec<PathBuf> = args.map(PathBuf::from).collect();
    let specs: Vec<BundleSpec> = bundles
        .iter()
        .map(|path| BundleSpec {
            path,
            nodes: NodeSelection::All,
        })
        .collect();
    let package = TypeTreePackage::parse(&std::fs::read(package)?)?;
    let player = Player::open_with_bundles(Path::new(&data), package, &specs)?;
    println!("{} serialized files", player.files.len());
    for scene in player.scenes()? {
        println!("scene {}: {} ({})", scene.index, scene.name, scene.file);
    }
    let meshes = list(&player, Some(&["Mesh"]))?;
    println!("{} meshes", meshes.entries.len());
    for entry in meshes.entries.iter().take(5) {
        let mesh = UnityMeshData::read(&player, &entry.object)?;
        println!(
            "  {}: {} vertices, {} indices in {} submeshes",
            mesh.name,
            mesh.vertex_count,
            mesh.indices.len(),
            mesh.submeshes.len()
        );
    }
    // the Universal Render Pipeline asset every quality level draws with, and the renderer its cameras use by default
    if let Some(asset) = player.urp_asset(None)? {
        let settings = player.urp_settings(&asset)?;
        let default = settings
            .renderers
            .get(usize::try_from(settings.default_renderer)?);
        if let Some(r) = default
            .map(|p| player.resolve_pptr(&asset.file, *p))
            .transpose()?
            .flatten()
        {
            let renderer = read_urp_renderer(&player.behaviour(&r)?.1)?;
            println!(
                "URP renderer {}/{}: rendering mode {}, depth priming {}, copy depth {}",
                r.file,
                r.path_id,
                renderer.rendering_mode,
                renderer.depth_priming_mode,
                renderer.copy_depth_mode
            );
            for p in &renderer.features {
                let Some(f) = player.resolve_pptr(&r.file, *p)? else {
                    continue;
                };
                let active = read_urp_feature_active(&player.behaviour(&f)?.1)?;
                let class = player.script_class(&f)?.unwrap_or_default();
                println!("  feature {class}{}", if active { "" } else { " (off)" });
            }
        }
    }
    Ok(())
}
