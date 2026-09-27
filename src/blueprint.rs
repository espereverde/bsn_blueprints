//! Flattened blueprints, and how inheritance patches them.

use std::any::TypeId;
use std::sync::Arc;

use bevy::asset::{Assets, Handle, UntypedHandle};
use bevy::ecs::world::World;
use bevy::prelude::ReflectDefault;
use bevy::reflect::{GetPath, ParsedPath, PartialReflect, Reflect, ReflectFromReflect, TypeRegistry};
use bevy::scene::ScenePatch;
use rand::Rng;

use crate::error::{BlueprintError, ErrorKind};
use crate::loader::BlueprintFile;
use crate::parse::RawItem;
use crate::random::{RandomField, RandomSpec, choose_weighted};
use crate::recipe::ReflectRecipe;
use crate::reflect_utils::{clone_value, leaf_paths, paths_overlap};

/// A blueprint, or a random choice of blueprints: what a labeled blueprint or a child can be.
#[derive(Clone)]
pub(crate) enum Node {
    Fixed(Arc<Blueprint>),
    /// One of these, by weight.
    OneOf(Arc<[(f64, Node)]>),
    /// This, with the given probability (children only).
    Maybe(Arc<(f64, Node)>),
}

impl Node {
    /// The blueprint for this spawn, if any.
    pub(crate) fn choose(&self) -> Option<&Blueprint> {
        match self {
            Node::Fixed(blueprint) => Some(blueprint),
            Node::OneOf(options) => choose_weighted(options).choose(),
            Node::Maybe(maybe) if rand::rng().random_bool(maybe.0) => maybe.1.choose(),
            Node::Maybe(_) => None,
        }
    }

    /// The blueprint files this node's references point to (at any depth), which must be loaded
    /// before it can spawn.
    pub(crate) fn referenced_files(&self, out: &mut Vec<UntypedHandle>) {
        match self {
            Node::Fixed(blueprint) => blueprint.referenced_files(out),
            Node::OneOf(options) => options.iter().for_each(|(_, node)| node.referenced_files(out)),
            Node::Maybe(maybe) => maybe.1.referenced_files(out),
        }
    }

    /// The same choice, with `f` applied to each blueprint in it.
    pub(crate) fn map(
        &self,
        f: &mut impl FnMut(&Blueprint) -> Result<Blueprint, BlueprintError>,
    ) -> Result<Node, BlueprintError> {
        Ok(match self {
            Node::Fixed(blueprint) => Node::Fixed(Arc::new(f(blueprint)?)),
            Node::OneOf(options) => Node::OneOf(
                options
                    .iter()
                    .map(|(weight, node)| Ok((*weight, node.map(f)?)))
                    .collect::<Result<Vec<_>, BlueprintError>>()?
                    .into(),
            ),
            Node::Maybe(maybe) => Node::Maybe(Arc::new((maybe.0, maybe.1.map(f)?))),
        })
    }
}

/// A flattened blueprint: fixed component values (with per-spawn random fields), children, and
/// random sets of components chosen at spawn time.
#[derive(Clone, Default)]
pub(crate) struct Blueprint {
    pub(crate) components: Vec<BlueprintComponent>,
    /// Named, so a derived blueprint can patch an inherited child.
    pub(crate) children: Vec<(String, Node)>,
    /// The random sets as written, so a derived blueprint can resolve them against its own
    /// components.
    pub(crate) raw_parts: Vec<Arc<RawItem>>,
    /// `raw_parts` resolved against `components`.
    pub(crate) parts: Vec<Part>,
}

/// A random set of components.
#[derive(Clone)]
pub(crate) enum Part {
    OneOf(Arc<[(f64, PartOption)]>),
    Maybe(Arc<(f64, PartOption)>),
}

/// What a chosen option adds: the components it touches, with its changes already merged into
/// the blueprint's values, and its own random sets.
pub(crate) struct PartOption {
    pub(crate) components: Vec<BlueprintComponent>,
    pub(crate) parts: Vec<Part>,
}

#[derive(Clone)]
pub(crate) struct BlueprintComponent {
    pub(crate) type_id: TypeId,
    pub(crate) value: Arc<dyn Reflect>,
    pub(crate) random: Vec<RandomField>,
    pub(crate) frozen: Vec<FrozenField>,
    pub(crate) refs: Vec<BlueprintRef>,
    /// Set for [`Recipe`](crate::Recipe) values, which expand to a bundle instead of being
    /// inserted themselves.
    pub(crate) recipe: Option<ReflectRecipe>,
}

/// A `Handle<ScenePatch>` field that gets a frozen copy of a blueprint when the component is
/// spawned: every random choice in it made once, for this owner.
#[derive(Clone)]
pub(crate) struct FrozenField {
    pub(crate) path: String,
    pub(crate) node: Node,
}

/// A `Handle<ScenePatch>` field referring to a blueprint of another file. The file is a
/// dependency of this one; the field is filled from it on spawn.
#[derive(Clone)]
pub(crate) struct BlueprintRef {
    pub(crate) path: String,
    parsed: Arc<ParsedPath>,
    file: Handle<BlueprintFile>,
    label: String,
}

impl BlueprintRef {
    pub(crate) fn new(path: String, file: Handle<BlueprintFile>, label: String) -> Result<Self, BlueprintError> {
        let parsed = ParsedPath::parse(&path)
            .map_err(|e| BlueprintError::new(ErrorKind::Reference, format!("field `{path}`: {e}")))?;
        Ok(Self {
            path,
            parsed: Arc::new(parsed),
            file,
            label,
        })
    }

    /// Sets the field in `value` to the referred blueprint.
    pub(crate) fn fill(&self, value: &mut dyn Reflect, world: &World) -> Result<(), BlueprintError> {
        let error = |message| BlueprintError::new(ErrorKind::Reference, message);
        let scene = world
            .resource::<Assets<BlueprintFile>>()
            .get(&self.file)
            .and_then(|file| file.get(&self.label))
            .ok_or_else(|| error(format!("no blueprint `{}` in {:?}", self.label, self.file.path())))?;
        *value
            .path_mut::<Handle<ScenePatch>>(&*self.parsed)
            .map_err(|e| error(format!("field `{}`: {e}", self.path)))? = scene.clone();
        Ok(())
    }
}

impl BlueprintComponent {
    /// A copy of the value with every random field sampled.
    pub(crate) fn sample(&self, registry: &TypeRegistry) -> Result<Box<dyn Reflect>, BlueprintError> {
        let mut value = clone_value(registry, &*self.value)?;
        for random in &self.random {
            random.apply(&mut *value)?;
        }
        Ok(value)
    }

    /// Forgets random and frozen values for the given fields, which now have fixed values.
    fn clear_fields(&mut self, paths: &[String]) {
        let overlaps = |field: &str| paths.iter().any(|p| paths_overlap(field, p));
        self.random.retain(|r| !overlaps(&r.path));
        self.frozen.retain(|f| !overlaps(&f.path));
        self.refs.retain(|r| !overlaps(&r.path));
    }
}

impl Blueprint {
    /// Applies a (possibly partial) component value on top of what was inherited. Fields it
    /// sets are no longer random.
    pub(crate) fn patch_component(
        &mut self,
        registry: &TypeRegistry,
        type_id: TypeId,
        patch: &dyn PartialReflect,
    ) -> Result<(), BlueprintError> {
        let error = |message: String| BlueprintError::new(ErrorKind::Type, message);
        let registration = registry
            .get(type_id)
            .ok_or_else(|| error("unregistered component type".into()))?;
        let type_path = registration.type_info().type_path();
        let patch_error = |e| error(format!("`{type_path}`: {e}"));

        if let Some(component) = self.component_mut(type_id) {
            component.clear_fields(&leaf_paths(patch));
            let mut value = clone_value(registry, &*component.value)?;
            value.try_apply(patch).map_err(patch_error)?;
            component.value = Arc::from(value);
            return Ok(());
        }

        let value: Box<dyn Reflect> = match registration.data::<ReflectDefault>() {
            Some(default) => {
                let mut value = default.default();
                value.try_apply(patch).map_err(patch_error)?;
                value
            }
            None => registration
                .data::<ReflectFromReflect>()
                .ok_or_else(|| error(format!("`{type_path}` needs #[reflect(Default)] or FromReflect")))?
                .from_reflect(patch)
                .ok_or_else(|| {
                    error(format!(
                        "`{type_path}` is incomplete; add #[reflect(Default)] to allow partial values"
                    ))
                })?,
        };
        self.components.push(BlueprintComponent {
            type_id,
            value: Arc::from(value),
            random: Vec::new(),
            frozen: Vec::new(),
            refs: Vec::new(),
            recipe: registration.data::<ReflectRecipe>().cloned(),
        });
        Ok(())
    }

    /// Makes a field of a component random, replacing what it had. `OneOf` options are merged
    /// into the field's current value here, so spawning only has to copy one in.
    pub(crate) fn set_random(
        &mut self,
        registry: &TypeRegistry,
        type_id: TypeId,
        path: &str,
        spec: &RandomSpec,
    ) -> Result<(), BlueprintError> {
        let error =
            |e: &dyn std::fmt::Display| BlueprintError::new(ErrorKind::Random, format!("random field `{path}`: {e}"));
        let component = self.field_component(registry, type_id, path)?;
        component.clear_fields(&[path.to_string()]);
        let spec = match spec {
            RandomSpec::Range(min, max) => RandomSpec::Range(*min, *max),
            RandomSpec::Pick(options) => {
                let current = if path.is_empty() {
                    (*component.value).as_partial_reflect()
                } else {
                    (*component.value).reflect_path(path).map_err(|e| error(&e))?
                };
                let mut whole = Vec::with_capacity(options.len());
                for (weight, option) in options {
                    let mut value = current.to_dynamic();
                    value.try_apply(&**option).map_err(|e| error(&e))?;
                    whole.push((*weight, value));
                }
                RandomSpec::Pick(whole)
            }
        };
        component.random.push(RandomField::new(path.to_string(), spec)?);
        Ok(())
    }

    /// Makes a field of a component a frozen blueprint, replacing what it had.
    pub(crate) fn set_frozen(
        &mut self,
        registry: &TypeRegistry,
        type_id: TypeId,
        field: FrozenField,
    ) -> Result<(), BlueprintError> {
        let component = self.field_component(registry, type_id, &field.path)?;
        component.clear_fields(std::slice::from_ref(&field.path));
        component.frozen.push(field);
        Ok(())
    }

    /// Makes a field of a component refer to another file's blueprint, replacing what it had.
    pub(crate) fn set_ref(
        &mut self,
        registry: &TypeRegistry,
        type_id: TypeId,
        field: BlueprintRef,
    ) -> Result<(), BlueprintError> {
        let component = self.field_component(registry, type_id, &field.path)?;
        component.clear_fields(std::slice::from_ref(&field.path));
        component.refs.push(field);
        Ok(())
    }

    /// Samples the random fields of a component once, so bad paths and types fail at load time,
    /// not at spawn. The sample is thrown away.
    pub(crate) fn validate_random(&self, registry: &TypeRegistry, type_id: TypeId) -> Result<(), BlueprintError> {
        match self.components.iter().find(|c| c.type_id == type_id) {
            Some(component) if !component.random.is_empty() => component.sample(registry).map(drop),
            _ => Ok(()),
        }
    }

    fn referenced_files(&self, out: &mut Vec<UntypedHandle>) {
        fn component_files(components: &[BlueprintComponent], out: &mut Vec<UntypedHandle>) {
            for component in components {
                for blueprint_ref in &component.refs {
                    let file = blueprint_ref.file.clone().untyped();
                    if !out.contains(&file) {
                        out.push(file);
                    }
                }
                for frozen in &component.frozen {
                    frozen.node.referenced_files(out);
                }
            }
        }
        fn part_files(part: &Part, out: &mut Vec<UntypedHandle>) {
            let options: Vec<&PartOption> = match part {
                Part::OneOf(options) => options.iter().map(|(_, option)| option).collect(),
                Part::Maybe(maybe) => vec![&maybe.1],
            };
            for option in options {
                component_files(&option.components, out);
                option.parts.iter().for_each(|part| part_files(part, out));
            }
        }
        component_files(&self.components, out);
        self.parts.iter().for_each(|part| part_files(part, out));
        self.children.iter().for_each(|(_, child)| child.referenced_files(out));
    }

    fn field_component(
        &mut self,
        registry: &TypeRegistry,
        type_id: TypeId,
        path: &str,
    ) -> Result<&mut BlueprintComponent, BlueprintError> {
        self.component_mut(type_id).ok_or_else(|| {
            let type_path = registry.get(type_id).map_or("?", |r| r.type_info().type_path());
            BlueprintError::new(
                ErrorKind::Type,
                format!("field `{path}` of `{type_path}`: the blueprint has no such component"),
            )
        })
    }

    fn component_mut(&mut self, type_id: TypeId) -> Option<&mut BlueprintComponent> {
        self.components.iter_mut().find(|c| c.type_id == type_id)
    }
}
