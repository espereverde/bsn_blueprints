//! A small headless "game": a loading screen, then turrets that shoot random bullets.
//!
//! ```sh
//! cargo run --example armory
//! ```
//!
//! Shows loading blueprint files during a loading state, spawning blueprints by name, a recipe
//! computing a component (a `Timer` from a fire rate), and blueprint references: plain ones make
//! a new random bullet every shot, `Frozen` ones make every shot of a turret identical. The
//! blueprints are in `assets/examples/`.

use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::scene::{ScenePatch, ScenePatchInstance, ScenePlugin};
use bevy::state::app::StatesPlugin;
use bsn_blueprints::{
    BlueprintCommandsExt, BlueprintLoadingPlugin, BlueprintLoadingProgress, BlueprintPlugin, BlueprintSet, Recipe,
    ReflectRecipe,
};

// ---- Game types: plain reflected components ----

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Bullet {
    speed: f32,
    damage: u32,
}

#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component, Default)]
enum Tint {
    #[default]
    Red,
    Green,
    Blue,
}

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Size(f32);

/// Built by [`GunRecipe`]: the timer can't be written in a file, only computed.
#[derive(Component)]
struct Gun {
    bullet: Handle<ScenePatch>,
    cooldown: Timer,
}

/// A gun, as written in a blueprint file.
#[derive(Reflect, Default)]
#[reflect(Recipe, Default)]
struct GunRecipe {
    bullet: Handle<ScenePatch>,
    shots_per_second: f32,
}

impl Recipe for GunRecipe {
    fn bundle(&self) -> impl Bundle + use<> {
        Gun {
            bullet: self.bullet.clone(),
            cooldown: Timer::from_seconds(1.0 / self.shots_per_second, TimerMode::Repeating),
        }
    }
}

/// Which turret a bullet came from.
#[derive(Component)]
struct FiredBy(Entity);

// ---- States and blueprint files ----

#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum GameState {
    #[default]
    Loading,
    Playing,
}

/// Names the set of blueprint files of the level.
struct Level;

fn main() -> AppExit {
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))),
            LogPlugin::default(),
            AssetPlugin::default(),
            ScenePlugin,
            StatesPlugin,
            BlueprintPlugin,
            BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing).load_set::<Level>([
                ("armory", "examples/armory.bp.ron"),
                ("scenery", "examples/scenery.bp.ron"),
            ]),
        ))
        .init_state::<GameState>()
        .register_type::<Name>()
        .register_type::<Bullet>()
        .register_type::<Tint>()
        .register_type::<Size>()
        .register_type::<GunRecipe>()
        .add_systems(
            Update,
            show_progress
                .run_if(in_state(GameState::Loading).and_then(resource_changed::<BlueprintLoadingProgress<GameState>>)),
        )
        .add_systems(OnEnter(GameState::Playing), spawn_level)
        .add_systems(
            Update,
            (fire, report_rocks, report_bullets, stop_after_a_while).run_if(in_state(GameState::Playing)),
        )
        .run()
}

// ---- Systems ----

/// The "loading screen". A file that fails keeps the app in the loading state (the error is
/// logged); a real game might show it and offer to retry. This one gives up.
fn show_progress(progress: Res<BlueprintLoadingProgress<GameState>>, mut exit: MessageWriter<AppExit>) {
    println!(
        "Loading... {}/{} files ({:.0}%)",
        progress.loaded(),
        progress.total(),
        progress.fraction() * 100.0
    );
    if progress.failed() > 0 {
        exit.write(AppExit::error());
    }
}

fn spawn_level(
    mut commands: Commands,
    level: Res<BlueprintSet<Level>>,
    progress: Res<BlueprintLoadingProgress<GameState>>,
) {
    println!("Loaded {} files; spawning the level.\n", progress.loaded());
    for turret in ["turret", "steady_turret", "fast_turret"] {
        commands.spawn_blueprint(&level["armory"], turret);
    }
    for _ in 0..3 {
        commands.spawn(level.instance("scenery.rock").expect("the level has scenery"));
    }
}

fn fire(mut commands: Commands, time: Res<Time>, mut guns: Query<(Entity, &mut Gun)>) {
    for (turret, mut gun) in &mut guns {
        if gun.cooldown.tick(time.delta()).just_finished() {
            // The bullet is itself a blueprint: spawning its scene makes its random choices.
            commands.spawn((ScenePatchInstance(gun.bullet.clone()), FiredBy(turret)));
        }
    }
}

fn report_rocks(rocks: Query<(&Size, &Tint), Added<Size>>) {
    for (size, tint) in &rocks {
        println!("A rock appears: size {:.1}, {tint:?}", size.0);
    }
}

fn report_bullets(bullets: Query<(&Bullet, &Tint, &FiredBy, Option<&Children>), Added<Bullet>>, names: Query<&Name>) {
    for (bullet, tint, fired_by, children) in &bullets {
        let turret = names.get(fired_by.0).map_or("?", |name| name.as_str());
        let trail = children.is_some_and(|children| {
            children
                .iter()
                .any(|child| names.get(child).is_ok_and(|name| name.as_str() == "Trail"))
        });
        println!(
            "{turret:>13} fired: speed {:>5.1}, damage {}, {tint:?}{}",
            bullet.speed,
            bullet.damage,
            if trail { ", with a trail" } else { "" }
        );
    }
}

fn stop_after_a_while(time: Res<Time>, mut playing_for: Local<f32>, mut exit: MessageWriter<AppExit>) {
    *playing_for += time.delta_secs();
    if *playing_for > 1.6 {
        println!("\nThe steady turret's bullets are all alike; the others vary.");
        exit.write(AppExit::Success);
    }
}
