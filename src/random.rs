//! Values sampled every time a blueprint is spawned.

use std::fmt::Display;
use std::sync::Arc;

use bevy::reflect::{GetPath, ParsedPath, PartialReflect, Reflect};
use rand::Rng;

use crate::error::{BlueprintError, ErrorKind};

/// How a random field gets its value.
pub(crate) enum RandomSpec {
    /// Uniform value in `[min, max]`, for any integer or float field.
    Range(f64, f64),
    /// One of these values, by weight.
    Pick(Vec<(f64, Box<dyn PartialReflect>)>),
}

impl RandomSpec {
    pub(crate) fn range(min: f64, max: f64) -> Result<Self, BlueprintError> {
        if min > max {
            return Err(BlueprintError::new(
                ErrorKind::Random,
                format!("Range({min}, {max}): min > max"),
            ));
        }
        Ok(Self::Range(min, max))
    }

    pub(crate) fn pick(options: Vec<(f64, Box<dyn PartialReflect>)>) -> Result<Self, BlueprintError> {
        if options.is_empty() {
            return Err(BlueprintError::new(
                ErrorKind::Random,
                "OneOf(..) needs at least one option",
            ));
        }
        Ok(Self::Pick(options))
    }
}

/// A field of a component that gets a new random value every time the blueprint is spawned.
/// The path is relative to the component; `""` is the whole component.
#[derive(Clone)]
pub(crate) struct RandomField {
    pub(crate) path: String,
    /// `path`, parsed once; `None` for the whole component.
    parsed: Option<Arc<ParsedPath>>,
    spec: Arc<RandomSpec>,
}

impl RandomField {
    /// `spec` must hold whole values for the field (see `Blueprint::set_random`).
    pub(crate) fn new(path: String, spec: RandomSpec) -> Result<Self, BlueprintError> {
        let parsed = match path.as_str() {
            "" => None,
            path => Some(Arc::new(ParsedPath::parse(path).map_err(|e| {
                BlueprintError::new(ErrorKind::Random, format!("random field `{path}`: {e}"))
            })?)),
        };
        Ok(Self {
            path,
            parsed,
            spec: Arc::new(spec),
        })
    }

    /// Sets the field in `component` to a new random value.
    pub(crate) fn apply(&self, component: &mut dyn Reflect) -> Result<(), BlueprintError> {
        let error =
            |e: &dyn Display| BlueprintError::new(ErrorKind::Random, format!("random field `{}`: {e}", self.path));
        let field = match &self.parsed {
            None => component.as_partial_reflect_mut(),
            Some(parsed) => component.reflect_path_mut(&**parsed).map_err(|e| error(&e))?,
        };
        match &*self.spec {
            RandomSpec::Range(min, max) => {
                set_random_number(field, *min, *max, &mut rand::rng()).map_err(|e| error(&e))?
            }
            RandomSpec::Pick(options) => field.try_apply(&**choose_weighted(options)).map_err(|e| error(&e))?,
        }
        Ok(())
    }
}

fn set_random_number(field: &mut dyn PartialReflect, min: f64, max: f64, rng: &mut impl Rng) -> Result<(), String> {
    macro_rules! try_number {
        ($($ty:ty),*) => {$(
            if let Some(value) = field.try_downcast_mut::<$ty>() {
                *value = rng.random_range(min as $ty..=max as $ty);
                return Ok(());
            }
        )*};
    }
    number_types!(try_number);
    Err(format!(
        "Range only works on numeric fields, found `{}`",
        field.reflect_type_path()
    ))
}

/// Picks an option with probability proportional to its weight. `options` is never empty.
pub(crate) fn choose_weighted<T>(options: &[(f64, T)]) -> &T {
    let total: f64 = options.iter().map(|(weight, _)| weight).sum();
    let mut roll = rand::rng().random_range(0.0..total);
    for (weight, option) in options {
        roll -= weight;
        if roll < 0.0 {
            return option;
        }
    }
    &options[options.len() - 1].1
}
