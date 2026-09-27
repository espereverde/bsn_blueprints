//! Spawn cost: plain Rust vs `bsn!` vs data blueprints. Run with `cargo run --release --example bench`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::*;
use bevy::scene::{ResolvedSceneRoot, ScenePatch, ScenePlugin, template_value};
use bsn_blueprints::BlueprintPlugin;
use rand::Rng;

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Stats {
    hp: u32,
    armor: u32,
}

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Speed(f32);

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
enum Tint {
    #[default]
    Grey,
    Gold,
}

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Points(u32);

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Bounty {
    gold: u32,
    gems: u32,
}

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Ammo {
    bullet: Handle<ScenePatch>,
}

const N: usize = 20_000;

fn main() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin, BlueprintPlugin))
        .register_type::<Name>()
        .register_type::<Stats>()
        .register_type::<Speed>()
        .register_type::<Tint>()
        .register_type::<Points>()
        .register_type::<Bounty>()
        .register_type::<Ammo>();

    let handles: Vec<Handle<ScenePatch>> = ["plain", "random", "full", "gun"]
        .iter()
        .map(|label| {
            app.world()
                .resource::<AssetServer>()
                .load(format!("bench.bp.ron#{label}"))
        })
        .collect();
    while !handles.iter().all(|h| {
        app.world()
            .resource::<Assets<ScenePatch>>()
            .get(h)
            .is_some_and(|p| p.resolved.is_some())
    }) {
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    let resolved = |app: &App, h: &Handle<ScenePatch>| -> Arc<ResolvedSceneRoot> {
        app.world()
            .resource::<Assets<ScenePatch>>()
            .get(h)
            .unwrap()
            .resolved
            .clone()
            .unwrap()
    };
    let [plain, random, full, gun] = [0, 1, 2, 3].map(|i| resolved(&app, &handles[i]));

    // The frozen bullet: spawn the gun once and take its ammo.
    let gun_entity = gun.spawn(app.world_mut()).unwrap().id();
    let frozen_handle = app.world().get::<Ammo>(gun_entity).unwrap().bullet.clone();
    let frozen = resolved(&app, &frozen_handle);

    let world = app.world_mut();
    println!("{N} spawns each (release build), best of 5 runs\n");
    bench(world, "Rust: world.spawn((5 components))", |world| {
        world.spawn((
            Name::new("Bullet"),
            Speed(500.0),
            Stats { hp: 10, armor: 2 },
            Tint::Gold,
            Points(3),
        ));
    });
    bench(world, "Rust: same with 3 random values", |world| {
        let mut rng = rand::rng();
        let tint = if rng.random_bool(0.5) { Tint::Gold } else { Tint::Grey };
        world.spawn((
            Name::new("Bullet"),
            Speed(rng.random_range(1.0..=1000.0)),
            Stats {
                hp: rng.random_range(1..=100),
                armor: 2,
            },
            tint,
            Points(3),
        ));
    });
    bench(world, "Rust: spawn_empty + 5 separate inserts", |world| {
        let mut e = world.spawn_empty();
        e.insert(Name::new("Bullet"));
        e.insert(Speed(500.0));
        e.insert(Stats { hp: 10, armor: 2 });
        e.insert(Tint::Gold);
        e.insert(Points(3));
    });
    let registry = world.resource::<AppTypeRegistry>().clone();
    let values: Vec<Box<dyn Reflect>> = vec![
        Box::new(Name::new("Bullet")),
        Box::new(Speed(500.0)),
        Box::new(Stats { hp: 10, armor: 2 }),
        Box::new(Tint::Gold),
        Box::new(Points(3)),
    ];
    bench(world, "Reflect: 5 x ReflectComponent::insert", |world| {
        let registry = registry.read();
        let mut e = world.spawn_empty();
        for value in &values {
            let rc = registry.get_type_data::<ReflectComponent>((**value).type_id()).unwrap();
            rc.insert(&mut e, value.as_partial_reflect(), &registry);
        }
    });
    bench(world, "Reflect: same + reflect_clone each", |world| {
        let registry = registry.read();
        let mut e = world.spawn_empty();
        for value in &values {
            let rc = registry.get_type_data::<ReflectComponent>((**value).type_id()).unwrap();
            let copy = value.reflect_clone().unwrap();
            rc.insert(&mut e, copy.as_partial_reflect(), &registry);
        }
    });
    let bsn_scene = Arc::new(
        ResolvedSceneRoot::resolve(
            Box::new((
                template_value(Name::new("Bullet")),
                template_value(Speed(500.0)),
                template_value(Stats { hp: 10, armor: 2 }),
                template_value(Tint::Gold),
                template_value(Points(3)),
            )),
            world.resource::<AssetServer>(),
            world.resource::<Assets<ScenePatch>>(),
        )
        .unwrap(),
    );
    bench(world, "Bevy scene (bsn!-style): same 5 components", |world| {
        bsn_scene.spawn(world).unwrap();
    });
    bench(world, "blueprint: 5 fixed components", |world| {
        plain.spawn(world).unwrap();
    });
    bench(world, "blueprint: 5 components, 3 random", |world| {
        random.spawn(world).unwrap();
    });
    bench(world, "blueprint: random + one_of + maybe + child", |world| {
        full.spawn(world).unwrap();
    });
    bench(world, "blueprint: frozen copy of the above", |world| {
        frozen.spawn(world).unwrap();
    });
}

fn bench(world: &mut World, name: &str, mut spawn: impl FnMut(&mut World)) {
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        for _ in 0..N {
            spawn(black_box(&mut *world));
        }
        best = best.min(start.elapsed());
        // Despawn everything spawned, keeping the benchmark world small.
        let spawned: Vec<Entity> = world
            .query_filtered::<Entity, With<Name>>()
            .iter(world)
            .filter(|e| world.get::<Ammo>(*e).is_none())
            .collect();
        for entity in spawned {
            world.despawn(entity);
        }
    }
    let per_spawn = best.as_nanos() as f64 / N as f64;
    println!(
        "{name:48} {:>8.2} ms  {:>7.0} ns/spawn",
        best.as_secs_f64() * 1000.0,
        per_spawn
    );
}
