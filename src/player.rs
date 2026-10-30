//! A Unity player's data folder: its SerializedFiles by name, pointers (`PPtr`) resolved across them, objects decoded
//! by their classes' type trees.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reader I/O: reads the player's data folder"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::bundle::Bundle;
use crate::resource::StreamingInfo;
use crate::serialized::{MONO_BEHAVIOUR, ObjectInfo, SerializedFile};
use crate::tpk::TypeTreePackage;
use crate::typetree::TypeTree;
use crate::value::{Array, Value};
use crate::{Error, Result, invalid};

/// An object of the player: its file (lower-case name) and path id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ObjRef {
    /// The file's lower-case name.
    pub file: String,
    /// The object's path id in that file.
    pub path_id: i64,
}

/// An object reference as stored (a `PPtr`): the file id, 0 for the file that holds it and otherwise one more than the
/// index of an external file in that file's list, and the object's path id, 0 for a null reference. A script's
/// serialized fields store references this way in 12 bytes; [`Player::resolve_pptr`] finds the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PPtr {
    /// `m_FileID`.
    pub file_id: i32,
    /// `m_PathID`.
    pub path_id: i64,
}

/// The file that holds a player's manager objects (its BuildSettings, GraphicsSettings, QualitySettings and
/// PlayerSettings).
const MANAGERS: &str = "globalgamemanagers";

/// The script class of Universal Render Pipeline's asset (`m_Namespace.m_ClassName` of its MonoScript).
pub const URP_ASSET: &str = "UnityEngine.Rendering.Universal.UniversalRenderPipelineAsset";

/// The render pipeline a player draws with: GraphicsSettings' default pipeline asset (`m_CustomRenderPipeline`), or a
/// quality level's own (`customRenderPipeline`) where it sets one. Which quality level runs is the user's choice, so
/// the pipeline is known only when every level gives the same kind.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RenderPipeline {
    /// No pipeline asset for any quality level: Unity's built-in renderer.
    BuiltIn,
    /// A Universal Render Pipeline asset ([`URP_ASSET`]) for every quality level.
    Universal,
    /// Anything else, or a mix: each level's asset script class (`built-in` for none), sorted, each once.
    Other(Vec<String>),
}

/// How many bone influences a vertex may have under a quality level (`QualitySettings`' `skinWeights`: 1, 2, 4 or
/// 255 for unlimited).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkinWeights {
    /// At most this many influences per vertex (1, 2 or 4).
    Bones(u32),
    /// Every influence a mesh stores.
    Unlimited,
}

impl SkinWeights {
    /// The setting `value` as stored.
    pub fn from_setting(value: i64) -> Result<SkinWeights> {
        match value {
            1 => Ok(SkinWeights::Bones(1)),
            2 => Ok(SkinWeights::Bones(2)),
            4 => Ok(SkinWeights::Bones(4)),
            255 => Ok(SkinWeights::Unlimited),
            v => Err(Error::Unsupported(format!("skinWeights {v}"))),
        }
    }
}

impl std::fmt::Display for SkinWeights {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkinWeights::Bones(1) => write!(f, "1 bone"),
            SkinWeights::Bones(n) => write!(f, "{n} bones"),
            SkinWeights::Unlimited => write!(f, "unlimited"),
        }
    }
}

/// A quality level of the player ([`Player::quality_levels`]): its name, the bone influences a vertex may have and
/// its shadow settings (`shadows`: 0 disabled, 1 hard shadows only, 2 all; the distance shadows are drawn to; the
/// built-in pipeline's cascade count, which a Universal Render Pipeline asset replaces with its own).
#[derive(Clone, Debug, PartialEq)]
pub struct QualityLevel {
    /// The level's name.
    pub name: String,
    /// The bone influences a vertex may have (`skinWeights`).
    pub skin_weights: SkinWeights,
    /// `shadows`: 0 disabled, 1 hard shadows only, 2 all.
    pub shadows: i64,
    /// `shadowDistance`: how far from the camera shadows are drawn.
    pub shadow_distance: f32,
    /// `shadowCascades`: the built-in pipeline's cascade count.
    pub shadow_cascades: i64,
}

/// The colour space a player renders in (PlayerSettings' `m_ActiveColorSpace`, a project setting the build stores): in
/// `Linear` the engine converts colour properties from their gamma-encoded storage to linear before a program reads
/// them and samples sRGB textures as linear; in `Gamma` it hands both to programs as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ColourSpace {
    /// `m_ActiveColorSpace` 0.
    Gamma,
    /// `m_ActiveColorSpace` 1.
    Linear,
}

/// A scene of the build ([`Player::scenes`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildScene {
    /// Its place in the build's scene list.
    pub index: usize,
    /// The file it is stored in (`level<index>`).
    pub file: String,
    /// The path the editor knew it by (`Assets/Scenes/Main Mountain Scene.unity`), as stored.
    pub path: String,
    /// The scene's name: its path's file name without `.unity`.
    pub name: String,
}

/// Which of a bundle's serialized files a player reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeSelection<'a> {
    /// Every serialized node of the bundle.
    All,
    /// The nodes with these paths, and every serialized node of the listed bundles that their external files name
    /// (transitively): the files a level needs, not the other levels of its bundle.
    Closure(&'a [&'a str]),
}

/// A bundle to read into a player, with the files wanted from it.
#[derive(Clone, Copy, Debug)]
pub struct BundleSpec<'a> {
    /// The bundle file.
    pub path: &'a Path,
    /// Which of its serialized files to read.
    pub nodes: NodeSelection<'a>,
}

/// A Unity player's data folder, read: every SerializedFile in it (and in its `Resources` folder), the serialized
/// files of the bundles it was opened with ([`Player::open_with_bundles`]), and the type tree package that decodes
/// their objects. Objects are named by [`ObjRef`] and decoded on demand.
#[derive(Debug)]
pub struct Player {
    /// The data folder.
    pub dir: PathBuf,
    /// By lower-case file name: `level1`, `sharedassets1.assets`, `unity default resources`, ...; a bundled file by
    /// the last segment of its node's path (`buildplayer-level_5.sharedassets`).
    pub files: BTreeMap<String, SerializedFile>,
    /// The folder each loose file is in (its streamed resources are beside it); a bundled file has none.
    pub folders: BTreeMap<String, PathBuf>,
    /// The bundles files were read from, in the order given.
    pub bundles: Vec<Arc<Bundle>>,
    package: TypeTreePackage,
    trees: Mutex<BTreeMap<(String, i32), std::sync::Arc<TypeTree>>>,
}

fn file_key(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_lowercase()
}

impl Player {
    /// The serialized files of `dir` (the `<Game>_Data` folder) and of its `Resources` folder, read with `package`'s
    /// class layouts. Files of other kinds (resources, bundles, configs) are skipped.
    ///
    /// A file whose header is a SerializedFile's of a version this crate does not read is refused, not left out: a
    /// player with such a file would otherwise read as one without it. Two files the player would name alike (they
    /// differ by case, or one is in the data folder and one in `Resources`) are refused too.
    pub fn open(dir: &Path, package: TypeTreePackage) -> Result<Self> {
        Self::open_with_bundles(dir, package, &[])
    }

    /// As [`Player::open`], with the serialized files of `bundles` as well: a Big Walk style `data.unity3d` that holds
    /// the player's own files, or a level's AssetBundle beside the loose ones. A bundled file is named like a loose one,
    /// by the lower-cased last segment of its node's path, and is refused the same way when another file has the name.
    /// Its objects are read from the bundle by range as they are asked for, so a selection of a bundle costs the
    /// metadata of its files. A file that stores its type trees decodes with them; others with the package.
    pub fn open_with_bundles(dir: &Path, package: TypeTreePackage, bundles: &[BundleSpec]) -> Result<Self> {
        let mut files = BTreeMap::new();
        let mut folders = BTreeMap::new();
        let mut paths_of: BTreeMap<String, PathBuf> = BTreeMap::new();
        for d in [dir.to_path_buf(), dir.join("Resources")] {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            paths.sort();
            for p in paths {
                let bytes = std::fs::read(&p).map_err(|source| Error::Io {
                    path: p.clone(),
                    source,
                })?;
                match SerializedFile::container_version(&bytes) {
                    None => continue,
                    Some(v) if !crate::serialized::VERSIONS.contains(&v) => {
                        return Err(Error::Unsupported(format!(
                            "{}: SerializedFile version {v} (read: {}..={})",
                            p.display(),
                            crate::serialized::VERSIONS.start(),
                            crate::serialized::VERSIONS.end()
                        )));
                    }
                    Some(_) => {}
                }
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if let Some(first) = paths_of.insert(name.clone(), p.clone()) {
                    return Err(invalid(format!(
                        "{} and {} are both the player's file {name:?}",
                        first.display(),
                        p.display()
                    )));
                }
                let f = SerializedFile::parse_with(bytes, Some(&package))
                    .map_err(|e| invalid(format!("{}: {e}", p.display())))?;
                folders.insert(name.clone(), d.clone());
                files.insert(name, f);
            }
        }
        let opened = bundles
            .iter()
            .map(|spec| Bundle::open(spec.path).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        // every serialized node of the bundles by the name a pointer gives it
        let mut by_key: BTreeMap<String, Vec<(usize, &str)>> = BTreeMap::new();
        for (i, b) in opened.iter().enumerate() {
            for n in b.nodes().iter().filter(|n| n.is_serialized()) {
                by_key
                    .entry(file_key(&n.path))
                    .or_default()
                    .push((i, n.path.as_str()));
            }
        }
        let mut pending: Vec<String> = Vec::new();
        for (spec, b) in bundles.iter().zip(&opened) {
            match spec.nodes {
                NodeSelection::All => pending.extend(
                    b.nodes()
                        .iter()
                        .filter(|n| n.is_serialized())
                        .map(|n| file_key(&n.path)),
                ),
                NodeSelection::Closure(paths) => {
                    for path in paths {
                        let n = b.node(path).filter(|n| n.is_serialized()).ok_or_else(|| {
                            invalid(format!("{}: no serialized node {path:?}", spec.path.display()))
                        })?;
                        pending.push(file_key(&n.path));
                    }
                }
            }
        }
        // a file read brings the files its pointers name, where the listed bundles hold them
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(key) = pending.pop() {
            if !seen.insert(key.clone()) {
                continue;
            }
            let Some(candidates) = by_key.get(&key) else {
                continue;
            };
            if let Some(loose) = paths_of.get(&key) {
                return Err(invalid(format!(
                    "{} and {candidates:?} are both the player's file {key:?}",
                    loose.display()
                )));
            }
            let [(b, path)] = candidates.as_slice() else {
                return Err(invalid(format!(
                    "{} bundled files are all the player's file {key:?}: {candidates:?}",
                    candidates.len()
                )));
            };
            let (b, path) = (*b, *path);
            let bundle = opened.get(b).ok_or_else(|| invalid("a bundle went missing"))?;
            let f = SerializedFile::from_bundle(bundle, path, Some(&package))?;
            pending.extend(f.externals.iter().map(|e| file_key(&e.path)));
            files.insert(key, f);
        }
        Ok(Player {
            dir: dir.to_path_buf(),
            files,
            folders,
            bundles: opened,
            package,
            trees: Mutex::new(BTreeMap::new()),
        })
    }

    /// The serialized file called `name` (lower case), or an error naming it.
    pub fn file(&self, name: &str) -> Result<&SerializedFile> {
        self.files
            .get(name)
            .ok_or_else(|| invalid(format!("no serialized file {name:?} in the player")))
    }

    /// The class id of an object (`None`: no such object).
    pub fn class_id(&self, r: &ObjRef) -> Option<i32> {
        self.files.get(&r.file)?.object(r.path_id).map(|o| o.class_id)
    }

    /// The class name of an object: the root of the type tree its file stores for the object's type, else the type tree
    /// package's name for its class id.
    pub fn class_name(&self, r: &ObjRef) -> Option<&str> {
        let f = self.files.get(&r.file)?;
        let o = f.object(r.path_id)?;
        match f.tree_of(o).and_then(|t| t.root()) {
            Some(root) => Some(&*root.ty),
            None => self.package.class_name(o.class_id, f.unity),
        }
    }

    /// The type tree that decodes `o` of `f`: the one the file stores for the object's type; the package's for a file
    /// that stores none, and for a MonoBehaviour, whose stored tree holds its script's fields as well (see
    /// [`Player::script_fields`]) where the package's holds the base fields every MonoBehaviour has.
    fn tree(&self, file: &str, f: &SerializedFile, o: &ObjectInfo) -> Result<Arc<TypeTree>> {
        let stored = f.tree_of(o);
        if o.class_id != MONO_BEHAVIOUR
            && let Some(t) = stored
        {
            return Ok(t.clone());
        }
        let key = (f.unity_version.clone(), o.class_id);
        // an entry is inserted whole, so a cache left by a thread that panicked is still valid
        let mut cache = self
            .trees
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = cache.get(&key) {
            return Ok(t.clone());
        }
        match self.package.type_tree(o.class_id, f.unity) {
            Ok(t) => {
                let t = Arc::new(t);
                cache.insert(key, t.clone());
                Ok(t)
            }
            // the package lacks the class for this Unity version, the file does not
            Err(_) if stored.is_some() => stored.cloned().ok_or_else(|| invalid("a type tree went missing")),
            Err(e) => Err(invalid(format!("{file}: {e}"))),
        }
    }

    /// Decode an object (its type tree must cover it; a MonoBehaviour's base fields only).
    pub fn decode(&self, r: &ObjRef) -> Result<Value> {
        let f = self.file(&r.file)?;
        let o = f
            .object(r.path_id)
            .ok_or_else(|| invalid(format!("{}: no object {}", r.file, r.path_id)))?;
        let d = f.decode_with(o, &*self.tree(&r.file, f, o)?)?;
        if !d.covered(o.class_id) {
            return Err(invalid(format!(
                "{}/{} (class {}): {} bytes after its type tree",
                r.file, r.path_id, o.class_id, d.rest
            )));
        }
        Ok(d.value)
    }

    /// A MonoBehaviour decoded whole, its script's fields with its base fields, by the type tree its file stores for
    /// its script's type. `None` for a file that stores no type trees (a player's: the script's fields are
    /// [`Player::behaviour`]'s bytes there) and for an object of another class. The tree must cover the object.
    pub fn script_fields(&self, r: &ObjRef) -> Result<Option<Value>> {
        let f = self.file(&r.file)?;
        let o = f
            .object(r.path_id)
            .ok_or_else(|| invalid(format!("{}: no object {}", r.file, r.path_id)))?;
        let Some(tree) = f.tree_of(o).filter(|_| o.class_id == MONO_BEHAVIOUR) else {
            return Ok(None);
        };
        let d = f.decode_with(o, tree)?;
        if d.rest != 0 {
            return Err(invalid(format!(
                "{}/{} (class {}): {} bytes after its stored type tree",
                r.file, r.path_id, o.class_id, d.rest
            )));
        }
        Ok(Some(d.value))
    }

    /// The field `name` of the object's root struct alone ([`TypeTree::decode_field`]): `None` when its class has no
    /// such field. The object need not decode whole, and a field stored early (`m_Name`) is read without the rest.
    pub fn field(&self, r: &ObjRef, name: &str) -> Result<Option<Value>> {
        let f = self.file(&r.file)?;
        let o = f
            .object(r.path_id)
            .ok_or_else(|| invalid(format!("{}: no object {}", r.file, r.path_id)))?;
        self.tree(&r.file, f, o)?
            .decode_field(&f.bytes(o)?, f.big_endian, name)
            .map_err(|e| invalid(format!("{}/{} (class {}): {e}", r.file, r.path_id, o.class_id)))
    }

    /// The bytes of a streamed resource (`.resS`, `.resource`) that an object of `file` names: from the file beside the
    /// loose file, or from the node of its bundle that the path's last segment names.
    pub fn read_stream(&self, file: &str, info: &StreamingInfo) -> Result<Vec<u8>> {
        if let Some((bundle, _)) = self.file(file)?.bundle() {
            return info.read_bundled(bundle);
        }
        let dir = self
            .folders
            .get(file)
            .ok_or_else(|| invalid(format!("no folder for {file}")))?;
        info.read(dir)
    }

    /// The scenes of the build, in `BuildSettings`' order: scene `i` is the file `level<i>`. `Ok(empty)` for a player
    /// without a `globalgamemanagers` or a `BuildSettings` (a player is never read for scenes it does not list).
    pub fn scenes(&self) -> Result<Vec<BuildScene>> {
        let Some(r) = self.manager_object("BuildSettings")? else {
            return Ok(Vec::new());
        };
        let list = self
            .field(&r, "scenes")?
            .ok_or_else(|| invalid(format!("{MANAGERS}: BuildSettings without a scene list")))?;
        let Value::Array(Array::Values(paths)) = list else {
            return Err(invalid(format!(
                "{MANAGERS}: BuildSettings' scenes is not a list"
            )));
        };
        paths
            .iter()
            .enumerate()
            .map(|(index, p)| {
                let path = p
                    .as_str()
                    .ok_or_else(|| invalid(format!("{MANAGERS}: scene {index} is not a path")))?
                    .to_owned();
                let name = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&path)
                    .trim_end_matches(".unity")
                    .to_owned();
                Ok(BuildScene {
                    index,
                    file: format!("level{index}"),
                    path,
                    name,
                })
            })
            .collect()
    }

    /// A MonoBehaviour's base fields and its script's serialized bytes, which a stripped build has no type tree for.
    pub fn behaviour(&self, r: &ObjRef) -> Result<(Value, Vec<u8>)> {
        let f = self.file(&r.file)?;
        let o = f
            .object(r.path_id)
            .ok_or_else(|| invalid(format!("{}: no object {}", r.file, r.path_id)))?;
        if o.class_id != crate::serialized::MONO_BEHAVIOUR {
            return Err(invalid(format!("{}/{}: not a MonoBehaviour", r.file, r.path_id)));
        }
        let d = f.decode_with(o, &*self.tree(&r.file, f, o)?)?;
        let bytes = f.bytes(o)?;
        let script = bytes
            .len()
            .checked_sub(d.rest)
            .and_then(|from| bytes.get(from..))
            .ok_or_else(|| {
                invalid(format!(
                    "{}/{}: {} bytes left of {}",
                    r.file,
                    r.path_id,
                    d.rest,
                    bytes.len()
                ))
            })?;
        Ok((d.value, script.to_vec()))
    }

    /// The pipeline asset each quality level draws with, each with the level's name: its own (`customRenderPipeline`),
    /// else GraphicsSettings' default (`m_CustomRenderPipeline`); one level without a name when the player has no
    /// QualitySettings levels. `None` when the player has no GraphicsSettings.
    #[allow(clippy::type_complexity)]
    fn pipeline_assets(&self) -> Result<Option<Vec<(Option<String>, Option<ObjRef>)>>> {
        let Some(graphics) = self.manager("GraphicsSettings")? else {
            return Ok(None);
        };
        let asset = |pptr: Option<&Value>| -> Result<Option<ObjRef>> {
            Ok(pptr.map(|p| self.resolve(MANAGERS, p)).transpose()?.flatten())
        };
        let default = asset(graphics.get("m_CustomRenderPipeline"))?;
        let mut levels = Vec::new();
        if let Some(Value::Array(crate::value::Array::Values(qs))) = self
            .manager("QualitySettings")?
            .as_ref()
            .and_then(|q| q.get("m_QualitySettings"))
        {
            for q in qs {
                let name = q.get("name").and_then(Value::as_str).map(str::to_owned);
                levels.push((
                    name,
                    asset(q.get("customRenderPipeline"))?.or_else(|| default.clone()),
                ));
            }
        }
        if levels.is_empty() {
            levels.push((None, default));
        }
        Ok(Some(levels))
    }

    /// A pipeline asset's script class (`namespace.class`).
    fn pipeline_class(&self, asset: &ObjRef) -> Result<String> {
        self.script_class(asset)?.ok_or_else(|| {
            invalid(format!(
                "{}/{}: pipeline asset without a script",
                asset.file, asset.path_id
            ))
        })
    }

    /// The player's [`RenderPipeline`], from `globalgamemanagers`' GraphicsSettings and QualitySettings; `None` when
    /// the player has no GraphicsSettings.
    pub fn render_pipeline(&self) -> Result<Option<RenderPipeline>> {
        let Some(levels) = self.pipeline_assets()? else {
            return Ok(None);
        };
        let levels: Vec<Option<String>> = levels
            .iter()
            .map(|(_, asset)| asset.as_ref().map(|a| self.pipeline_class(a)).transpose())
            .collect::<Result<_>>()?;
        Ok(Some(if levels.iter().all(Option::is_none) {
            RenderPipeline::BuiltIn
        } else if levels.iter().all(|l| l.as_deref() == Some(URP_ASSET)) {
            RenderPipeline::Universal
        } else {
            let mut names: Vec<String> = levels
                .into_iter()
                .map(|l| l.unwrap_or_else(|| "built-in".into()))
                .collect();
            names.sort();
            names.dedup();
            RenderPipeline::Other(names)
        }))
    }

    /// The Universal Render Pipeline asset the quality level `quality` draws with (every level's when none is named
    /// and they all draw with one); `Ok(None)` when the level (or the levels) draw with no URP asset, or with more
    /// than one where no level is named. An unknown level name is an error.
    pub fn urp_asset(&self, quality: Option<&str>) -> Result<Option<ObjRef>> {
        let Some(levels) = self.pipeline_assets()? else {
            return Ok(None);
        };
        let chosen: Vec<&Option<ObjRef>> = match quality {
            Some(name) => vec![
                levels
                    .iter()
                    .find(|(n, _)| n.as_deref() == Some(name))
                    .map(|(_, a)| a)
                    .ok_or_else(|| invalid(format!("no quality level named {name}")))?,
            ],
            None => levels.iter().map(|(_, a)| a).collect(),
        };
        let mut found: Option<&ObjRef> = None;
        for asset in chosen {
            let Some(asset) = asset else { return Ok(None) };
            if self.pipeline_class(asset)? != URP_ASSET {
                return Ok(None);
            }
            match found {
                Some(f) if f != asset => return Ok(None),
                _ => found = Some(asset),
            }
        }
        Ok(found.cloned())
    }

    /// The settings of the Universal Render Pipeline asset `asset` ([`Player::urp_asset`]), read in the layout its file's
    /// Unity version identifies ([`crate::pipeline::UrpAssetLayout::of`]) before any field is read: a version whose
    /// layout is not verified is refused, naming it.
    pub fn urp_settings(&self, asset: &ObjRef) -> Result<crate::pipeline::UnityUrpSettings> {
        let layout = crate::pipeline::UrpAssetLayout::of(self.file(&asset.file)?.unity)?;
        crate::pipeline::read_urp_settings(&self.behaviour(asset)?.1, layout)
    }

    /// The quality levels of `globalgamemanagers`' QualitySettings, in order, each with its name and the skin weights
    /// it allows; none when the player has no QualitySettings. Which level is current at run time is not data a
    /// consumer can read (see [`Player::build_quality`]), so a consumer states the level it wants where levels differ.
    pub fn quality_levels(&self) -> Result<Vec<QualityLevel>> {
        match self.quality_settings()? {
            Some(settings) => levels_of(&settings),
            None => Ok(Vec::new()),
        }
    }

    /// The name of the level the build was made with: the level `m_CurrentQuality` of the player's QualitySettings
    /// indexes (the three players read so far hold the top or only level). It is what the build states, not what the
    /// game runs at: a game may change its quality level at run time. `None` when the player has no QualitySettings or
    /// states no `m_CurrentQuality`; an index outside the levels is an error. (The per-platform default
    /// `m_PerPlatformDefaultQuality` is not in a player: SkateBIRD, Sledding Game and PEAK carry none.)
    pub fn build_quality(&self) -> Result<Option<String>> {
        let Some(settings) = self.quality_settings()? else {
            return Ok(None);
        };
        current_level(&settings, &levels_of(&settings)?)
    }

    /// The colour space the player renders in: its PlayerSettings' `m_ActiveColorSpace` ([`ColourSpace`]). `None` when
    /// the player has no PlayerSettings; a value other than 0 or 1 (the editor's -1, uninitialised) is an error.
    ///
    /// The field is read alone ([`Player::field`]), not the object whole: for the Unity 6000.3 players read so far
    /// (Sledding Game, PEAK, Big Walk) the type tree package's PlayerSettings layout covers 4 bytes fewer than the
    /// build stores, the shortfall UnityPy's layouts read too, while the fields before the colour space decode alike.
    pub fn colour_space(&self) -> Result<Option<ColourSpace>> {
        let Some(settings) = self.manager_object("PlayerSettings")? else {
            return Ok(None);
        };
        let value = self.field(&settings, "m_ActiveColorSpace")?;
        colour_space_of(value.as_ref()).map(Some)
    }

    /// The decoded QualitySettings of `globalgamemanagers`, `None` when the player has none.
    fn quality_settings(&self) -> Result<Option<Value>> {
        self.manager("QualitySettings")
    }

    /// The one object of the manager class `class` (GraphicsSettings, QualitySettings) in
    /// `globalgamemanagers`, decoded; `None` when the player has no such file or object. Two of one class are an error.
    fn manager(&self, class: &str) -> Result<Option<Value>> {
        self.manager_object(class)?.map(|r| self.decode(&r)).transpose()
    }

    /// The one object of the manager class `class` in `globalgamemanagers` ([`Player::manager`]), not decoded.
    fn manager_object(&self, class: &str) -> Result<Option<ObjRef>> {
        let Some(f) = self.files.get(MANAGERS) else {
            return Ok(None);
        };
        let mut found = None;
        for o in &f.objects {
            let r = ObjRef {
                file: MANAGERS.into(),
                path_id: o.path_id,
            };
            if self.class_name(&r) == Some(class) {
                if found.is_some() {
                    return Err(invalid(format!("{MANAGERS}: more than one {class}")));
                }
                found = Some(r);
            }
        }
        Ok(found)
    }

    /// The object a decoded `PPtr` (`m_FileID`, `m_PathID`) found in `file` points at; `None` for a null pointer.
    pub fn resolve(&self, file: &str, pptr: &Value) -> Result<Option<ObjRef>> {
        let fid = pptr
            .get("m_FileID")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("PPtr without m_FileID"))?;
        let pid = pptr
            .get("m_PathID")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("PPtr without m_PathID"))?;
        let file_id = i32::try_from(fid).map_err(|_| invalid(format!("{file}: PPtr file id {fid}")))?;
        self.resolve_pptr(
            file,
            PPtr {
                file_id,
                path_id: pid,
            },
        )
    }

    /// The object a stored reference found in `file` points at (for one a script's fields store, read as bytes);
    /// `None` for a null reference.
    pub fn resolve_pptr(&self, file: &str, pptr: PPtr) -> Result<Option<ObjRef>> {
        let PPtr { file_id, path_id } = pptr;
        if path_id == 0 {
            return Ok(None);
        }
        let target = if file_id == 0 {
            file.to_owned()
        } else {
            let f = self.file(file)?;
            let ext = file_id
                .checked_sub(1)
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| f.externals.get(i))
                .ok_or_else(|| {
                    invalid(format!(
                        "{file}: PPtr file id {file_id} of {} externals",
                        f.externals.len()
                    ))
                })?;
            file_key(&ext.path)
        };
        Ok(Some(ObjRef {
            file: target,
            path_id,
        }))
    }

    /// The script class of a MonoBehaviour (`namespace.class` of the MonoScript its `m_Script` names, the class alone
    /// for a script without a namespace); `None` when it names no script.
    pub fn script_class(&self, behaviour: &ObjRef) -> Result<Option<String>> {
        let (base, _) = self.behaviour(behaviour)?;
        self.script_class_of(&behaviour.file, &base)
    }

    /// The script class the decoded base fields `base` of a MonoBehaviour in `file` name ([`Player::script_class`]).
    pub(crate) fn script_class_of(&self, file: &str, base: &Value) -> Result<Option<String>> {
        let Some(script) = base
            .get("m_Script")
            .map(|p| self.resolve(file, p))
            .transpose()?
            .flatten()
        else {
            return Ok(None);
        };
        let s = self.decode(&script)?;
        let field = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("");
        Ok(Some(match field("m_Namespace") {
            "" => field("m_ClassName").to_owned(),
            ns => format!("{ns}.{}", field("m_ClassName")),
        }))
    }
}

/// The quality levels a decoded QualitySettings lists, in order.
fn levels_of(settings: &Value) -> Result<Vec<QualityLevel>> {
    let levels = match settings.get("m_QualitySettings") {
        Some(Value::Array(Array::Values(levels))) => levels.as_slice(),
        Some(Value::Array(a)) if a.is_empty() => &[],
        _ => {
            return Err(invalid(format!(
                "{MANAGERS}: QualitySettings without m_QualitySettings"
            )));
        }
    };
    levels
        .iter()
        .map(|q| {
            let name = q
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("a quality level without a name"))?
                .to_owned();
            let weights = q
                .get("skinWeights")
                .and_then(Value::as_i64)
                .ok_or_else(|| invalid(format!("quality level {name}: no skinWeights")))?;
            let field = |k: &str| -> Result<&Value> {
                q.get(k)
                    .ok_or_else(|| invalid(format!("quality level {name}: no {k}")))
            };
            Ok(QualityLevel {
                skin_weights: SkinWeights::from_setting(weights)?,
                shadows: field("shadows")?
                    .as_i64()
                    .ok_or_else(|| invalid(format!("quality level {name}: shadows is not a number")))?,
                shadow_distance: field("shadowDistance")?.as_f32().ok_or_else(|| {
                    invalid(format!("quality level {name}: shadowDistance is not a number"))
                })?,
                shadow_cascades: field("shadowCascades")?.as_i64().ok_or_else(|| {
                    invalid(format!("quality level {name}: shadowCascades is not a number"))
                })?,
                name,
            })
        })
        .collect()
}

/// The colour space PlayerSettings' `m_ActiveColorSpace` states (0 gamma, 1 linear); `None`: the field is absent.
fn colour_space_of(value: Option<&Value>) -> Result<ColourSpace> {
    let value = value.ok_or_else(|| invalid("PlayerSettings without m_ActiveColorSpace"))?;
    match value.as_i64() {
        Some(0) => Ok(ColourSpace::Gamma),
        Some(1) => Ok(ColourSpace::Linear),
        _ => Err(invalid(format!(
            "PlayerSettings: m_ActiveColorSpace {value:?} is neither 0 (gamma) nor 1 (linear)"
        ))),
    }
}

/// The name of the level a decoded QualitySettings' `m_CurrentQuality` indexes among `levels`, `None` where it states none.
fn current_level(settings: &Value, levels: &[QualityLevel]) -> Result<Option<String>> {
    let Some(current) = settings.get("m_CurrentQuality") else {
        return Ok(None);
    };
    let index = current
        .as_i64()
        .ok_or_else(|| invalid("QualitySettings: m_CurrentQuality is not a number"))?;
    usize::try_from(index)
        .ok()
        .and_then(|i| levels.get(i))
        .map(|level| Some(level.name.clone()))
        .ok_or_else(|| {
            invalid(format!(
                "QualitySettings: m_CurrentQuality {index} is not one of its {} levels",
                levels.len()
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn level(name: &str) -> QualityLevel {
        QualityLevel {
            name: name.to_owned(),
            skin_weights: SkinWeights::from_setting(4).expect("a skin weights setting"),
            shadows: 2,
            shadow_distance: 50.0,
            shadow_cascades: 4,
        }
    }

    fn settings(current: Option<Value>) -> Value {
        Value::Struct(
            current
                .map(|v| (Arc::<str>::from("m_CurrentQuality"), v))
                .into_iter()
                .collect(),
        )
    }

    /// The level the build was made with is the one `m_CurrentQuality` indexes; a player that states none has none, and
    /// an index outside the levels is refused, not clamped.
    #[test]
    fn the_build_quality_is_the_level_m_current_quality_indexes() {
        let levels = [level("Low"), level("Mid"), level("High")];
        let name = |v| current_level(&settings(Some(v)), &levels);
        assert_eq!(name(Value::Int(2)).unwrap().as_deref(), Some("High"));
        assert_eq!(name(Value::UInt(0)).unwrap().as_deref(), Some("Low"));
        assert_eq!(current_level(&settings(None), &levels).unwrap(), None);
        for out in [Value::Int(3), Value::Int(-1), Value::UInt(99)] {
            let error = name(out).unwrap_err().to_string();
            assert!(error.contains("m_CurrentQuality"), "{error}");
        }
        assert!(name(Value::String("2".into())).is_err());
        assert!(current_level(&settings(Some(Value::Int(0))), &[]).is_err());
    }

    /// `m_ActiveColorSpace` 0 is gamma and 1 linear; the editor's uninitialised -1, any other number, a value that is no
    /// number and a PlayerSettings without the field are refused, not read as either.
    #[test]
    fn the_colour_space_is_player_settings_active_color_space() {
        let space = |v| colour_space_of(Some(&v));
        assert_eq!(space(Value::Int(0)).unwrap(), ColourSpace::Gamma);
        assert_eq!(space(Value::Int(1)).unwrap(), ColourSpace::Linear);
        assert_eq!(space(Value::UInt(1)).unwrap(), ColourSpace::Linear);
        for refused in [Value::Int(-1), Value::Int(2), Value::String("1".into())] {
            let error = space(refused).unwrap_err().to_string();
            assert!(error.contains("m_ActiveColorSpace"), "{error}");
        }
        assert!(colour_space_of(None).is_err());
    }

    /// A SerializedFile of `version` with no objects, laid out as tests/serialized_headers.rs lays them out: 21 keeps
    /// its sizes in the 20 byte header, later ones in the 48 byte header.
    fn empty_file(version: u32) -> Vec<u8> {
        let mut metadata = b"2019.4.20f1\0".to_vec();
        metadata.extend_from_slice(&19_i32.to_le_bytes());
        metadata.push(0);
        // no types, objects, scripts, externals or reference types
        metadata.extend_from_slice(&[0; 20]);
        metadata.push(0);
        let header = if version < 22 { 20 } else { 48 };
        let size = header + metadata.len();
        let mut out = vec![0; header];
        out[8..12].copy_from_slice(&version.to_be_bytes());
        if version < 22 {
            out[0..4].copy_from_slice(&(metadata.len() as u32).to_be_bytes());
            out[4..8].copy_from_slice(&(size as u32).to_be_bytes());
            out[12..16].copy_from_slice(&(size as u32).to_be_bytes());
        } else {
            out[20..24].copy_from_slice(&(metadata.len() as u32).to_be_bytes());
            out[24..32].copy_from_slice(&(size as u64).to_be_bytes());
            out[32..40].copy_from_slice(&(size as u64).to_be_bytes());
        }
        out.extend(metadata);
        out
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("unity-formats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Resources")).unwrap();
        dir
    }

    /// A file of a container version the crate does not read is refused with its path and version, not left out of the
    /// player; files that are no SerializedFile are still skipped.
    #[test]
    fn a_file_of_an_unread_version_is_refused_not_absent() {
        for version in [20, 23] {
            let dir = scratch(&format!("version{version}"));
            std::fs::write(dir.join("level0"), empty_file(21)).unwrap();
            std::fs::write(dir.join("sharedassets0.assets.resS"), [7u8; 100]).unwrap();
            assert_eq!(
                Player::open(&dir, TypeTreePackage::empty()).unwrap().files.len(),
                1
            );
            std::fs::write(dir.join("level1"), empty_file(version)).unwrap();
            let e = Player::open(&dir, TypeTreePackage::empty()).unwrap_err();
            assert!(matches!(e, Error::Unsupported(_)), "{e}");
            let e = e.to_string();
            assert!(
                e.contains("level1") && e.contains(&format!("version {version}")),
                "{e}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    /// Two files the player names alike are refused, naming both, instead of the later replacing the earlier.
    #[test]
    fn two_files_with_one_name_are_refused() {
        let dir = scratch("duplicate");
        std::fs::write(dir.join("resources.assets"), empty_file(21)).unwrap();
        std::fs::write(dir.join("Resources").join("Resources.assets"), empty_file(22)).unwrap();
        let e = Player::open(&dir, TypeTreePackage::empty())
            .unwrap_err()
            .to_string();
        assert!(e.contains("both the player's file \"resources.assets\""), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
