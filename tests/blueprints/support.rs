//! Shared by all tests: "game" types (plain reflected components, no blueprint-specific
//! code), a test asset, the app harness, and imports.

pub use std::time::Duration;

pub use bevy::asset::io::Reader;
pub use bevy::asset::{AssetLoadError, AssetLoader, LoadContext, RecursiveDependencyLoadState, ReflectAsset};
pub use bevy::prelude::*;
pub use bevy::scene::{CachedSceneAsset, ScenePatch, ScenePatchInstance, ScenePlugin, WorldSceneExt, bsn};
pub use bevy::state::app::StatesPlugin;
pub use bsn_blueprints::{
    BlueprintCommandsExt, BlueprintEntityCommandsExt, BlueprintError, BlueprintFile, BlueprintInstance,
    BlueprintLoadingPlugin, BlueprintLoadingProgress, BlueprintPlugin, BlueprintSet, Blueprints, ErrorKind, Position,
    Recipe, ReflectRecipe,
};
pub use serde::Deserialize;

// ---- Game types ----

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct Stats {
    pub hp: u32,
    pub armor: u32,
}

#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
pub struct Speed(pub f32);

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub enum Tint {
    #[default]
    Grey,
    Gold,
    Custom(u8),
}

/// No `#[reflect(Default)]`: must be written completely the first time.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component)]
pub struct Points(pub u32);

/// No `#[reflect(Default)]` either, with named fields.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
pub struct Bounty {
    pub gold: u32,
    pub gems: u32,
}

#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
pub struct Skin(pub Handle<Palette>);

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct Placement {
    pub offset: Offset,
    pub layer: u8,
    pub facing: Facing,
}

#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq)]
#[reflect(Default)]
pub enum Facing {
    #[default]
    Left,
    Right,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct Offset {
    pub x: f32,
    pub y: f32,
}

#[derive(Component, Default, Clone)]
pub struct Marker;

/// A recipe: `size` expands to several components, and a child entity.
#[derive(Reflect, Default, Clone)]
#[reflect(Recipe, Default)]
pub struct RockRecipe {
    pub size: u32,
}

impl Recipe for RockRecipe {
    fn bundle(&self) -> impl Bundle + use<> {
        (
            Stats {
                hp: self.size * 10,
                armor: self.size,
            },
            Speed(10.0 / self.size as f32),
            Points(self.size * 5),
            children![Name::new("Shard")],
        )
    }
}

/// An enum with a variant named like a wrapper.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub enum Mode {
    #[default]
    Other,
    OneOf,
}

/// Bevy's required components must still be added when a blueprint inserts this.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
#[require(Marker, Speed(7.0))]
pub struct Shielded;

/// Several kinds of ammunition.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
pub struct Arsenal {
    pub bullets: Vec<Handle<ScenePatch>>,
}

/// A gun's ammunition: the blueprint of the bullets it fires.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
pub struct Ammo {
    pub bullet: Handle<ScenePatch>,
}

#[derive(Asset, Reflect, Deserialize)]
#[reflect(Asset)]
pub struct Palette {
    pub colors: Vec<String>,
}

#[derive(TypePath, Default)]
pub struct PaletteLoader;

impl AssetLoader for PaletteLoader {
    type Asset = Palette;
    type Settings = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    async fn load(&self, reader: &mut dyn Reader, _: &(), _: &mut LoadContext<'_>) -> Result<Palette, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ron::de::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &["palette.ron"]
    }
}

// ---- Harness ----

pub fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin, BlueprintPlugin))
        .init_asset::<Palette>()
        .register_asset_reflect::<Palette>()
        .init_asset_loader::<PaletteLoader>()
        .register_type::<Name>()
        .register_type::<Stats>()
        .register_type::<Speed>()
        .register_type::<Tint>()
        .register_type::<Points>()
        .register_type::<Skin>()
        .register_type::<Placement>()
        .register_type::<Bounty>()
        .register_type::<Ammo>()
        .register_type::<Shielded>()
        .register_type::<RockRecipe>()
        .register_type::<Mode>()
        .register_type::<Arsenal>();
    app
}

/// Loads a blueprint and runs the app until its scene patch is resolved.
pub fn load(app: &mut App, path: &'static str) -> Handle<ScenePatch> {
    let handle = app.world().resource::<AssetServer>().load(path);
    for _ in 0..1000 {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(state) = server.get_recursive_dependency_load_state(&handle)
            && state.is_failed()
        {
            panic!("{path} failed to load: {state:?}");
        }
        let patches = app.world().resource::<Assets<ScenePatch>>();
        if patches.get(&handle).is_some_and(|p| p.resolved.is_some()) {
            return handle;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("{path} did not load in time");
}

pub fn spawn(app: &mut App, path: &'static str) -> Entity {
    app.world_mut()
        .spawn_scene(CachedSceneAsset::from(path))
        .expect("spawn")
        .id()
}

pub fn get<C: Component + Clone>(app: &App, entity: Entity) -> C {
    app.world()
        .get::<C>(entity)
        .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<C>()))
        .clone()
}

pub fn children(app: &App, entity: Entity) -> Vec<Entity> {
    app.world()
        .get::<Children>(entity)
        .map(|c| c.iter().collect())
        .unwrap_or_default()
}

pub fn child_named(app: &App, entity: Entity, name: &str) -> Option<Entity> {
    children(app, entity)
        .into_iter()
        .find(|c| app.world().get::<Name>(*c).is_some_and(|n| n.as_str() == name))
}

/// Runs the app until `done` holds.
pub fn update_until(app: &mut App, what: &str, mut done: impl FnMut(&World) -> bool) {
    for _ in 0..1000 {
        app.update();
        if done(app.world()) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out waiting for {what}");
}
