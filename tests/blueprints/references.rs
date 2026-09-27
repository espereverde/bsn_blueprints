//! Blueprint references in components: plain `Path`s and `Frozen` copies.

use crate::support::*;

/// Spawns one bullet from a gun's ammo, like a shooting system would.
fn fire(app: &mut App, gun: Entity) -> Entity {
    let bullet = get::<Ammo>(app, gun).bullet;
    spawn_from(app, bullet)
}

/// Spawns a blueprint from a handle, waiting until it is ready.
fn spawn_from(app: &mut App, scene: Handle<ScenePatch>) -> Entity {
    let entity = app.world_mut().spawn(ScenePatchInstance(scene)).id();
    for _ in 0..1000 {
        if app.world().get::<Name>(entity).is_some() {
            return entity;
        }
        app.update();
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("bullet was never spawned");
}

/// Everything random about a bullet: speed, the one_of choice, the maybe part, and its child.
fn fingerprint(app: &App, bullet: Entity) -> String {
    let world = app.world();
    let trail = child_named(app, bullet, "Trail").expect("trail child");
    format!(
        "speed {} tint {:?} points {:?} bounty {} trail {:?}",
        get::<Speed>(app, bullet).0,
        world.get::<Tint>(bullet),
        world.get::<Points>(bullet),
        world.get::<Bounty>(bullet).is_some(),
        get::<Stats>(app, trail),
    )
}

#[test]
fn frozen_blueprint_is_fixed_per_owner() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#steady_turret");

    let mut per_turret = Vec::new();
    for _ in 0..2 {
        let turret = spawn(&mut app, "turrets.bp.ron#steady_turret");
        let shots: Vec<String> = (0..5)
            .map(|_| {
                let bullet = fire(&mut app, turret);
                fingerprint(&app, bullet)
            })
            .collect();
        assert!(
            shots.iter().all(|s| *s == shots[0]),
            "one turret, different bullets: {shots:#?}"
        );
        per_turret.push(shots[0].clone());
    }
    assert_ne!(per_turret[0], per_turret[1], "two turrets fired identical bullets");
}

#[test]
fn plain_blueprint_handle_varies_per_shot() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#varied_turret");
    let turret = spawn(&mut app, "turrets.bp.ron#varied_turret");
    let shots: Vec<String> = (0..5)
        .map(|_| {
            let bullet = fire(&mut app, turret);
            fingerprint(&app, bullet)
        })
        .collect();
    assert!(shots.iter().any(|s| *s != shots[0]), "all shots identical: {shots:#?}");
}

#[test]
fn frozen_blueprint_from_another_file() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#steady_other_file");
    let turret = spawn(&mut app, "turrets.bp.ron#steady_other_file");
    let speeds: Vec<f32> = (0..5)
        .map(|_| {
            let bullet = fire(&mut app, turret);
            assert_eq!(get::<Name>(&app, bullet).as_str(), "Turret");
            get::<Speed>(&app, bullet).0
        })
        .collect();
    assert!((1.0..=2.0).contains(&speeds[0]));
    assert!(speeds.iter().all(|s| *s == speeds[0]), "{speeds:?}");
}

#[test]
fn frozen_scene_is_freed_with_its_owner() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#steady_turret");
    let turret = spawn(&mut app, "turrets.bp.ron#steady_turret");

    // The turret holds its own frozen scene, not the file's `#bullet` blueprint.
    let frozen = get::<Ammo>(&app, turret).bullet;
    assert!(
        frozen.path().is_none(),
        "expected a frozen copy, got {:?}",
        frozen.path()
    );
    let frozen_id = frozen.id();
    drop(frozen);
    assert!(app.world().resource::<Assets<ScenePatch>>().contains(frozen_id));

    app.world_mut().despawn(turret);
    for _ in 0..10 {
        app.update();
    }
    assert!(
        !app.world().resource::<Assets<ScenePatch>>().contains(frozen_id),
        "frozen scene outlived its turret"
    );
}

#[test]
fn inherited_same_file_reference_from_another_file() {
    let mut app = app();
    let _h = load(&mut app, "armory.bp.ron#gunship");
    let gunship = spawn(&mut app, "armory.bp.ron#gunship");
    let bullet = fire(&mut app, gunship);
    assert!(
        child_named(&app, bullet, "Trail").is_some(),
        "the bullet from turrets.bp.ron"
    );
}

#[test]
fn blueprint_references_in_a_list() {
    let mut app = app();
    let _h = load(&mut app, "armory.bp.ron#arsenal");
    let arsenal = spawn(&mut app, "armory.bp.ron#arsenal");
    let bullets = get::<Arsenal>(&app, arsenal).bullets;
    assert_eq!(bullets.len(), 4);
    let names: Vec<String> = bullets
        .into_iter()
        .map(|scene| {
            let e = spawn_from(&mut app, scene);
            get::<Name>(&app, e).as_str().to_string()
        })
        .collect();
    assert_eq!(names, ["Bullet", "Turret", "Shell", "Shell"]);

    // The frozen entry fires identical shells; the plain one doesn't.
    let arsenal = get::<Arsenal>(&app, arsenal).bullets;
    let speeds = |app: &mut App, scene: &Handle<ScenePatch>| -> Vec<f32> {
        (0..5)
            .map(|_| {
                let e = spawn_from(app, scene.clone());
                get::<Speed>(app, e).0
            })
            .collect()
    };
    let frozen = speeds(&mut app, &arsenal[3]);
    assert!(frozen.iter().all(|s| *s == frozen[0]), "{frozen:?}");
    let plain = speeds(&mut app, &arsenal[2]);
    assert!(plain.iter().any(|s| *s != plain[0]), "{plain:?}");
}
