//! Turning blueprints into Bevy scenes, and what happens at each spawn.

use std::alloc::{Layout, dealloc};
use std::ptr::NonNull;
use std::sync::Arc;

use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::template::{Template, TemplateContext};
use bevy::prelude::*;
use bevy::ptr::OwningPtr;
use bevy::reflect::TypeRegistry;
use bevy::scene::{ResolveContext, ResolveSceneError, ResolvedScene, Scene};
use rand::Rng;

use crate::blueprint::{Blueprint, BlueprintComponent, Node, Part};
use crate::error::{BlueprintError, ErrorKind};
use crate::random::choose_weighted;

/// The [`Scene`] stored in each labeled `ScenePatch`.
pub(crate) struct BlueprintScene(pub(crate) Node);

impl Scene for BlueprintScene {
    fn resolve(
        self,
        context: &mut ResolveContext,
        scene: &mut ResolvedScene,
    ) -> Result<(), ResolveSceneError> {
        let Node::Fixed(blueprint) = self.0 else {
            scene.push_bundle_template(NodeTemplate(self.0));
            return Ok(());
        };
        // Fixed children are ordinary related scenes, spawned by the scene system.
        let mut child_scenes = Vec::new();
        for (_, child) in &blueprint.children {
            if let Node::Fixed(_) = child {
                let mut child_scene = ResolvedScene::default();
                BlueprintScene(child.clone()).resolve(context, &mut child_scene)?;
                child_scenes.push(child_scene);
            }
        }
        if !child_scenes.is_empty() {
            scene
                .get_or_insert_related_resolved_scenes::<ChildOf>()
                .scenes
                .extend(child_scenes);
        }
        scene.push_bundle_template(BlueprintTemplate(blueprint));
        Ok(())
    }
}

/// Applies a blueprint on every spawn: its components with this spawn's random choices, and
/// its random children (its fixed children are related scenes).
///
/// Its output is `()`, so the scene system treats it as a bundle template: components are
/// inserted by [`insert_components`] rather than the scene's typed bundle writer.
struct BlueprintTemplate(Arc<Blueprint>);

impl Template for BlueprintTemplate {
    type Output = ();

    fn build_template(&self, context: &mut TemplateContext) -> Result<()> {
        let registry = context.entity.resource::<AppTypeRegistry>().clone();
        let registry = registry.read();
        let chosen = self.0.choose(false);
        insert_components(context.entity, &chosen.components, &registry)?;
        spawn_children(context.entity, &chosen.children, &registry)
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone())
    }
}

/// A blueprint that is itself a random choice (`OneOf` at the top): the chosen blueprint is
/// applied whole, children included.
struct NodeTemplate(Node);

impl Template for NodeTemplate {
    type Output = ();

    fn build_template(&self, context: &mut TemplateContext) -> Result<()> {
        let Some(blueprint) = self.0.choose() else {
            return Ok(());
        };
        let registry = context.entity.resource::<AppTypeRegistry>().clone();
        apply_blueprint(context.entity, blueprint, &registry.read())
    }

    fn clone_template(&self) -> Self {
        Self(self.0.clone())
    }
}

/// What one spawn of a blueprint consists of, once its random choices are made.
pub(crate) struct Chosen<'a> {
    /// At most one per component type; chosen options replace the base values they touch.
    pub(crate) components: Vec<&'a BlueprintComponent>,
    pub(crate) children: Vec<(&'a str, &'a Blueprint)>,
}

impl Blueprint {
    /// Makes this spawn's random choices. Fixed children are only included with
    /// `fixed_children` (otherwise the scene system spawns them).
    pub(crate) fn choose(&self, fixed_children: bool) -> Chosen<'_> {
        let mut components: Vec<&BlueprintComponent> = self.components.iter().collect();
        for part in &self.parts {
            choose_part(part, &mut components);
        }
        let children = self
            .children
            .iter()
            .filter(|(_, child)| fixed_children || !matches!(child, Node::Fixed(_)))
            .filter_map(|(name, child)| child.choose().map(|blueprint| (name.as_str(), blueprint)))
            .collect();
        Chosen { components, children }
    }
}

fn choose_part<'a>(part: &'a Part, components: &mut Vec<&'a BlueprintComponent>) {
    let option = match part {
        Part::OneOf(options) => Some(choose_weighted(options)),
        Part::Maybe(maybe) => rand::rng().random_bool(maybe.0).then_some(&maybe.1),
    };
    let Some(option) = option else {
        return;
    };
    for component in &option.components {
        match components.iter_mut().find(|c| c.type_id == component.type_id) {
            Some(existing) => *existing = component,
            None => components.push(component),
        }
    }
    for part in &option.parts {
        choose_part(part, components);
    }
}

/// Applies a whole blueprint to `entity`, spawning all of its children.
fn apply_blueprint(entity: &mut EntityWorldMut, blueprint: &Blueprint, registry: &TypeRegistry) -> Result<()> {
    let chosen = blueprint.choose(true);
    insert_components(entity, &chosen.components, registry)?;
    spawn_children(entity, &chosen.children, registry)
}

fn spawn_children(
    entity: &mut EntityWorldMut,
    children: &[(&str, &Blueprint)],
    registry: &TypeRegistry,
) -> Result<()> {
    let parent = entity.id();
    for (_, child) in children {
        entity.world_scope(|world| apply_blueprint(&mut world.spawn(ChildOf(parent)), child, registry))?;
    }
    Ok(())
}

/// Inserts `components` into `entity`, with their random and frozen fields filled. Recipe
/// bundles go first, so explicitly listed components override them; the components themselves
/// go in one batch (a single archetype move).
///
/// Each value is moved into the entity as is. `ReflectComponent::insert` would instead rebuild
/// every value with `FromReflect`, and insert them one at a time.
fn insert_components(
    entity: &mut EntityWorldMut,
    components: &[&BlueprintComponent],
    registry: &TypeRegistry,
) -> Result<()> {
    let value_of = |component: &BlueprintComponent, entity: &mut EntityWorldMut| {
        if component.frozen.is_empty() && component.refs.is_empty() {
            component.sample(registry).map_err(BevyError::from)
        } else {
            entity.world_scope(|world| component.spawn_value(registry, world))
        }
    };

    for component in components {
        if let Some(recipe) = &component.recipe {
            let value = value_of(component, entity)?;
            recipe.insert(&*value, entity)?;
        }
    }

    let plain = || components.iter().filter(|c| c.recipe.is_none());
    if plain().next().is_none() {
        return Ok(());
    }
    let mut ids = Vec::with_capacity(components.len());
    let mut values = Vec::with_capacity(components.len());
    for component in plain() {
        let value = value_of(component, entity)?;
        // The pointer handed to `insert_by_ids` must be the component type itself.
        if value.as_any().type_id() != component.type_id {
            let message = format!("`{}` did not clone to its concrete type", value.reflect_type_path());
            return Err(BlueprintError::new(ErrorKind::Type, message).into());
        }
        let id = match entity.world().components().get_id(component.type_id) {
            Some(id) => id,
            None => {
                let reflect_component = registry
                    .get_type_data::<ReflectComponent>(component.type_id)
                    .ok_or_else(|| {
                        BlueprintError::new(ErrorKind::Type, "blueprint component lost its ReflectComponent registration")
                    })?;
                entity.world_scope(|world| reflect_component.register_component(world))
            }
        };
        ids.push(id);
        values.push(value);
    }

    let raw: Vec<(*mut u8, Layout)> = values
        .into_iter()
        .map(|value| {
            let layout = Layout::for_value(&*value);
            (Box::into_raw(value).cast::<u8>(), layout)
        })
        .collect();
    // SAFETY: each pointer owns a live value of exactly the type of the matching component id
    // (checked above), all from this entity's world. `insert_by_ids` moves the values out, so
    // afterwards only the boxes' memory is freed, without dropping the values again.
    unsafe {
        entity.insert_by_ids(
            &ids,
            raw.iter().map(|(ptr, _)| OwningPtr::new(NonNull::new_unchecked(*ptr))),
        );
        for (ptr, layout) in raw {
            if layout.size() != 0 {
                dealloc(ptr, layout);
            }
        }
    }
    Ok(())
}
