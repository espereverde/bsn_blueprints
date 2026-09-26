//! Spike: data-driven blueprints on top of Bevy 0.19 scenes (BSN).
//!
//! A `*.bp.ron` file is a map of `label -> blueprint`. Every blueprint becomes a labeled
//! `ScenePatch` sub-asset (`rocks.bp.ron#big_rock`), so it can be spawned with the stock scene
//! API or inherited from `bsn!` (`:"rocks.bp.ron#big_rock"`).
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
//! can be written partially; others must be complete the first time they appear in a chain. A
//! component entry can also be a [`Recipe`]: a few parameters expanded into a bundle by Rust
//! code, inserted before the listed components (which override it).
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
//! # Asset handles
//!
//! Handle fields take `Path("..")`, relative to the blueprint file (`"#label"` is another
//! blueprint of the same file, `"/x.png"` is relative to the assets folder). References to
//! blueprints (`Handle<ScenePatch>` fields, also inside lists) load each referenced file once, however
//! many of its blueprints are used. A
//! `Handle<ScenePatch>` field can refer to another blueprint, to be spawned later by game code
//! (e.g. a gun's bullets); each such spawn makes new random choices. Wrapped in `Frozen(..)`,
//! the field instead gets a copy with every random choice made once, when the owning component
//! is spawned, so each owner keeps spawning identical entities:
//!
//! ```ron
//! "varied_turret": ( components: { "Ammo": (bullet: Path("#bullet")) } ),         // new bullet each shot
//! "steady_turret": ( components: { "Ammo": (bullet: Frozen(Path("#bullet"))) } ), // same bullet per turret
//! ```
//!
//! # Loading
//!
//! Load the file and get blueprints from it by name; the file keeps them alive:
//!
//! ```ignore
//! let file: Handle<BlueprintFile> = asset_server.load("enemies.bp.ron");
//! // once loaded:
//! let ufo = files.get(&file).unwrap().get("ufo").unwrap().clone();
//! commands.spawn(ScenePatchInstance(ufo));
//! ```
//!
//! Loading `"enemies.bp.ron#ufo"` directly also works, but Bevy loads the whole file once for
//! every label requested before the file has loaded, so avoid requesting many that way.
//!
//! Files are parsed with `ron2`, which keeps every name and position, so wrappers are recognized
//! anywhere and errors point at `line:col`. Inheritance is flattened inside the loader
//! (same-file parents in memory, other files through an immediate nested load), so resolution
//! order of the resulting scene patches does not matter.

/// Calls `$mac!` with every numeric type a random `Range` can fill.
macro_rules! number_types {
    ($mac:ident) => {
        $mac!(f32, f64, i8, i16, i32, i64, u8, u16, u32, u64, usize)
    };
}

mod blueprint;
mod flatten;
mod freeze;
mod loader;
mod parse;
mod random;
mod recipe;
mod reflect_utils;
mod spawn;

use bevy::prelude::*;

pub use loader::{BlueprintFile, BlueprintLoader};
pub use recipe::{Recipe, ReflectRecipe};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Registers the `*.bp.ron` loader.
pub struct BlueprintPlugin;

impl Plugin for BlueprintPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<BlueprintFile>()
            .init_asset_loader::<BlueprintLoader>();
    }
}
