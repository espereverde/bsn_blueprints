//! The `*.bp.ron` asset loader.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoadError, AssetLoader, AssetPath, LoadContext, LoadDirectError};
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::reflect::TypeRegistryArc;
use bevy::scene::ScenePatch;
use ron2::error::Span;

use crate::blueprint::Node;
use crate::error::{BlueprintError, ErrorKind};
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
    type Error = BlueprintError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<BlueprintFile, BlueprintError> {
        let here = load_context.path().clone_owned();
        self.load_file(reader, &here, load_context)
            .await
            .map_err(|e| e.in_file(&here))
    }

    fn extensions(&self) -> &[&str] {
        &["bp.ron"]
    }
}

impl BlueprintLoader {
    async fn load_file(
        &self,
        reader: &mut dyn Reader,
        here: &AssetPath<'static>,
        load_context: &mut LoadContext<'_>,
    ) -> Result<BlueprintFile, BlueprintError> {
        let read_error = |e: &dyn std::fmt::Display| BlueprintError::new(ErrorKind::Read, e.to_string());
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.map_err(|e| read_error(&e))?;

        // Handle fields (`Path("image.png")`) become handles and dependencies of this file.
        let source = String::from_utf8(bytes).map_err(|e| read_error(&e))?;
        let raw = parse_file(&source, &self.registry.read(), load_context)?;
        let parent_files = load_parent_files(&raw, here, load_context).await?;
        let entries = Flattener::new(here, &raw, &parent_files, &self.registry.read()).flatten_all()?;

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
}

/// Loads the other blueprint files that parents live in. They become loader dependencies, so
/// editing a parent file hot-reloads this one too.
async fn load_parent_files(
    raw: &HashMap<String, RawNode>,
    here: &AssetPath<'static>,
    load_context: &mut LoadContext<'_>,
) -> Result<HashMap<AssetPath<'static>, BlueprintFile>, BlueprintError> {
    let mut all_extends = Vec::new();
    for entry in raw.values() {
        entry.all_extends(&mut all_extends);
    }
    let mut files = HashMap::default();
    for (extends, span) in all_extends {
        let file = here
            .resolve_embed_str(extends)
            .map_err(|e| BlueprintError::new(ErrorKind::Inheritance, e.to_string()).at(&span))?
            .without_label()
            .into_owned();
        if file.path() != here.path() && !files.contains_key(&file) {
            let loaded = load_context
                .load_builder()
                .load_value::<BlueprintFile>(file.clone())
                .await
                .map_err(|e| parent_error(e, &span))?;
            files.insert(file, loaded.take());
        }
    }
    Ok(files)
}

/// The error of a parent file that failed to load: its own [`BlueprintError`] (located in that
/// file) when it has one, else Bevy's, placed at the reference to it.
fn parent_error(error: LoadDirectError, span: &Span) -> BlueprintError {
    if let LoadDirectError::LoadError {
        error: AssetLoadError::AssetLoaderError(error),
        ..
    } = &error
        && let Some(error) = error.error().downcast_ref::<BlueprintError>()
    {
        return error.clone();
    }
    BlueprintError::new(ErrorKind::Inheritance, error.to_string()).at(span)
}
