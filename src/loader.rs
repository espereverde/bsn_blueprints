//! The `*.bp.ron` asset loader.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AssetPath, LoadContext};
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use bevy::scene::ScenePatch;

use crate::BoxError;
use crate::blueprint::Node;
use crate::flatten::Flattener;
use crate::parse::{RawNode, parse_file};
use crate::spawn::BlueprintScene;

/// Root asset of a `*.bp.ron` file: every blueprint in it, flattened, and its spawnable scene.
///
/// Load the file and get blueprints from it with [`BlueprintFile::get`]. Loading
/// `"file.bp.ron#label"` paths directly also works, but Bevy loads the whole file once for each
/// label requested before the file has loaded.
#[derive(Asset, TypePath, Default)]
pub struct BlueprintFile {
    pub(crate) entries: HashMap<String, Node>,
    /// Strong handles to the labeled scenes, so they live as long as the file.
    scenes: HashMap<String, Handle<ScenePatch>>,
}

impl BlueprintFile {
    /// The scene of the blueprint `label` in this file, ready to spawn.
    pub fn get(&self, label: &str) -> Option<&Handle<ScenePatch>> {
        self.scenes.get(label)
    }

    /// The labels of all blueprints in this file.
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.scenes.keys().map(String::as_str)
    }
}

/// Loads a `*.bp.ron` file and publishes each blueprint as a labeled `ScenePatch`.
#[derive(TypePath)]
pub struct BlueprintLoader {
    registry: TypeRegistryArc,
}

impl FromWorld for BlueprintLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for BlueprintLoader {
    type Asset = BlueprintFile;
    type Settings = ();
    type Error = BoxError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<BlueprintFile, BoxError> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        // Handle fields (`Path("image.png")`) become handles and dependencies of this file.
        let source = String::from_utf8(bytes)?;
        let raw = parse_file(&source, &self.registry.read(), load_context)?;
        let here = load_context.path().clone_owned();
        let parent_files = load_parent_files(&raw, &here, load_context).await?;
        let entries = Flattener::new(&here, &raw, &parent_files, &self.registry.read()).flatten_all()?;

        let mut scenes = HashMap::default();
        for (label, node) in &entries {
            // Blueprint files its references point to must load before it can spawn.
            let mut dependencies = Vec::new();
            node.referenced_files(&mut dependencies);
            let scene = load_context.add_labeled_asset(
                label.clone(),
                ScenePatch {
                    scene: Some(Box::new(BlueprintScene(node.clone()))),
                    dependencies,
                    resolved: None,
                },
            );
            scenes.insert(label.clone(), scene);
        }
        Ok(BlueprintFile { entries, scenes })
    }

    fn extensions(&self) -> &[&str] {
        &["bp.ron"]
    }
}

/// Loads the other blueprint files that parents live in. They become loader dependencies, so
/// editing a parent file hot-reloads this one too.
async fn load_parent_files(
    raw: &HashMap<String, RawNode>,
    here: &AssetPath<'static>,
    load_context: &mut LoadContext<'_>,
) -> Result<HashMap<AssetPath<'static>, BlueprintFile>, BoxError> {
    let mut all_extends = Vec::new();
    for entry in raw.values() {
        entry.all_extends(&mut all_extends);
    }
    let mut files = HashMap::default();
    for extends in all_extends {
        let file = here.resolve_embed_str(extends)?.without_label().into_owned();
        if file.path() != here.path() && !files.contains_key(&file) {
            let loaded = load_context
                .load_builder()
                .load_value::<BlueprintFile>(file.clone())
                .await?;
            files.insert(file, loaded.take());
        }
    }
    Ok(files)
}
