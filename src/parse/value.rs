//! Reading component values with the type registry, recording random and frozen fields.

use std::any::TypeId;

use bevy::asset::{AssetPath, Handle, LoadContext, ReflectHandle};
use bevy::reflect::array::DynamicArray;
use bevy::reflect::enums::{DynamicEnum, DynamicVariant, EnumInfo, VariantInfo};
use bevy::reflect::list::DynamicList;
use bevy::reflect::map::{DynamicMap, Map};
use bevy::reflect::set::{DynamicSet, Set};
use bevy::reflect::structs::DynamicStruct;
use bevy::reflect::tuple::DynamicTuple;
use bevy::reflect::tuple_struct::DynamicTupleStruct;
use bevy::reflect::{
    PartialReflect, ReflectDeserialize, ReflectRef, Type, TypeInfo, TypeRegistration, TypeRegistry,
};
use bevy::scene::ScenePatch;
use ron2::ast::{Expr, StructBody, StructField};

use super::expr::{ExprDeserializer, ParseResult, call, error, number, string};
use crate::error::ErrorKind;
use super::{FrozenRef, Wrapper, wrapper};
use crate::blueprint::BlueprintRef;
use crate::loader::BlueprintFile;
use crate::random::RandomSpec;
use crate::reflect_utils::join_path;

/// Reads component values. Handle fields (`Path("..")`) are loaded as dependencies of the file,
/// relative to it.
pub(crate) struct ValueReader<'a, 'c> {
    pub(crate) registry: &'a TypeRegistry,
    pub(crate) here: AssetPath<'static>,
    pub(crate) load_context: &'a mut LoadContext<'c>,
}

/// The random, frozen and blueprint-reference fields found in a component value, by path.
#[derive(Default)]
pub(crate) struct Found {
    pub(crate) random: Vec<(String, RandomSpec)>,
    pub(crate) frozen: Vec<FrozenRef>,
    pub(crate) refs: Vec<BlueprintRef>,
}

/// Where a value sits in its component.
#[derive(Clone)]
struct At {
    /// Its path; `None` inside maps.
    path: Option<String>,
    /// Whether random values may go here (not inside lists, maps or `OneOf` options).
    random_ok: bool,
    /// Whether it may be left out of the patch (named struct fields, and the component itself).
    omittable: bool,
}

impl At {
    fn field(&self, segment: &str, omittable: bool) -> At {
        At {
            path: self.path.as_deref().map(|path| join_path(path, segment)),
            random_ok: self.random_ok,
            omittable,
        }
    }

    fn item(&self, index: usize) -> At {
        At {
            path: self.path.as_deref().map(|path| format!("{path}[{index}]")),
            random_ok: false,
            omittable: false,
        }
    }

    fn unreachable() -> At {
        At {
            path: None,
            random_ok: false,
            omittable: false,
        }
    }
}

impl ValueReader<'_, '_> {
    /// Reads a (possibly partial) component value, recording its random and frozen fields.
    pub(crate) fn component(
        &mut self,
        registration: &TypeRegistration,
        expr: &Expr,
        found: &mut Found,
    ) -> ParseResult<Box<dyn PartialReflect>> {
        let omittable = matches!(registration.type_info(), TypeInfo::Struct(_));
        let at = At {
            path: Some(String::new()),
            random_ok: true,
            omittable,
        };
        Ok(match self.read(expr, registration, at, found)? {
            Some(value) => value,
            None => {
                let mut empty = DynamicStruct::default();
                empty.set_represented_type(Some(registration.type_info()));
                Box::new(empty)
            }
        })
    }

    /// Reads a value that may be random. `None` when it is left out of the patch: a `OneOf` of
    /// partial values, each applied on spawn on top of what the field otherwise holds.
    fn read(
        &mut self,
        expr: &Expr,
        registration: &TypeRegistration,
        at: At,
        found: &mut Found,
    ) -> ParseResult<Option<Box<dyn PartialReflect>>> {
        if is_number(registration.type_id())
            && let Some(("Range", args)) = call(expr)
            && args.len() == 2
        {
            let path = random_path(expr, &at)?;
            let spec = RandomSpec::range(number(args[0])?, number(args[1])?).map_err(|e| e.at(expr.span()))?;
            let placeholder = self.serde(args[0], registration)?;
            found.random.push((path, spec));
            return Ok(Some(placeholder));
        }
        match wrapper(expr)? {
            Wrapper::Plain(expr) => self.plain(expr, registration, at, found).map(Some),
            Wrapper::Maybe(..) => Err(error(
                ErrorKind::Random,
                expr,
                "Maybe(..) only works on components, sets of components and children; use OneOf(..) for values",
            )),
            Wrapper::OneOf(options) => {
                let path = random_path(expr, &at)?;
                let mut values = Vec::with_capacity(options.len());
                for (weight, option) in options {
                    let mut nested = Found::default();
                    let value = self.plain(option, registration, At::unreachable(), &mut nested)?;
                    if !nested.random.is_empty() || !nested.frozen.is_empty() {
                        return Err(error(ErrorKind::Random, option, "OneOf(..) options can't contain random values"));
                    }
                    values.push((weight, value));
                }
                // Partial struct options are applied on top of the field's value at spawn, so
                // they are left out of the patch; whole values stand in with the first option.
                let partial = matches!(
                    values[0].1.reflect_ref(),
                    ReflectRef::Struct(_) | ReflectRef::TupleStruct(_) | ReflectRef::Tuple(_)
                );
                let placeholder = (!(partial && at.omittable)).then(|| values[0].1.to_dynamic());
                found.random.push((path, RandomSpec::pick(values).map_err(|e| e.at(expr.span()))?));
                Ok(placeholder)
            }
        }
    }

    /// Reads a fixed value (its fields may still be random).
    fn plain(
        &mut self,
        expr: &Expr,
        registration: &TypeRegistration,
        at: At,
        found: &mut Found,
    ) -> ParseResult<Box<dyn PartialReflect>> {
        if let Some(handle) = registration.data::<ReflectHandle>() {
            return self.handle(expr, handle, &at, found);
        }
        let info = registration.type_info();
        let structured = match info {
            TypeInfo::Struct(info) => match struct_fields(expr) {
                Some(fields) => {
                    let mut value = DynamicStruct::default();
                    for field in fields {
                        let name = &*field.name.name;
                        let Some(field_info) = info.field(name) else {
                            let expected: Vec<_> = info.field_names().to_vec();
                            return Err(error(ErrorKind::Type, &field.value, format!(
                                "no field `{name}` in `{}`; expected one of {expected:?}",
                                info.type_path()
                            )));
                        };
                        let field_registration = lookup(self.registry, &field.value, field_info.ty())?;
                        if let Some(field_value) = self.read(&field.value, field_registration, at.field(name, true), found)? {
                            value.insert_boxed(name.to_string(), field_value);
                        }
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                None => None,
            },
            TypeInfo::TupleStruct(info) => match tuple_items(expr, true) {
                Some(items) => {
                    let mut value = DynamicTupleStruct::default();
                    for (i, item) in items.into_iter().enumerate() {
                        let field_info = info.field_at(i).ok_or_else(|| error(ErrorKind::Type, item, "too many values"))?;
                        let field_registration = lookup(self.registry, item, field_info.ty())?;
                        value.insert_boxed(self.required(item, field_registration, at.field(&i.to_string(), false), found)?);
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                None => None,
            },
            TypeInfo::Tuple(info) => match tuple_items(expr, false) {
                Some(items) => {
                    let mut value = DynamicTuple::default();
                    for (i, item) in items.into_iter().enumerate() {
                        let field_info = info.field_at(i).ok_or_else(|| error(ErrorKind::Type, item, "too many values"))?;
                        let field_registration = lookup(self.registry, item, field_info.ty())?;
                        value.insert_boxed(self.required(item, field_registration, at.field(&i.to_string(), false), found)?);
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                None => None,
            },
            TypeInfo::Enum(info) => self.enum_value(expr, registration, info, &at, found)?,
            TypeInfo::List(info) => match expr {
                Expr::Seq(seq) => {
                    let item_registration = lookup(self.registry, expr, &info.item_ty())?;
                    let mut value = DynamicList::default();
                    for (i, item) in seq.items.iter().enumerate() {
                        value.push_box(self.required(&item.expr, item_registration, at.item(i), found)?);
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                _ => None,
            },
            TypeInfo::Array(info) => match expr {
                Expr::Seq(seq) => {
                    let item_registration = lookup(self.registry, expr, &info.item_ty())?;
                    let mut items = Vec::with_capacity(seq.items.len());
                    for (i, item) in seq.items.iter().enumerate() {
                        items.push(self.required(&item.expr, item_registration, at.item(i), found)?);
                    }
                    let mut value = DynamicArray::new(items.into_boxed_slice());
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                _ => None,
            },
            TypeInfo::Map(info) => match expr {
                Expr::Map(map) => {
                    let key_registration = lookup(self.registry, expr, &info.key_ty())?;
                    let value_registration = lookup(self.registry, expr, &info.value_ty())?;
                    let mut value = DynamicMap::default();
                    for entry in &map.entries {
                        let k = self.required(&entry.key, key_registration, At::unreachable(), found)?;
                        let v = self.required(&entry.value, value_registration, At::unreachable(), found)?;
                        value.insert_boxed(k, v);
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                _ => None,
            },
            TypeInfo::Set(info) => match expr {
                Expr::Seq(seq) => {
                    let item_registration = lookup(self.registry, expr, &info.value_ty())?;
                    let mut value = DynamicSet::default();
                    for item in &seq.items {
                        value.insert_boxed(self.required(&item.expr, item_registration, At::unreachable(), found)?);
                    }
                    value.set_represented_type(Some(registration.type_info()));
                    Some(Box::new(value) as Box<dyn PartialReflect>)
                }
                _ => None,
            },
            TypeInfo::Opaque(_) => None,
        };
        match structured {
            Some(value) => Ok(value),
            None => self.serde(expr, registration),
        }
    }

    /// Reads a value that can't be left out.
    fn required(
        &mut self,
        expr: &Expr,
        registration: &TypeRegistration,
        at: At,
        found: &mut Found,
    ) -> ParseResult<Box<dyn PartialReflect>> {
        match self.read(expr, registration, at, found)? {
            Some(value) => Ok(value),
            None => Err(error(ErrorKind::Random, expr, "this value can't be left out")),
        }
    }

    /// `Variant`, `Variant(..)`, `Variant(field: ..)`; `Some(..)` / `None` for `Option`.
    fn enum_value(
        &mut self,
        expr: &Expr,
        registration: &TypeRegistration,
        info: &EnumInfo,
        at: &At,
        found: &mut Found,
    ) -> ParseResult<Option<Box<dyn PartialReflect>>> {
        let (name, variant) = if info.type_path().starts_with("core::option::Option<") {
            let Expr::Option(option) = expr else {
                return Err(error(ErrorKind::Type, expr, "expected Some(..) or None"));
            };
            match &option.value {
                None => ("None", DynamicVariant::Unit),
                Some(inner) => {
                    let Some(VariantInfo::Tuple(some)) = info.variant("Some") else {
                        return Err(error(ErrorKind::Type, expr, "unexpected Option type"));
                    };
                    let inner_registration = lookup(self.registry, expr, some.field_at(0).expect("Some has a field").ty())?;
                    let value = self.required(&inner.expr, inner_registration, at.field("0", false), found)?;
                    let mut tuple = DynamicTuple::default();
                    tuple.insert_boxed(value);
                    ("Some", DynamicVariant::Tuple(tuple))
                }
            }
        } else {
            let Expr::Struct(s) = expr else {
                return Ok(None);
            };
            let name = &*s.name.name;
            let Some(variant_info) = info.variant(name) else {
                return Err(error(ErrorKind::Type, expr, format!(
                    "unknown variant `{name}` of `{}`; expected one of {:?}",
                    info.type_path(),
                    info.variant_names()
                )));
            };
            let variant = match (variant_info, &s.body) {
                (VariantInfo::Unit(_), None) => DynamicVariant::Unit,
                (VariantInfo::Tuple(fields), Some(StructBody::Tuple(body))) => {
                    let mut tuple = DynamicTuple::default();
                    for (i, element) in body.elements.iter().enumerate() {
                        let field = fields.field_at(i).ok_or_else(|| error(ErrorKind::Type, &element.expr, "too many values"))?;
                        let field_registration = lookup(self.registry, &element.expr, field.ty())?;
                        tuple.insert_boxed(self.required(&element.expr, field_registration, at.field(&i.to_string(), false), found)?);
                    }
                    DynamicVariant::Tuple(tuple)
                }
                (VariantInfo::Struct(fields), Some(StructBody::Fields(body))) => {
                    let mut value = DynamicStruct::default();
                    for field in &body.fields {
                        let field_name = &*field.name.name;
                        let field_info = fields.field(field_name).ok_or_else(|| {
                            error(ErrorKind::Type, &field.value, format!("no field `{field_name}` in variant `{name}`"))
                        })?;
                        let field_registration = lookup(self.registry, &field.value, field_info.ty())?;
                        if let Some(v) = self.read(&field.value, field_registration, at.field(field_name, true), found)? {
                            value.insert_boxed(field_name.to_string(), v);
                        }
                    }
                    DynamicVariant::Struct(value)
                }
                _ => return Err(error(ErrorKind::Type, expr, format!("wrong shape for variant `{name}`"))),
            };
            (name, variant)
        };
        let mut value = DynamicEnum::new(name, variant);
        value.set_represented_type(Some(registration.type_info()));
        Ok(Some(Box::new(value)))
    }

    /// `Path("..")` relative to this file, or `Frozen(Path(".."))` for a blueprint handle.
    ///
    /// Blueprint handles never load `"file#label"` paths: Bevy would load the whole file again
    /// for every label. Blueprints of this file use its own label handles; blueprints of other
    /// files load that file once, and the field is filled from it on spawn.
    fn handle(
        &mut self,
        expr: &Expr,
        handle: &ReflectHandle,
        at: &At,
        found: &mut Found,
    ) -> ParseResult<Box<dyn PartialReflect>> {
        let is_blueprint = handle.asset_type_id() == TypeId::of::<ScenePatch>();
        match call(expr) {
            Some(("Path", args)) if args.len() == 1 => {
                let path = self.resolve(args[0])?;
                if is_blueprint && let Some(label) = path.label() {
                    if path.path() == self.here.path() {
                        let scene = self.load_context.get_label_handle::<ScenePatch>(label.to_string());
                        return Ok(Box::new(scene));
                    }
                    if let Some(field) = &at.path {
                        let file = self.load_context.load::<BlueprintFile>(path.without_label().into_owned());
                        let blueprint = BlueprintRef::new(field.clone(), file, label.to_string())
                            .map_err(|e| e.at(expr.span()))?;
                        found.refs.push(blueprint);
                        return Ok(Box::new(Handle::<ScenePatch>::default()));
                    }
                }
                let loaded = self.load_context.load_builder().load_erased(handle.asset_type_id(), path);
                Ok(handle.typed(loaded).into_partial_reflect())
            }
            Some(("Frozen", args)) if args.len() == 1 => {
                if !is_blueprint {
                    return Err(error(ErrorKind::Reference, expr, "Frozen(..) only works on Handle<ScenePatch> fields"));
                }
                let target = match call(args[0]) {
                    Some(("Path", path)) if path.len() == 1 => path[0],
                    _ => return Err(error(ErrorKind::Reference, args[0], "expected Frozen(Path(\"file.bp.ron#label\"))")),
                };
                let path = at
                    .path
                    .clone()
                    .ok_or_else(|| error(ErrorKind::Reference, expr, "Frozen(..) can't go inside maps or OneOf options"))?;
                found.frozen.push(FrozenRef {
                    path,
                    target: string(target)?.to_string(),
                    span: *target.span(),
                });
                // The field gets its frozen copy on spawn.
                Ok(Box::new(Handle::<ScenePatch>::default()))
            }
            _ => Err(error(ErrorKind::Reference, expr, "expected Path(\"..\")")),
        }
    }

    /// An asset path, relative to this file.
    fn resolve(&self, expr: &Expr) -> ParseResult<AssetPath<'static>> {
        let path = AssetPath::try_parse(string(expr)?).map_err(|e| error(ErrorKind::Reference, expr, e.to_string()))?;
        Ok(self.here.resolve_embed(&path))
    }

    /// Reads a value through its serde implementation.
    fn serde(&self, expr: &Expr, registration: &TypeRegistration) -> ParseResult<Box<dyn PartialReflect>> {
        let type_path = registration.type_info().type_path();
        let Some(deserialize) = registration.data::<ReflectDeserialize>() else {
            return Err(error(ErrorKind::Type, expr, format!("wrong shape for `{type_path}`")));
        };
        deserialize
            .deserialize(ExprDeserializer(expr))
            .map(|value| value.into_partial_reflect())
            .map_err(|e| error(ErrorKind::Type, expr, format!("`{type_path}`: {e}")))
    }

}

fn lookup<'r>(registry: &'r TypeRegistry, expr: &Expr, ty: &Type) -> ParseResult<&'r TypeRegistration> {
    registry
        .get(ty.id())
        .ok_or_else(|| error(ErrorKind::Type, expr, format!("type `{}` is not registered", ty.path())))
}

fn random_path(expr: &Expr, at: &At) -> ParseResult<String> {
    at.path
        .clone()
        .filter(|_| at.random_ok)
        .ok_or_else(|| error(ErrorKind::Random, expr, "random values can't go inside lists, maps or OneOf options"))
}

/// The fields of `(a: ..)` or `Name(a: ..)`; `()` has none.
fn struct_fields<'e, 'a>(expr: &'e Expr<'a>) -> Option<&'e [StructField<'a>]> {
    match expr {
        Expr::AnonStruct(s) => Some(&s.fields),
        Expr::Struct(s) => match &s.body {
            Some(StructBody::Fields(body)) => Some(&body.fields),
            None => Some(&[]),
            Some(StructBody::Tuple(_)) => None,
        },
        Expr::Unit(_) => Some(&[]),
        _ => None,
    }
}

/// The values of `(a, ..)`, or of `Name(a, ..)` for tuple structs.
fn tuple_items<'e, 'a>(expr: &'e Expr<'a>, named: bool) -> Option<Vec<&'e Expr<'a>>> {
    match expr {
        Expr::Tuple(t) => Some(t.elements.iter().map(|e| &e.expr).collect()),
        Expr::Unit(_) => Some(Vec::new()),
        Expr::Struct(_) if named => call(expr).map(|(_, args)| args),
        _ => None,
    }
}

fn is_number(type_id: TypeId) -> bool {
    macro_rules! check {
        ($($ty:ty),*) => { false $(|| type_id == TypeId::of::<$ty>())* };
    }
    number_types!(check)
}
