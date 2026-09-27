//! Recipes: a few parameters in a blueprint file, expanded to a bundle by Rust code.

use bevy::prelude::*;
use bevy::reflect::FromType;

use crate::error::{BlueprintError, ErrorKind};

/// A set of components built in Rust from a few parameters.
///
/// Written in a blueprint's `components` like a component (`"RockRecipe": (size: 3)`), with
/// the same inheritance, random fields and options. When the blueprint spawns, the recipe's
/// bundle is inserted first, so components the blueprint lists explicitly override it. The recipe
/// value itself is not added to the entity.
///
/// ```ignore
/// #[derive(Reflect, Default)]
/// #[reflect(Recipe, Default)]
/// struct RockRecipe { size: u32 }
///
/// impl Recipe for RockRecipe {
///     fn bundle(&self) -> impl Bundle + use<> {
///         (Stats { hp: self.size * 10, armor: self.size }, Speed(10.0 / self.size as f32))
///     }
/// }
/// ```
pub trait Recipe: Reflect {
    /// The components this recipe expands to. Called on every spawn, with this spawn's values
    /// (random fields already chosen). The bundle must own its data: clone what it needs from
    /// `self`.
    fn bundle(&self) -> impl Bundle + use<Self>;
}

/// Type data for [`Recipe`] types, added with `#[reflect(Recipe)]`.
#[derive(Clone)]
pub struct ReflectRecipe {
    insert: fn(&dyn Reflect, &mut EntityWorldMut) -> bool,
}

impl ReflectRecipe {
    /// Inserts the bundle of `recipe`, which must be of this recipe type.
    pub(crate) fn insert(&self, recipe: &dyn Reflect, entity: &mut EntityWorldMut) -> Result<()> {
        if (self.insert)(recipe, entity) {
            Ok(())
        } else {
            let message = format!("`{}` is not the recipe type", recipe.reflect_type_path());
            Err(BlueprintError::new(ErrorKind::Type, message).into())
        }
    }
}

impl<T: Recipe> FromType<T> for ReflectRecipe {
    fn from_type() -> Self {
        Self {
            insert: |recipe, entity| match recipe.downcast_ref::<T>() {
                Some(recipe) => {
                    entity.insert(recipe.bundle());
                    true
                }
                None => false,
            },
        }
    }
}
