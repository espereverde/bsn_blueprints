//! Spawning blueprints by file and label, without waiting for the file in game code.

use std::borrow::Cow;

use bevy::asset::LoadState;
use bevy::prelude::*;
use bevy::scene::ScenePatchInstance;

use crate::loader::BlueprintFile;

/// Spawns the blueprint `label` of `file` on this entity, as soon as the file has loaded.
///
/// Add it like any component, or use [`BlueprintCommandsExt::spawn_blueprint`] and
/// [`BlueprintEntityCommandsExt::insert_blueprint`]. If the file is already loaded when the
/// entity is spawned during `Update`, the blueprint's components are there by the end of that
/// frame; otherwise they arrive once the file (and every blueprint file it refers to) has loaded.
///
/// ```ignore
/// commands.spawn((BlueprintInstance::new(&enemies, "ufo"), Transform::from_xyz(0.0, 50.0, 0.0)));
/// ```
///
/// The blueprint's components are added to the entity, replacing ones it already has. The
/// component stays on the entity as a record of where it came from; changing it later has no
/// effect. If the file fails to load or has no such blueprint, the error is logged and the
/// component removed.
#[derive(Component, Clone, Debug)]
pub struct BlueprintInstance {
    pub file: Handle<BlueprintFile>,
    pub label: Cow<'static, str>,
}

impl BlueprintInstance {
    pub fn new(file: &Handle<BlueprintFile>, label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            file: file.clone(),
            label: label.into(),
        }
    }
}

/// Spawning blueprints from [`Commands`].
pub trait BlueprintCommandsExt {
    /// Spawns an entity with the blueprint `label` of `file` (see [`BlueprintInstance`]).
    fn spawn_blueprint(
        &mut self,
        file: &Handle<BlueprintFile>,
        label: impl Into<Cow<'static, str>>,
    ) -> EntityCommands<'_>;
}

impl BlueprintCommandsExt for Commands<'_, '_> {
    fn spawn_blueprint(
        &mut self,
        file: &Handle<BlueprintFile>,
        label: impl Into<Cow<'static, str>>,
    ) -> EntityCommands<'_> {
        self.spawn(BlueprintInstance::new(file, label))
    }
}

/// Adding blueprints to existing entities.
pub trait BlueprintEntityCommandsExt {
    /// Adds the blueprint `label` of `file` to this entity (see [`BlueprintInstance`]).
    fn insert_blueprint(&mut self, file: &Handle<BlueprintFile>, label: impl Into<Cow<'static, str>>) -> &mut Self;
}

impl BlueprintEntityCommandsExt for EntityCommands<'_> {
    fn insert_blueprint(&mut self, file: &Handle<BlueprintFile>, label: impl Into<Cow<'static, str>>) -> &mut Self {
        self.insert(BlueprintInstance::new(file, label))
    }
}

/// Hands each [`BlueprintInstance`] whose file has loaded to the scene system. Runs just before
/// scenes are spawned, so instances added during `Update` spawn in the same frame.
pub(crate) fn spawn_blueprint_instances(
    mut commands: Commands,
    pending: Query<(Entity, &BlueprintInstance), Without<ScenePatchInstance>>,
    files: Res<Assets<BlueprintFile>>,
    server: Res<AssetServer>,
) {
    for (entity, instance) in &pending {
        let scene = match files.get(&instance.file) {
            Some(file) => match file.get(&instance.label) {
                Some(scene) => scene.clone(),
                None => {
                    let labels: Vec<_> = file.labels().collect();
                    fail(&mut commands, entity, instance, format!("no such blueprint; the file has {labels:?}"));
                    continue;
                }
            },
            None => {
                if let LoadState::Failed(error) = server.load_state(&instance.file) {
                    fail(&mut commands, entity, instance, format!("the file failed to load: {error}"));
                }
                continue;
            }
        };
        commands.entity(entity).insert(ScenePatchInstance(scene));
    }
}

fn fail(commands: &mut Commands, entity: Entity, instance: &BlueprintInstance, reason: String) {
    let path = instance.file.path().map_or_else(|| format!("{:?}", instance.file.id()), ToString::to_string);
    error!("{entity}: can't spawn blueprint `{}` of {path}: {reason}", instance.label);
    commands.entity(entity).remove::<BlueprintInstance>();
}
