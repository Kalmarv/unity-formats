//! Lists a Unity player's scenes and reads a few of its meshes.
//!
//! `cargo run --example inspect_player -- <the game's *_Data folder> <a type tree package (.tpk)>`

use std::path::Path;

use unity_formats::TypeTreePackage;
use unity_formats::listing::list;
use unity_formats::mesh::UnityMeshData;
use unity_formats::player::Player;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(data), Some(package)) = (args.next(), args.next()) else {
        eprintln!("usage: inspect_player <the game's *_Data folder> <type tree package (.tpk)>");
        return Ok(());
    };
    let package = TypeTreePackage::parse(&std::fs::read(package)?)?;
    let player = Player::open(Path::new(&data), package)?;
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
    Ok(())
}
