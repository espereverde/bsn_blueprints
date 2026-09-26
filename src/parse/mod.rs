//! Reading `*.bp.ron` files into raw blueprints, before inheritance is applied.
//!
//! Files are parsed with `ron2`, which keeps every name, so wherever something may vary its
//! value can be wrapped: `OneOf([..])` (items optionally `Weight(w, ..)`), `Maybe(p, ..)`, or
//! `Range(a, b)` for numbers. Anything unwrapped, or wrapped in `Fixed(..)`, is fixed.

mod expr;
mod value;

use std::any::TypeId;
use std::sync::Arc;

use bevy::asset::LoadContext;
use bevy::ecs::reflect::ReflectComponent;
use bevy::platform::collections::HashMap;
use bevy::reflect::{PartialReflect, TypeRegistration, TypeRegistry};
use ron2::ast::{Expr, MapExpr, parse_document};

use crate::BoxError;
use crate::blueprint::BlueprintRef;
use crate::random::RandomSpec;
use crate::recipe::ReflectRecipe;
use expr::{ParseError, ParseResult, call, error, number, string};
use value::{Found, ValueReader};

/// A blueprint, or a child, as written: an entry, or a random choice of them.
pub(crate) enum RawNode {
    Entry(RawEntry),
    OneOf(Vec<(f64, RawNode)>),
    Maybe(f64, Box<RawNode>),
}

/// `(extends: .., components: .., children: ..)`
#[derive(Default)]
pub(crate) struct RawEntry {
    pub(crate) extends: Option<String>,
    /// Fixed sets of components and random ones (`OneOf` / `Maybe`), in order.
    pub(crate) items: Vec<Arc<RawItem>>,
    pub(crate) children: Vec<(String, RawNode)>,
}

/// Components as written in `components`.
pub(crate) enum RawItem {
    Set(Vec<RawComponent>),
    All(Vec<RawItem>),
    OneOf(Vec<(f64, RawItem)>),
    Maybe(f64, Box<RawItem>),
}

/// A (possibly partial) component value, with its random and frozen fields.
pub(crate) struct RawComponent {
    pub(crate) type_id: TypeId,
    pub(crate) value: Box<dyn PartialReflect>,
    pub(crate) random: Vec<(String, RandomSpec)>,
    pub(crate) frozen: Vec<FrozenRef>,
    pub(crate) refs: Vec<BlueprintRef>,
}

/// `Frozen(Path(".."))` in a `Handle<ScenePatch>` field: the field at `path` gets a frozen copy
/// of the blueprint `target` (written like an `extends`) when the component is spawned.
pub(crate) struct FrozenRef {
    pub(crate) path: String,
    pub(crate) target: String,
}

impl RawNode {
    /// Every blueprint this node refers to (`extends` and frozen fields), at any depth.
    pub(crate) fn all_extends<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            RawNode::Entry(entry) => {
                out.extend(entry.extends.as_deref());
                for item in &entry.items {
                    item.all_extends(out);
                }
                for (_, child) in &entry.children {
                    child.all_extends(out);
                }
            }
            RawNode::OneOf(options) => options.iter().for_each(|(_, node)| node.all_extends(out)),
            RawNode::Maybe(_, node) => node.all_extends(out),
        }
    }
}

impl RawItem {
    fn all_extends<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            RawItem::Set(components) => {
                for component in components {
                    out.extend(component.frozen.iter().map(|f| f.target.as_str()));
                }
            }
            RawItem::All(items) => items.iter().for_each(|item| item.all_extends(out)),
            RawItem::OneOf(options) => options.iter().for_each(|(_, item)| item.all_extends(out)),
            RawItem::Maybe(_, item) => item.all_extends(out),
        }
    }
}

/// Parses a whole file: a map of label to blueprint.
pub(crate) fn parse_file(
    source: &str,
    registry: &TypeRegistry,
    load_context: &mut LoadContext,
) -> Result<HashMap<String, RawNode>, BoxError> {
    let document = parse_document(source).map_err(|e| ParseError {
        span: *e.span(),
        message: e.kind().to_string(),
    })?;
    let Some(root) = &document.value else {
        return Ok(HashMap::default());
    };
    let Expr::Map(map) = root else {
        return Err(error(root, "expected a map of blueprint name to blueprint").into());
    };
    let mut reader = ValueReader {
        registry,
        here: load_context.path().clone_owned(),
        load_context,
    };
    let mut nodes = HashMap::default();
    for entry in &map.entries {
        let label = string(&entry.key)?;
        let node = read_node(&entry.value, &mut reader)?;
        if nodes.insert(label.to_string(), node).is_some() {
            return Err(error(&entry.key, format!("duplicate blueprint `{label}`")).into());
        }
    }
    Ok(nodes)
}

// ============================================================================
// Wrappers
// ============================================================================

pub(crate) enum Wrapper<'e, 'a> {
    OneOf(Vec<(f64, &'e Expr<'a>)>),
    Maybe(f64, &'e Expr<'a>),
    Plain(&'e Expr<'a>),
}

/// Recognizes `OneOf([..])`, `Maybe(p, ..)` and `Fixed(..)` by name and shape. Anything else,
/// including enum variants that happen to share a name but not the shape, is a plain value.
pub(crate) fn wrapper<'e, 'a>(expr: &'e Expr<'a>) -> ParseResult<Wrapper<'e, 'a>> {
    match call(expr) {
        Some(("Fixed", args)) if args.len() == 1 => Ok(Wrapper::Plain(args[0])),
        Some(("OneOf", args)) if args.len() == 1 && matches!(args[0], Expr::Seq(_)) => {
            let Expr::Seq(seq) = args[0] else { unreachable!() };
            if seq.items.is_empty() {
                return Err(error(expr, "OneOf(..) needs at least one option"));
            }
            let mut options = Vec::with_capacity(seq.items.len());
            for item in &seq.items {
                options.push(match call(&item.expr) {
                    Some(("Weight", weighted)) if weighted.len() == 2 => {
                        let weight = number(weighted[0])?;
                        if !(weight.is_finite() && weight > 0.0) {
                            return Err(error(weighted[0], format!("weight must be positive, found {weight}")));
                        }
                        (weight, weighted[1])
                    }
                    _ => (1.0, &item.expr),
                });
            }
            Ok(Wrapper::OneOf(options))
        }
        Some(("Maybe", args)) if args.len() == 2 => {
            let chance = number(args[0])?;
            if !(0.0..=1.0).contains(&chance) {
                return Err(error(args[0], format!("Maybe chance must be between 0 and 1, found {chance}")));
            }
            Ok(Wrapper::Maybe(chance, args[1]))
        }
        _ => Ok(Wrapper::Plain(expr)),
    }
}

// ============================================================================
// Blueprints
// ============================================================================

fn read_node(expr: &Expr, reader: &mut ValueReader) -> ParseResult<RawNode> {
    Ok(match wrapper(expr)? {
        Wrapper::Plain(expr) => RawNode::Entry(read_entry(expr, reader)?),
        Wrapper::OneOf(options) => RawNode::OneOf(
            options
                .into_iter()
                .map(|(weight, option)| Ok((weight, read_node(option, reader)?)))
                .collect::<ParseResult<_>>()?,
        ),
        Wrapper::Maybe(chance, inner) => RawNode::Maybe(chance, Box::new(read_node(inner, reader)?)),
    })
}

const ENTRY_FIELDS: &str = "extends, components or children";

fn read_entry(expr: &Expr, reader: &mut ValueReader) -> ParseResult<RawEntry> {
    let fields = match expr {
        Expr::Unit(_) => &[][..],
        Expr::AnonStruct(s) => &s.fields[..],
        _ => return Err(error(expr, "expected a blueprint: (extends: .., components: .., children: ..)")),
    };
    let mut entry = RawEntry::default();
    for field in fields {
        match &*field.name.name {
            "extends" => entry.extends = Some(string(&field.value)?.to_string()),
            "components" => entry.items.extend(read_items(&field.value, reader)?.into_iter().map(Arc::new)),
            "children" => {
                let Expr::Map(map) = &field.value else {
                    return Err(error(&field.value, "expected a map of child name to blueprint"));
                };
                for child in &map.entries {
                    let name = string(&child.key)?;
                    if entry.children.iter().any(|(n, _)| n == name) {
                        return Err(error(&child.key, format!("duplicate child `{name}`")));
                    }
                    entry.children.push((name.to_string(), read_node(&child.value, reader)?));
                }
            }
            other => {
                return Err(ParseError {
                    span: field.name.span,
                    message: format!("unknown field `{other}`; expected {ENTRY_FIELDS}"),
                });
            }
        }
    }
    Ok(entry)
}

// ============================================================================
// Components
// ============================================================================

/// `{ .. }`, `[ .. ]`, or a `OneOf` / `Maybe` of them. Top-level lists are flattened.
fn read_items(expr: &Expr, reader: &mut ValueReader) -> ParseResult<Vec<RawItem>> {
    match wrapper(expr)? {
        Wrapper::Plain(Expr::Map(map)) => read_map(map, reader),
        Wrapper::Plain(Expr::Seq(seq)) => {
            let mut items = Vec::new();
            for item in &seq.items {
                items.extend(read_items(&item.expr, reader)?);
            }
            Ok(items)
        }
        Wrapper::Plain(other) => Err(error(other, "expected a map of components, or a list of them")),
        Wrapper::OneOf(options) => Ok(vec![RawItem::OneOf(
            options
                .into_iter()
                .map(|(weight, option)| Ok((weight, group(read_items(option, reader)?))))
                .collect::<ParseResult<_>>()?,
        )]),
        Wrapper::Maybe(chance, inner) => Ok(vec![RawItem::Maybe(chance, Box::new(group(read_items(inner, reader)?)))]),
    }
}

fn group(mut items: Vec<RawItem>) -> RawItem {
    if items.len() == 1 {
        items.pop().expect("one item")
    } else {
        RawItem::All(items)
    }
}

/// `{ "Type": value, .. }`. A `Maybe(p, ..)` value makes that component optional.
fn read_map(map: &MapExpr, reader: &mut ValueReader) -> ParseResult<Vec<RawItem>> {
    let registry = reader.registry;
    let mut set = Vec::new();
    let mut optional = Vec::new();
    for entry in &map.entries {
        let registration = component_registration(registry, &entry.key)?;
        match wrapper(&entry.value)? {
            Wrapper::Maybe(chance, value) => {
                let component = read_component(reader, registration, value)?;
                optional.push(RawItem::Maybe(chance, Box::new(RawItem::Set(vec![component]))));
            }
            _ => set.push(read_component(reader, registration, &entry.value)?),
        }
    }
    let mut items = Vec::with_capacity(1 + optional.len());
    if !set.is_empty() {
        items.push(RawItem::Set(set));
    }
    items.extend(optional);
    Ok(items)
}

fn read_component(
    reader: &mut ValueReader,
    registration: &TypeRegistration,
    expr: &Expr,
) -> ParseResult<RawComponent> {
    let mut found = Found::default();
    let value = reader.component(registration, expr, &mut found)?;
    Ok(RawComponent {
        type_id: registration.type_id(),
        value,
        random: found.random,
        frozen: found.frozen,
        refs: found.refs,
    })
}

/// A component or recipe type, by full or (unambiguous) short type path.
fn component_registration<'r>(registry: &'r TypeRegistry, key: &Expr) -> ParseResult<&'r TypeRegistration> {
    let name = string(key)?;
    let registration = registry
        .get_with_type_path(name)
        .or_else(|| registry.get_with_short_type_path(name))
        .ok_or_else(|| error(key, format!("unknown or ambiguous type `{name}`; is it registered?")))?;
    if registration.data::<ReflectComponent>().is_none() && registration.data::<ReflectRecipe>().is_none() {
        return Err(error(key, format!(
            "`{name}` is neither a component nor a recipe; add #[reflect(Component)] or #[reflect(Recipe)]"
        )));
    }
    Ok(registration)
}
