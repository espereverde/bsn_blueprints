//! Data-driven game entities for Bevy, defined in `*.bp.ron` files that can be edited without
//! recompiling. Blueprints inherit from each other, refer to each other, and describe random
//! variation that is rolled every time an entity is spawned.
//!
//! Each blueprint becomes a regular Bevy scene (`ScenePatch`), so it spawns through the stock
//! scene API and can be used from `bsn!`. The README has the full format reference.
//!
//! # Quick start
//!
//! ```ignore
//! App::new()
//!     .add_plugins((DefaultPlugins, BlueprintPlugin))
//!     .register_type::<Stats>()   // every type used in a blueprint must be registered
//!     .add_systems(Startup, |mut commands: Commands, server: Res<AssetServer>| {
//!         let enemies = server.load("enemies.bp.ron");
//!         commands.spawn_blueprint(&enemies, "ufo");   // spawns once the file has loaded
//!     })
//!     .run();
//! ```
//!
//! # Blueprint files
//!
//! A file is a map of `label -> blueprint`:
//!
//! ```ron
//! {
//!     "rock": (
//!         components: {
//!             "Name": "Rock",
//!             "Stats": (hp: 10, armor: 2),
//!             "Speed": (1.0),
//!         },
//!     ),
//!     "big_rock": (
//!         extends: "#rock",                     // same file; "other.bp.ron#x" for another file
//!         components: { "Stats": (hp: 40) },    // field-level override: armor is inherited
//!         children: {                           // named, so derived blueprints can patch them
//!             "shard": ( extends: "#shard" ),
//!         },
//!     ),
//! }
//! ```
//!
//! Components are read through the type registry, so any `#[derive(Reflect)]
//! #[reflect(Component)]` type works without extra code. Types that also `#[reflect(Default)]`
//! can be written partially; others must be complete the first time they appear in a chain.
//!
//! # Random values
//!
//! Everything is fixed unless wrapped. The same wrappers work at every level:
//!
//! | Wrapper | Value / field | Component | Set of components | Child | Blueprint |
//! |---|---|---|---|---|---|
//! | `OneOf([a, Weight(3, b)])` | pick one | pick one value | pick one set | pick one | pick one |
//! | `Maybe(p, x)` | – | add it or not | add it or not | spawn it or not | – |
//! | `Range(min, max)` | numbers | – | – | – | – |
//! | `Fixed(x)` | `x` as written, even if it is named like a wrapper | | | | |
//!
//! ```ron
//! "saucer": OneOf([
//!     (
//!         extends: "#base",
//!         components: [                                   // a list: sets applied in order
//!             { "Speed": (Range(1.0, 2.0)), "Tint": OneOf([Gold, Grey]) },
//!             Maybe(0.5, { "Bounty": (gold: 10) }),
//!         ],
//!         children: { "pilot": Maybe(0.3, ( extends: "#pilot" )) },
//!     ),
//!     ( extends: "#bomber", components: { "Stats": OneOf([(hp: 5), (armor: 9)]) } ),
//! ]),
//! ```
//!
//! Random choices are made on every spawn. Chosen sets are applied after the fixed components
//! and merge onto them (they only change the fields they set). A blueprint that extends a
//! `OneOf` applies its changes to every option. A child that sets a field replaces any random
//! value it inherited there.
//!
//! Redeclaring an inherited child as a plain entry patches it (every option, if it is random).
//! Redeclaring it with `OneOf` / `Maybe` replaces its randomness: the options build on the
//! inherited child without its `Maybe`s (`"lamp": Maybe(0.5, ())` makes a child optional,
//! with its values), or on nothing if the inherited child is a `OneOf`.
//!
//! # Asset handles and blueprint references
//!
//! Handle fields take `Path("..")`, relative to the blueprint file (`"#label"` is another
//! blueprint of the same file, `"/x.png"` is relative to the assets folder). A
//! `Handle<ScenePatch>` field can refer to another blueprint, for game code to spawn later (a
//! gun's bullets); each such spawn makes new random choices. Wrapped in `Frozen(..)`, the field
//! instead gets a copy with every random choice made once, when the owning component is
//! spawned, so each owner keeps spawning identical entities:
//!
//! ```ron
//! "varied_turret": ( components: { "Ammo": (bullet: Path("#bullet")) } ),         // new bullet each shot
//! "steady_turret": ( components: { "Ammo": (bullet: Frozen(Path("#bullet"))) } ), // same bullet per turret
//! ```
//!
//! Referenced files are loaded once, however many of their blueprints are used.
//!
//! # Spawning
//!
//! Keep the file's handle and spawn blueprints from it by label, with
//! [`spawn_blueprint`](BlueprintCommandsExt::spawn_blueprint),
//! [`insert_blueprint`](BlueprintEntityCommandsExt::insert_blueprint), or a
//! [`BlueprintInstance`] component. None of them need the file to be loaded yet:
//!
//! ```ignore
//! let enemies: Handle<BlueprintFile> = asset_server.load("enemies.bp.ron");
//! commands.spawn_blueprint(&enemies, "ufo").insert(Transform::from_xyz(0.0, 50.0, 0.0));
//! commands.entity(boss).insert_blueprint(&enemies, "mothership");
//! commands.spawn((BlueprintInstance::new(&enemies, "ufo"), Transform::default()));
//! ```
//!
//! Once loaded, [`BlueprintFile::get`] gives each blueprint's scene. Avoid loading
//! `"enemies.bp.ron#ufo"` paths for many labels: Bevy loads the whole file again for every label
//! requested before the file has loaded.
//!
//! # Loading state
//!
//! [`BlueprintLoadingPlugin`] loads files when the app enters a loading state and switches to
//! the next state once they (and everything they depend on) have loaded. Files go into
//! [`Blueprints<T>`] and [`BlueprintSet<T>`] resources; [`BlueprintLoadingProgress`] feeds a
//! loading screen.
//!
//! ```ignore
//! struct Enemies;
//!
//! app.add_plugins(BlueprintLoadingPlugin::new(AppState::Loading, AppState::Playing).load::<Enemies>("enemies.bp.ron"))
//!     .add_systems(OnEnter(AppState::Playing), |mut commands: Commands, enemies: Res<Blueprints<Enemies>>| {
//!         commands.spawn_blueprint(&enemies, "ufo");
//!     });
//! ```
//!
//! # Recipes
//!
//! A component entry can also be a [`Recipe`]: a few parameters expanded into a bundle by Rust
//! code, for values that must be computed (a sprite atlas, a timer from a frame rate). Its
//! bundle is inserted before the listed components, which override it.
//!
//! # Errors
//!
//! Loading fails with a [`BlueprintError`] that names the file and `line:col` and says what was
//! expected:
//!
//! ```text
//! enemies.bp.ron:2:49: no field `colour` in `game::Stats`; expected one of ["hp", "armor"]
//! ```
//!
//! Its [`ErrorKind`] tells syntax, type, random, inheritance and reference problems apart.

#![warn(missing_docs)]

/// Calls `$mac!` with every numeric type a random `Range` can fill.
macro_rules! number_types {
    ($mac:ident) => {
        $mac!(f32, f64, i8, i16, i32, i64, u8, u16, u32, u64, usize)
    };
}

mod blueprint;
mod error;
mod flatten;
mod freeze;
mod instance;
mod loader;
mod loading;
mod parse;
mod random;
mod recipe;
mod reflect_utils;
mod spawn;

use bevy::app::SceneSpawnerSystems;
use bevy::prelude::*;

pub use error::{BlueprintError, ErrorKind, Position};
pub use instance::{BlueprintCommandsExt, BlueprintEntityCommandsExt, BlueprintInstance};
pub use loader::{BlueprintFile, BlueprintLoader};
pub use loading::{BlueprintLoadingPlugin, BlueprintLoadingProgress, BlueprintSet, Blueprints};
pub use recipe::{Recipe, ReflectRecipe};

/// Registers the `*.bp.ron` loader and spawns [`BlueprintInstance`]s. Needs Bevy's
/// `ScenePlugin`.
pub struct BlueprintPlugin;

impl Plugin for BlueprintPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<BlueprintFile>()
            .init_asset_loader::<BlueprintLoader>()
            .add_systems(
                SpawnScene,
                instance::spawn_blueprint_instances.before(SceneSpawnerSystems::SceneSpawn),
            );
    }
}
