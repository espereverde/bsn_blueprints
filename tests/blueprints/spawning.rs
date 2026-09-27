//! `BlueprintInstance`, `spawn_blueprint` and `insert_blueprint`.

use crate::support::*;

fn loaded_file(app: &mut App, path: &'static str) -> Handle<BlueprintFile> {
    let file: Handle<BlueprintFile> = app.world().resource::<AssetServer>().load(path);
    update_until(app, path, |world| {
        world.resource::<Assets<BlueprintFile>>().contains(&file)
    });
    file
}

#[test]
fn spawn_blueprint_waits_for_its_file() {
    let mut app = app();
    let file: Handle<BlueprintFile> = app.world().resource::<AssetServer>().load("rocks.bp.ron");
    let e = app
        .world_mut()
        .commands()
        .spawn_blueprint(&file, "big_rock")
        .insert(Transform::from_xyz(1.0, 2.0, 3.0))
        .id();
    app.world_mut().flush();
    update_until(&mut app, "big_rock", |world| world.get::<Stats>(e).is_some());
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 40, armor: 2 });
    assert_eq!(get::<Transform>(&app, e).translation, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(get::<BlueprintInstance>(&app, e).label, "big_rock");
}

#[test]
fn spawn_blueprint_with_a_loaded_file_spawns_in_the_same_frame() {
    let mut app = app();
    let file = loaded_file(&mut app, "rocks.bp.ron");
    // Let the file's scenes resolve too.
    let _ = load(&mut app, "rocks.bp.ron#rock");
    let e = app.world_mut().commands().spawn_blueprint(&file, "rock").id();
    app.world_mut().flush();
    app.update();
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 10, armor: 2 });
}

#[test]
fn insert_blueprint_adds_to_an_existing_entity() {
    let mut app = app();
    let file = loaded_file(&mut app, "rocks.bp.ron");
    let e = app.world_mut().spawn((Marker, Name::new("Before"))).id();
    app.world_mut().commands().entity(e).insert_blueprint(&file, "rock");
    app.world_mut().flush();
    update_until(&mut app, "rock", |world| world.get::<Stats>(e).is_some());
    assert!(app.world().get::<Marker>(e).is_some(), "other components are kept");
    assert_eq!(
        get::<Name>(&app, e).as_str(),
        "Rock",
        "the blueprint's components replace existing ones"
    );
}

#[test]
fn a_blueprint_instance_that_cant_spawn_is_removed() {
    let mut app = app();
    let file = loaded_file(&mut app, "rocks.bp.ron");
    let missing_label = app.world_mut().spawn(BlueprintInstance::new(&file, "pebble")).id();
    let broken: Handle<BlueprintFile> = app.world().resource::<AssetServer>().load("missing_parent.bp.ron");
    let broken_file = app.world_mut().spawn(BlueprintInstance::new(&broken, "orphan")).id();
    update_until(&mut app, "both to fail", |world| {
        world.get::<BlueprintInstance>(missing_label).is_none() && world.get::<BlueprintInstance>(broken_file).is_none()
    });
    for e in [missing_label, broken_file] {
        assert!(app.world().get_entity(e).is_ok(), "the entity itself stays");
        assert!(app.world().get::<Stats>(e).is_none());
    }
}
