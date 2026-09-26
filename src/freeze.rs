//! Frozen blueprints: a copy of a blueprint with every random choice made once.

use std::sync::Arc;

use bevy::asset::{AssetServer, Assets, Handle};
use bevy::ecs::world::World;
use bevy::prelude::Result;
use bevy::reflect::{GetPath, Reflect, TypeRegistry};
use bevy::scene::{ResolvedSceneRoot, ScenePatch};

use crate::blueprint::{Blueprint, BlueprintComponent, Node};
use crate::error::{BlueprintError, ErrorKind};
use crate::spawn::BlueprintScene;

impl BlueprintComponent {
    /// The value to insert when spawning: random fields sampled, frozen fields filled with
    /// freshly frozen blueprints, references to other files' blueprints filled in.
    pub(crate) fn spawn_value(&self, registry: &TypeRegistry, world: &mut World) -> Result<Box<dyn Reflect>> {
        let mut value = self.sample(registry)?;
        for frozen in &self.frozen {
            let handle = freeze_to_scene(&frozen.node, registry, world)?;
            *value
                .path_mut::<Handle<ScenePatch>>(frozen.path.as_str())
                .map_err(|e| BlueprintError::new(ErrorKind::Reference, format!("frozen field `{}`: {e}", frozen.path)))? =
                handle;
        }
        for blueprint_ref in &self.refs {
            blueprint_ref.fill(&mut *value, world)?;
        }
        Ok(value)
    }
}

impl Blueprint {
    /// A copy with every random choice made, children included.
    fn freeze(&self, registry: &TypeRegistry, world: &mut World) -> Result<Blueprint> {
        let chosen = self.choose(true);
        let mut frozen = Blueprint::default();
        for component in chosen.components {
            frozen.components.push(BlueprintComponent {
                type_id: component.type_id,
                value: Arc::from(component.spawn_value(registry, world)?),
                random: Vec::new(),
                frozen: Vec::new(),
                refs: Vec::new(),
                recipe: component.recipe.clone(),
            });
        }
        for (name, child) in chosen.children {
            let child = child.freeze(registry, world)?;
            frozen.children.push((name.to_string(), Node::Fixed(Arc::new(child))));
        }
        Ok(frozen)
    }
}

/// Freezes `node` and adds it as a ready-to-spawn scene. The scene is freed when the last handle
/// to it (normally the owner's component) is dropped.
fn freeze_to_scene(node: &Node, registry: &TypeRegistry, world: &mut World) -> Result<Handle<ScenePatch>> {
    let blueprint = node
        .choose()
        .ok_or_else(|| BlueprintError::new(ErrorKind::Random, "a frozen blueprint can't be Maybe(..)"))?;
    let frozen = Node::Fixed(Arc::new(blueprint.freeze(registry, world)?));
    let resolved = ResolvedSceneRoot::resolve(
        Box::new(BlueprintScene(frozen)),
        world.resource::<AssetServer>(),
        world.resource::<Assets<ScenePatch>>(),
    )?;
    Ok(world.resource_mut::<Assets<ScenePatch>>().add(ScenePatch {
        scene: None,
        dependencies: Vec::new(),
        resolved: Some(Arc::new(resolved)),
    }))
}
