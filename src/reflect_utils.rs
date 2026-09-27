//! Helpers for walking and copying reflected values.

use bevy::reflect::{PartialReflect, Reflect, ReflectFromReflect, ReflectRef, TypeRegistry};

use crate::error::{BlueprintError, ErrorKind};

pub(crate) fn clone_value(registry: &TypeRegistry, value: &dyn Reflect) -> Result<Box<dyn Reflect>, BlueprintError> {
    if let Ok(clone) = value.reflect_clone() {
        return Ok(clone);
    }
    registry
        .get_type_data::<ReflectFromReflect>(value.type_id())
        .and_then(|from_reflect| from_reflect.from_reflect(value.as_partial_reflect()))
        .ok_or_else(|| BlueprintError::new(ErrorKind::Type, format!("cannot clone `{}`", value.reflect_type_path())))
}

/// The fields of a struct, tuple struct or tuple, with their path segments (`name` or index).
/// `None` for anything else, which is treated as a leaf value.
pub(crate) fn fields(value: &dyn PartialReflect) -> Option<Vec<(String, &dyn PartialReflect)>> {
    match value.reflect_ref() {
        ReflectRef::Struct(value) => Some(
            value
                .iter_fields()
                .map(|(name, field)| (name.to_string(), field))
                .collect(),
        ),
        ReflectRef::TupleStruct(value) => Some(
            value
                .iter_fields()
                .enumerate()
                .map(|(i, field)| (i.to_string(), field))
                .collect(),
        ),
        ReflectRef::Tuple(value) => Some(
            value
                .iter_fields()
                .enumerate()
                .map(|(i, field)| (i.to_string(), field))
                .collect(),
        ),
        _ => None,
    }
}

/// The paths of the leaf fields a (partial) value sets.
pub(crate) fn leaf_paths(value: &dyn PartialReflect) -> Vec<String> {
    fn collect(value: &dyn PartialReflect, path: &str, out: &mut Vec<String>) {
        match fields(value) {
            Some(fields) => {
                for (segment, field) in fields {
                    collect(field, &join_path(path, &segment), out);
                }
            }
            None => out.push(path.to_string()),
        }
    }
    let mut out = Vec::new();
    collect(value, "", &mut out);
    out
}

pub(crate) fn join_path(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_string()
    } else {
        format!("{path}.{segment}")
    }
}

/// True if one path is the other or contains it (`""` is the whole value).
pub(crate) fn paths_overlap(a: &str, b: &str) -> bool {
    let contains =
        |outer: &str, inner: &str| outer.is_empty() || inner == outer || inner.starts_with(&format!("{outer}."));
    contains(a, b) || contains(b, a)
}
