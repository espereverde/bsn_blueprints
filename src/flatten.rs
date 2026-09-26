//! Applying inheritance: turning the raw blueprints of a file into [`Node`]s.

use std::sync::Arc;

use bevy::asset::AssetPath;
use bevy::platform::collections::HashMap;
use bevy::reflect::TypeRegistry;

use crate::BoxError;
use crate::blueprint::{Blueprint, FrozenField, Node, Part, PartOption};
use crate::loader::BlueprintFile;
use crate::parse::{RawComponent, RawEntry, RawItem, RawNode};

pub(crate) struct Flattener<'a> {
    here: &'a AssetPath<'static>,
    raw: &'a HashMap<String, RawNode>,
    parent_files: &'a HashMap<AssetPath<'static>, BlueprintFile>,
    registry: &'a TypeRegistry,
    done: HashMap<String, Node>,
    visiting: Vec<String>,
}

impl<'a> Flattener<'a> {
    pub(crate) fn new(
        here: &'a AssetPath<'static>,
        raw: &'a HashMap<String, RawNode>,
        parent_files: &'a HashMap<AssetPath<'static>, BlueprintFile>,
        registry: &'a TypeRegistry,
    ) -> Self {
        Self {
            here,
            raw,
            parent_files,
            registry,
            done: HashMap::default(),
            visiting: Vec::new(),
        }
    }

    /// Flattens every blueprint of the file.
    pub(crate) fn flatten_all(mut self) -> Result<HashMap<String, Node>, BoxError> {
        let raw = self.raw;
        for label in raw.keys() {
            self.flatten(label)?;
        }
        Ok(self.done)
    }

    /// Flattens a blueprint of this file (memoized, with cycle detection).
    fn flatten(&mut self, label: &str) -> Result<Node, BoxError> {
        if let Some(done) = self.done.get(label) {
            return Ok(done.clone());
        }
        if self.visiting.iter().any(|l| l == label) {
            return Err(format!(
                "inheritance cycle in {}: {} -> {label}",
                self.here,
                self.visiting.join(" -> ")
            )
            .into());
        }
        let raw = self
            .raw
            .get(label)
            .ok_or_else(|| format!("no blueprint `{label}` in {}", self.here))?;

        // Still "visiting" while children are built, so a child extending one of its own
        // ancestors is reported as a cycle.
        self.visiting.push(label.to_string());
        let node = self.build_node(raw, None)?;
        self.visiting.pop();

        ensure_no_maybe(&node).map_err(|e| format!("blueprint `{label}`: {e}"))?;
        self.done.insert(label.to_string(), node.clone());
        Ok(node)
    }

    /// Builds a node on top of what it inherits: its `extends` target, or else `inherited`
    /// (an inherited child of the same name).
    ///
    /// A plain entry applies its changes to every option of an inherited `OneOf` / `Maybe`. A
    /// `OneOf` / `Maybe` replaces the inherited randomness instead: its options build on the
    /// inherited child without its `Maybe`s, or on nothing if that child is a `OneOf`.
    fn build_node(&mut self, raw: &RawNode, inherited: Option<&Node>) -> Result<Node, BoxError> {
        Ok(match raw {
            RawNode::Entry(entry) => {
                let base = match &entry.extends {
                    Some(extends) => self.resolve_extends(extends)?,
                    None => inherited.cloned().unwrap_or_else(|| Node::Fixed(Arc::default())),
                };
                base.map(&mut |blueprint| self.build_entry(entry, blueprint.clone()))?
            }
            RawNode::OneOf(options) => {
                let inherited = inherited.and_then(without_maybe);
                Node::OneOf(
                    options
                        .iter()
                        .map(|(weight, option)| Ok((*weight, self.build_node(option, inherited)?)))
                        .collect::<Result<Vec<_>, BoxError>>()?
                        .into(),
                )
            }
            RawNode::Maybe(chance, inner) => {
                let inherited = inherited.and_then(without_maybe);
                Node::Maybe(Arc::new((*chance, self.build_node(inner, inherited)?)))
            }
        })
    }

    fn build_entry(&mut self, entry: &RawEntry, mut blueprint: Blueprint) -> Result<Blueprint, BoxError> {
        for item in &entry.items {
            match &**item {
                RawItem::Set(components) => {
                    for component in components {
                        self.apply_component(&mut blueprint, component)?;
                    }
                }
                _ => blueprint.raw_parts.push(item.clone()),
            }
        }

        for (name, raw_child) in &entry.children {
            let index = blueprint.children.iter().position(|(n, _)| n == name);
            let inherited = index.map(|i| blueprint.children[i].1.clone());
            let child = self.build_node(raw_child, inherited.as_ref())?;
            match index {
                Some(i) => blueprint.children[i].1 = child,
                None => blueprint.children.push((name.clone(), child)),
            }
        }

        let parts = blueprint
            .raw_parts
            .iter()
            .map(|raw| self.resolve_part(raw, &blueprint))
            .collect::<Result<_, _>>()?;
        blueprint.parts = parts;
        blueprint.validate_random(self.registry)?;
        Ok(blueprint)
    }

    fn apply_component(&mut self, blueprint: &mut Blueprint, raw: &RawComponent) -> Result<(), BoxError> {
        blueprint.patch_component(self.registry, raw.type_id, &*raw.value)?;
        for (path, spec) in &raw.random {
            blueprint.set_random(self.registry, raw.type_id, path, spec)?;
        }
        for blueprint_ref in &raw.refs {
            blueprint.set_ref(self.registry, raw.type_id, blueprint_ref.clone())?;
        }
        for frozen in &raw.frozen {
            let node = self.resolve_extends(&frozen.target)?;
            ensure_no_maybe(&node).map_err(|e| format!("Frozen(\"{}\"): {e}", frozen.target))?;
            let field = FrozenField {
                path: frozen.path.clone(),
                node,
            };
            blueprint.set_frozen(self.registry, raw.type_id, field)?;
        }
        Ok(())
    }

    /// Resolves a random set against `base`: each option's values are merged into the base's.
    fn resolve_part(&mut self, raw: &RawItem, base: &Blueprint) -> Result<Part, BoxError> {
        Ok(match raw {
            RawItem::OneOf(options) => Part::OneOf(
                options
                    .iter()
                    .map(|(weight, option)| Ok((*weight, self.resolve_option(option, base)?)))
                    .collect::<Result<Vec<_>, BoxError>>()?
                    .into(),
            ),
            RawItem::Maybe(chance, option) => Part::Maybe(Arc::new((*chance, self.resolve_option(option, base)?))),
            RawItem::Set(_) | RawItem::All(_) => unreachable!("fixed sets are applied, not resolved"),
        })
    }

    fn resolve_option(&mut self, raw: &RawItem, base: &Blueprint) -> Result<PartOption, BoxError> {
        let mut scratch = Blueprint {
            components: base.components.clone(),
            ..Blueprint::default()
        };
        let mut touched = Vec::new();
        let mut parts = Vec::new();
        self.add_to_option(raw, &mut scratch, &mut touched, &mut parts)?;
        scratch.validate_random(self.registry)?;
        let components = scratch
            .components
            .into_iter()
            .filter(|c| touched.contains(&c.type_id))
            .collect();
        Ok(PartOption { components, parts })
    }

    fn add_to_option(
        &mut self,
        raw: &RawItem,
        scratch: &mut Blueprint,
        touched: &mut Vec<std::any::TypeId>,
        parts: &mut Vec<Part>,
    ) -> Result<(), BoxError> {
        match raw {
            RawItem::Set(components) => {
                for component in components {
                    self.apply_component(scratch, component)?;
                    if !touched.contains(&component.type_id) {
                        touched.push(component.type_id);
                    }
                }
            }
            RawItem::All(items) => {
                for item in items {
                    self.add_to_option(item, scratch, touched, parts)?;
                }
            }
            RawItem::OneOf(_) | RawItem::Maybe(..) => parts.push(self.resolve_part(raw, scratch)?),
        }
        Ok(())
    }

    fn resolve_extends(&mut self, extends: &str) -> Result<Node, BoxError> {
        let target = self.here.resolve_embed_str(extends)?;
        let label = target
            .label()
            .ok_or_else(|| format!("`extends: \"{extends}\"` needs a #label"))?;
        if target.path() == self.here.path() {
            return self.flatten(label);
        }
        let file = target.without_label().into_owned();
        self.parent_files[&file]
            .entries
            .get(label)
            .cloned()
            .ok_or_else(|| format!("no blueprint `{label}` in {file}").into())
    }
}

/// The single blueprint under any `Maybe`s; `None` for a `OneOf`, which has no single one.
fn without_maybe(node: &Node) -> Option<&Node> {
    match node {
        Node::Fixed(_) => Some(node),
        Node::Maybe(maybe) => without_maybe(&maybe.1),
        Node::OneOf(_) => None,
    }
}

/// Only children can be `Maybe(..)`: a blueprint always spawns something.
fn ensure_no_maybe(node: &Node) -> Result<(), BoxError> {
    match node {
        Node::Fixed(_) => Ok(()),
        Node::OneOf(options) => options.iter().try_for_each(|(_, option)| ensure_no_maybe(option)),
        Node::Maybe(_) => Err("only children can be Maybe(..)".into()),
    }
}
