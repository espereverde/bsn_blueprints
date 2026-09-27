//! `BlueprintLoadingPlugin`, `Blueprints<T>`, `BlueprintSet<T>` and loading progress.

use crate::support::*;

#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum GameState {
    #[default]
    Loading,
    Menu,
    LevelLoading,
    Playing,
}

/// `(loaded, failed, total)`.
fn progress(app: &App) -> (usize, usize, usize) {
    let progress = app.world().resource::<BlueprintLoadingProgress<GameState>>();
    (progress.loaded(), progress.failed(), progress.total())
}

fn state(world: &World) -> GameState {
    *world.resource::<State<GameState>>().get()
}

struct RockFile;
struct TurretFile;
struct BrokenFile;

#[derive(Resource)]
struct SpawnedRock(Entity);

fn loading_app(plugin: BlueprintLoadingPlugin<GameState>) -> App {
    let mut app = app();
    app.add_plugins((StatesPlugin, plugin)).init_state::<GameState>();
    app
}

#[test]
fn loading_plugin_switches_state_when_files_are_loaded() {
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing)
            .load::<RockFile>("rocks.bp.ron")
            .load::<TurretFile>("turrets.bp.ron"),
    );
    app.add_systems(
        OnEnter(GameState::Playing),
        |mut commands: Commands, rocks: Res<Blueprints<RockFile>>| {
            let rock = commands.spawn_blueprint(&rocks, "rock").id();
            commands.insert_resource(SpawnedRock(rock));
        },
    );

    app.update();
    assert!(
        app.world().contains_resource::<Blueprints<RockFile>>(),
        "resources exist once loading starts"
    );
    assert!(app.world().contains_resource::<Blueprints<TurretFile>>());

    update_until(&mut app, "Playing", |world| {
        *world.resource::<State<GameState>>() == GameState::Playing
    });
    assert_eq!(progress(&app), (2, 0, 2));
    assert_eq!(
        app.world().resource::<BlueprintLoadingProgress<GameState>>().fraction(),
        1.0
    );

    let rock = app.world().resource::<SpawnedRock>().0;
    update_until(&mut app, "the rock", |world| world.get::<Stats>(rock).is_some());
    assert_eq!(get::<Stats>(&app, rock), Stats { hp: 10, armor: 2 });
}

#[test]
fn loading_plugin_stays_in_loading_state_if_a_file_fails() {
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing)
            .load::<RockFile>("rocks.bp.ron")
            .load::<BrokenFile>("missing_parent.bp.ron"),
    );
    update_until(&mut app, "the load to settle", |world| {
        let progress = world.resource::<BlueprintLoadingProgress<GameState>>();
        progress.loaded() + progress.failed() == 2
    });
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(progress(&app), (1, 1, 2));
    assert_eq!(*app.world().resource::<State<GameState>>(), GameState::Loading);
}

struct Scenery;

#[test]
fn loading_plugin_loads_a_named_set_of_files() {
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing)
            .load::<TurretFile>("turrets.bp.ron")
            .load_set::<Scenery>([("rocks", "rocks.bp.ron"), ("saucers", "saucers.bp.ron")]),
    );
    update_until(&mut app, "Playing", |world| {
        *world.resource::<State<GameState>>() == GameState::Playing
    });
    assert_eq!(progress(&app).2, 3);

    let set = app.world().resource::<BlueprintSet<Scenery>>();
    let mut names: Vec<_> = set.names().collect();
    names.sort_unstable();
    assert_eq!(names, ["rocks", "saucers"]);
    let rocks = set["rocks"].clone();
    let big_rock = set.instance("rocks.big_rock").expect("file.label");
    assert!(set.instance("pebbles.rock").is_none(), "unknown file");
    assert!(set.instance("rocks").is_none(), "no label");

    let rock = app.world_mut().commands().spawn_blueprint(&rocks, "rock").id();
    let big = app.world_mut().spawn(big_rock).id();
    app.world_mut().flush();
    app.update();
    assert_eq!(get::<Stats>(&app, rock), Stats { hp: 10, armor: 2 });
    assert_eq!(get::<Stats>(&app, big), Stats { hp: 40, armor: 2 });
}

#[test]
#[should_panic(expected = "no blueprint file `pebbles` in BlueprintSet")]
fn blueprint_set_index_panics_on_unknown_file() {
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing)
            .load_set::<Scenery>([("rocks", "rocks.bp.ron")]),
    );
    app.update();
    let _ = &app.world().resource::<BlueprintSet<Scenery>>()["pebbles"];
}

#[test]
#[should_panic(expected = "without `.`")]
fn blueprint_set_names_cannot_contain_dots() {
    let _ = BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing)
        .load_set::<Scenery>([("rocks.v2", "rocks.bp.ron")]);
}

#[test]
fn loading_plugins_for_the_same_state_are_combined() {
    // As if two game plugins each registered their own files.
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing).load::<RockFile>("rocks.bp.ron"),
    );
    app.add_plugins(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing).load::<TurretFile>("turrets.bp.ron"),
    );
    update_until(&mut app, "Playing", |world| state(world) == GameState::Playing);
    assert_eq!(progress(&app), (2, 0, 2));
    assert!(app.world().contains_resource::<Blueprints<RockFile>>());
    assert!(app.world().contains_resource::<Blueprints<TurretFile>>());
}

#[test]
fn loading_plugins_for_different_states_load_their_own_files() {
    let mut app =
        loading_app(BlueprintLoadingPlugin::new(GameState::Loading, GameState::Menu).load::<RockFile>("rocks.bp.ron"));
    app.add_plugins(
        BlueprintLoadingPlugin::new(GameState::LevelLoading, GameState::Playing).load::<TurretFile>("turrets.bp.ron"),
    );

    update_until(&mut app, "Menu", |world| state(world) == GameState::Menu);
    assert_eq!(progress(&app), (1, 0, 1));
    assert!(app.world().contains_resource::<Blueprints<RockFile>>());
    assert!(
        !app.world().contains_resource::<Blueprints<TurretFile>>(),
        "not loaded before its state"
    );

    app.world_mut()
        .resource_mut::<NextState<GameState>>()
        .set(GameState::LevelLoading);
    update_until(&mut app, "Playing", |world| state(world) == GameState::Playing);
    assert_eq!(progress(&app), (1, 0, 1));
    assert!(app.world().contains_resource::<Blueprints<TurretFile>>());
}

#[test]
#[should_panic(expected = "already switches to Playing; it can't also switch to Menu")]
fn loading_plugins_for_the_same_state_must_agree_on_the_next() {
    let mut app = loading_app(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Playing).load::<RockFile>("rocks.bp.ron"),
    );
    app.add_plugins(
        BlueprintLoadingPlugin::new(GameState::Loading, GameState::Menu).load::<TurretFile>("turrets.bp.ron"),
    );
}
