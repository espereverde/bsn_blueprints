//! Applying inheritance: turning the raw blueprints of a file into [`Node`]s.

use std::sync::Arc;

use bevy::asset::AssetPath;
use bevy::platform::collections::HashMap;
use bevy::reflect::TypeRegistry;
use ron2::error::Span;

use crate::blueprint::{Blueprint, FrozenField, Node, Part, PartOption};
use crate::error::{BlueprintError, ErrorKind};
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
    pub(crate) fn flatten_all(mut self) -> Result<HashMap<String, Node>, BlueprintError> {
        let raw = self.raw;
        for label in raw.keys() {
            self.flatten(label)?;
        }
        Ok(self.done)
    }

    /// Flattens a blueprint of this file (memoized, with cycle detection). `label` must exist.
    fn flatten(&mut self, label: &str) -> Result<Node, BlueprintError> {
        if let Some(done) = self.done.get(label) {
            return Ok(done.clone());
        }
        if self.visiting.iter().any(|l| l == label) {
            return Err(BlueprintError::new(
                ErrorKind::Inheritance,
                format!(
                    "inheritance cycle in {}: {} -> {label}",
                    self.here,
                    self.visiting.join(" -> ")
                ),
            ));
        }
        let raw = &self.raw[label];

        // Still "visiting" while children are built, so a child extending one of its own
        // ancestors is reported as a cycle.
        self.visiting.push(label.to_string());
        let node = self.build_node(raw, None)?;
        self.visiting.pop();

        self.done.insert(label.to_string(), node.clone());
        Ok(node)
    }

    /// Builds a node on top of what it inherits: its `extends` target, or else `inherited`
    /// (an inherited child of the same name).
    ///
    /// A plain entry applies its changes to every option of an inherited `OneOf` / `Maybe`. A
    /// `OneOf` / `Maybe` replaces the inherited randomness instead: its options build on the
    /// inherited child without its `Maybe`s, or on nothing if that child is a `OneOf`.
    fn build_node(&mut self, raw: &RawNode, inherited: Option<&Node>) -> Result<Node, BlueprintError> {
        Ok(match raw {
            RawNode::Entry(entry) => {
                let base = match &entry.extends {
                    Some((extends, span)) => self.resolve_blueprint(extends, span, ErrorKind::Inheritance)?,
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
                        .collect::<Result<Vec<_>, BlueprintError>>()?
                        .into(),
                )
            }
            RawNode::Maybe(chance, inner) => {
                let inherited = inherited.and_then(without_maybe);
                Node::Maybe(Arc::new((*chance, self.build_node(inner, inherited)?)))
            }
        })
    }

    fn build_entry(&mut self, entry: &RawEntry, mut blueprint: Blueprint) -> Result<Blueprint, BlueprintError> {
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
        Ok(blueprint)
    }

    fn apply_component(&mut self, blueprint: &mut Blueprint, raw: &RawComponent) -> Result<(), BlueprintError> {
        let at = |e: BlueprintError| e.at(&raw.span);
        blueprint
            .patch_component(self.registry, raw.type_id, &*raw.value)
            .map_err(at)?;
        for (path, spec) in &raw.random {
            blueprint
                .set_random(self.registry, raw.type_id, path, spec)
                .map_err(at)?;
        }
        blueprint.validate_random(self.registry, raw.type_id).map_err(at)?;
        for blueprint_ref in &raw.refs {
            blueprint
                .set_ref(self.registry, raw.type_id, blueprint_ref.clone())
                .map_err(at)?;
        }
        for frozen in &raw.frozen {
            let field = FrozenField {
                path: frozen.path.clone(),
                node: self.resolve_blueprint(&frozen.target, &frozen.span, ErrorKind::Reference)?,
            };
            blueprint.set_frozen(self.registry, raw.type_id, field).map_err(at)?;
        }
        Ok(())
    }

    /// Resolves a random set against `base`: each option's values are merged into the base's.
    fn resolve_part(&mut self, raw: &RawItem, base: &Blueprint) -> Result<Part, BlueprintError> {
        Ok(match raw {
            RawItem::OneOf(options) => Part::OneOf(
                options
                    .iter()
                    .map(|(weight, option)| Ok((*weight, self.resolve_option(option, base)?)))
                    .collect::<Result<Vec<_>, BlueprintError>>()?
                    .into(),
            ),
            RawItem::Maybe(chance, option) => Part::Maybe(Arc::new((*chance, self.resolve_option(option, base)?))),
            RawItem::Set(_) | RawItem::All(_) => unreachable!("fixed sets are applied, not resolved"),
        })
    }

    fn resolve_option(&mut self, raw: &RawItem, base: &Blueprint) -> Result<PartOption, BlueprintError> {
        let mut scratch = Blueprint {
            components: base.components.clone(),
            ..Blueprint::default()
        };
        let mut touched = Vec::new();
        let mut parts = Vec::new();
        self.add_to_option(raw, &mut scratch, &mut touched, &mut parts)?;
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
    ) -> Result<(), BlueprintError> {
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

    /// The blueprint `target` refers to (`"#label"` or `"file.bp.ron#label"`), as written at
    /// `span` in an `extends` or a `Frozen(..)`; `kind` says which, for errors.
    fn resolve_blueprint(&mut self, target: &str, span: &Span, kind: ErrorKind) -> Result<Node, BlueprintError> {
        let error = |message: String| BlueprintError::new(kind, message);
        let path = self
            .here
            .resolve_embed_str(target)
            .map_err(|e| error(e.to_string()).at(span))?;
        let label = path.label().ok_or_else(|| {
            error(format!(
                "`{target}` needs a #label (\"#label\" or \"file.bp.ron#label\")"
            ))
            .at(span)
        })?;
        let file = path.without_label().into_owned();
        if path.path() == self.here.path() {
            if !self.raw.contains_key(label) {
                return Err(error(format!("no blueprint `{label}` in {file}")).at(span));
            }
            // A cycle is reported at the reference that closes it.
            return self.flatten(label).map_err(|e| e.at(span));
        }
        self.parent_files[&file]
            .entries
            .get(label)
            .cloned()
            .ok_or_else(|| error(format!("no blueprint `{label}` in {file}")).at(span))
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
