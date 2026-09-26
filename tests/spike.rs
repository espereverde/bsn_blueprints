use std::time::Duration;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, ReflectAsset};
use bevy::prelude::*;
use bevy::scene::{CachedSceneAsset, ScenePatch, ScenePatchInstance, ScenePlugin, WorldSceneExt, bsn};
use bsn_blueprints::{BlueprintPlugin, Recipe, ReflectRecipe};
use serde::Deserialize;

// ---- "Game" types: plain reflected components, no blueprint-specific code ----

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
struct Stats {
    hp: u32,
    armor: u32,
}

#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
struct Speed(f32);

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
enum Tint {
    #[default]
    Grey,
    Gold,
    Custom(u8),
}

/// No `#[reflect(Default)]`: must be written completely the first time.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component)]
struct Points(u32);

/// No `#[reflect(Default)]` either, with named fields.
#[derive(Component, Reflect, Clone, Debug)]
#[reflect(Component)]
struct Bounty {
    gold: u32,
    gems: u32,
}

#[derive(Component, Reflect, Default, Clone, Debug)]
#[reflect(Component, Default)]
struct Skin(Handle<Palette>);

#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
struct Placement {
    offset: Offset,
    layer: u8,
    facing: Facing,
}

#[derive(Reflect, Default, Clone, Copy, Debug, PartialEq)]
#[reflect(Default)]
enum Facing {
    #[default]
    Left,
    Right,
}

#[derive(Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Default)]
struct Offset {
    x: f32,
    y: f32,
}

#[derive(Component, Default, Clone)]
struct Marker;

/// A recipe: `size` expands to several components, and a child entity.
#[derive(Reflect, Default, Clone)]
#[reflect(Recipe, Default)]
struct RockRecipe {
    size: u32,
}

impl Recipe for RockRecipe {
    fn bundle(&self) -> impl Bundle + use<> {
        (
            Stats { hp: self.size * 10, armor: self.size },
            Speed(10.0 / self.size as f32),
            Points(self.size * 5),
            children![Name::new("Shard")],
        )
    }
}

/// An enum with a variant named like a wrapper.
#[derive(Component, Reflect, Default, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
enum Mode {
    #[default]
    Other,
    OneOf,
}

/// Bevy's required components must still be added when a blueprint inserts this.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
#[require(Marker, Speed(7.0))]
struct Shielded;

/// Several kinds of ammunition.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Arsenal {
    bullets: Vec<Handle<ScenePatch>>,
}

/// A gun's ammunition: the blueprint of the bullets it fires.
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Ammo {
    bullet: Handle<ScenePatch>,
}

#[derive(Asset, Reflect, Deserialize)]
#[reflect(Asset)]
struct Palette {
    colors: Vec<String>,
}

#[derive(TypePath, Default)]
struct PaletteLoader;

impl AssetLoader for PaletteLoader {
    type Asset = Palette;
    type Settings = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _: &(),
        _: &mut LoadContext<'_>,
    ) -> Result<Palette, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(ron::de::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &["palette.ron"]
    }
}

// ---- Harness ----

fn app() -> App {
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
fn load(app: &mut App, path: &'static str) -> Handle<ScenePatch> {
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

/// Loads a blueprint that is expected to fail and returns the error text.
fn load_error(app: &mut App, path: &'static str) -> String {
    let handle: Handle<ScenePatch> = app.world().resource::<AssetServer>().load(path);
    for _ in 0..1000 {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(state) = server.get_recursive_dependency_load_state(&handle)
            && state.is_failed()
        {
            return format!("{state:?}");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("{path} was expected to fail");
}

fn spawn(app: &mut App, path: &'static str) -> Entity {
    app.world_mut()
        .spawn_scene(CachedSceneAsset::from(path))
        .expect("spawn")
        .id()
}

fn get<C: Component + Clone>(app: &App, entity: Entity) -> C {
    app.world()
        .get::<C>(entity)
        .unwrap_or_else(|| panic!("missing {}", std::any::type_name::<C>()))
        .clone()
}

// ---- Tests ----

#[test]
fn same_file_inheritance_with_field_override() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#big_rock");
    let e = spawn(&mut app, "rocks.bp.ron#big_rock");

    assert_eq!(get::<Stats>(&app, e), Stats { hp: 40, armor: 2 });
    assert_eq!(get::<Points>(&app, e), Points(20));
    assert_eq!(get::<Tint>(&app, e), Tint::Grey);
    assert_eq!(get::<Name>(&app, e).as_str(), "Rock");
    assert_eq!(
        get::<Placement>(&app, e),
        Placement { offset: Offset { x: 1.0, y: 5.0 }, layer: 3, facing: Facing::Left }
    );
}

#[test]
fn random_values_are_sampled_per_spawn() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#lucky_rock");
    let _h2 = load(&mut app, "rocks.bp.ron#big_rock");

    // Inline Range, inherited by big_rock.
    let speeds: Vec<f32> = (0..50)
        .map(|_| {
            let e = spawn(&mut app, "rocks.bp.ron#big_rock");
            get::<Speed>(&app, e).0
        })
        .collect();
    assert!(speeds.iter().all(|s| (0.5..=2.0).contains(s)), "{speeds:?}");
    assert!(speeds.iter().any(|s| *s != speeds[0]), "all speeds equal: {speeds:?}");

    // Number and enum OneOf, an integer Range, an overridden Range, and a OneOf of partial structs.
    let (mut armors, mut points, mut tints) = (Vec::new(), Vec::new(), Vec::new());
    let (mut facings, mut offsets) = (Vec::new(), Vec::new());
    for _ in 0..50 {
        let e = spawn(&mut app, "rocks.bp.ron#lucky_rock");
        let stats = get::<Stats>(&app, e);
        assert_eq!(stats.hp, 40);
        assert!((10.0..=11.0).contains(&get::<Speed>(&app, e).0));
        armors.push(stats.armor);
        points.push(get::<Points>(&app, e).0);
        tints.push(get::<Tint>(&app, e));
        let placement = get::<Placement>(&app, e);
        assert_eq!(placement.layer, 3);
        facings.push(placement.facing);
        offsets.push(placement.offset);
    }
    assert!(armors.iter().all(|a| [1, 3, 7].contains(a)), "{armors:?}");
    assert!(armors.iter().any(|a| *a != armors[0]), "all armors equal: {armors:?}");
    assert!(points.iter().all(|p| (100..=200).contains(p)), "{points:?}");
    assert!(points.iter().any(|p| *p != points[0]), "all points equal: {points:?}");
    assert!(tints.iter().all(|t| [Tint::Gold, Tint::Custom(3)].contains(t)), "{tints:?}");
    assert!(tints.contains(&Tint::Gold) && tints.contains(&Tint::Custom(3)), "{tints:?}");
    assert!(facings.contains(&Facing::Left) && facings.contains(&Facing::Right), "{facings:?}");
    // `(y: 9.0)` merges onto the inherited offset (x: 1.0, y: 5.0).
    let corners = [Offset { x: 0.0, y: 0.0 }, Offset { x: 1.0, y: 9.0 }];
    assert!(offsets.iter().all(|o| corners.contains(o)), "{offsets:?}");
    assert!(offsets.iter().any(|o| *o != offsets[0]), "all offsets equal: {offsets:?}");
}

#[test]
fn fixed_value_cancels_inherited_random() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#steady_rock");
    for _ in 0..20 {
        let e = spawn(&mut app, "rocks.bp.ron#steady_rock");
        assert_eq!(get::<Speed>(&app, e).0, 3.0);
    }
}

#[test]
fn cross_file_inheritance_and_handles() {
    let mut app = app();
    let _h = load(&mut app, "ufos.bp.ron#boss");
    let e = spawn(&mut app, "ufos.bp.ron#boss");

    assert_eq!(get::<Stats>(&app, e), Stats { hp: 40, armor: 9 });
    assert_eq!(get::<Points>(&app, e), Points(20));
    assert_eq!(get::<Name>(&app, e).as_str(), "Boss");
    assert_eq!(get::<Tint>(&app, e), Tint::Custom(9));

    let skin = get::<Skin>(&app, e);
    assert_eq!(skin.0.path().map(|p| p.to_string()).as_deref(), Some("red.palette.ron"));
    // The blueprint does not wait for assets its components reference (like a sprite's image),
    // so wait for the palette here.
    for _ in 0..1000 {
        if app.world().resource::<Assets<Palette>>().contains(&skin.0) {
            break;
        }
        app.update();
        std::thread::sleep(Duration::from_millis(2));
    }
    let palettes = app.world().resource::<Assets<Palette>>();
    assert_eq!(palettes.get(&skin.0).expect("palette loaded").colors.len(), 2);
}

#[test]
fn bsn_can_inherit_a_data_blueprint() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#big_rock");

    let e = app
        .world_mut()
        .spawn_scene(bsn! { :"rocks.bp.ron#big_rock" Marker })
        .expect("spawn")
        .id();
    assert!(app.world().get::<Marker>(e).is_some());
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 40, armor: 2 });
}

/// Known limitation: `bsn!` field patches start from `Default`, not from the data blueprint,
/// because reflected components are not stored as patchable templates. The bsn! value wins
/// as a whole component.
#[test]
fn bsn_field_patch_replaces_whole_data_component() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#big_rock");

    let e = app
        .world_mut()
        .spawn_scene(bsn! { :"rocks.bp.ron#big_rock" Stats { armor: 7 } })
        .expect("spawn")
        .id();
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 0, armor: 7 });
}

#[test]
fn inheritance_cycle_fails_to_load() {
    let mut app = app();
    let error = load_error(&mut app, "cycle.bp.ron#a");
    assert!(error.contains("cycle"), "{error}");
}

#[test]
fn incomplete_component_without_default_fails_to_load() {
    let mut app = app();
    let error = load_error(&mut app, "incomplete.bp.ron#bad");
    assert!(error.contains("incomplete"), "{error}");
}

// ---- Hierarchies and optional parts ----

fn children(app: &App, entity: Entity) -> Vec<Entity> {
    app.world()
        .get::<Children>(entity)
        .map(|c| c.iter().collect())
        .unwrap_or_default()
}

fn child_named(app: &App, entity: Entity, name: &str) -> Option<Entity> {
    children(app, entity)
        .into_iter()
        .find(|c| app.world().get::<Name>(*c).is_some_and(|n| n.as_str() == name))
}

#[test]
fn children_are_spawned_with_their_own_children() {
    let mut app = app();
    let _h = load(&mut app, "saucers.bp.ron#saucer");
    let e = spawn(&mut app, "saucers.bp.ron#saucer");

    let turret = child_named(&app, e, "Turret").expect("turret child");
    assert_eq!(get::<Stats>(&app, turret), Stats { hp: 3, armor: 4 });
    assert!((1.0..=2.0).contains(&get::<Speed>(&app, turret).0));
    assert_eq!(app.world().get::<ChildOf>(turret).map(|c| c.parent()), Some(e));

    let light = child_named(&app, e, "Light").expect("light child");
    assert!(child_named(&app, light, "Bulb").is_some());
}

#[test]
fn derived_blueprint_patches_inherited_child() {
    let mut app = app();
    let _h = load(&mut app, "saucers.bp.ron#big_saucer");
    let e = spawn(&mut app, "saucers.bp.ron#big_saucer");

    assert_eq!(get::<Stats>(&app, e), Stats { hp: 50, armor: 0 });
    let turret = child_named(&app, e, "Turret").expect("turret child");
    assert_eq!(get::<Stats>(&app, turret), Stats { hp: 30, armor: 4 });
    assert!(child_named(&app, e, "Light").is_some());
}

#[test]
fn one_of_and_maybe_are_chosen_per_spawn() {
    let mut app = app();
    let _h = load(&mut app, "saucers.bp.ron#saucer");

    let (mut gold, mut custom, mut bounties) = (0, 0, 0);
    for _ in 0..400 {
        let e = spawn(&mut app, "saucers.bp.ron#saucer");
        let has_pilot = child_named(&app, e, "Pilot").is_some();
        match get::<Tint>(&app, e) {
            Tint::Gold => {
                gold += 1;
                assert!(!has_pilot, "Gold option has no pilot");
            }
            Tint::Custom(1) => {
                custom += 1;
                assert!(has_pilot, "Custom option adds a pilot child");
            }
            other => panic!("unexpected tint {other:?}"),
        }
        // Fixed children are there whatever option was chosen.
        assert!(child_named(&app, e, "Turret").is_some());
        if app.world().get::<Bounty>(e).is_some() {
            bounties += 1;
        }
    }
    // Weights 3:1 and chance 0.5, with generous margins.
    assert!((240..=360).contains(&gold), "gold {gold}, custom {custom}");
    assert_eq!(gold + custom, 400);
    assert!((140..=260).contains(&bounties), "bounties {bounties}");
}

#[test]
fn child_extending_its_ancestor_is_a_cycle() {
    let mut app = app();
    let error = load_error(&mut app, "child_cycle.bp.ron#loop");
    assert!(error.contains("cycle"), "{error}");
}

#[test]
fn maybe_chance_out_of_range_fails_to_load() {
    let mut app = app();
    let error = load_error(&mut app, "bad_chance.bp.ron#x");
    assert!(error.contains("between 0 and 1"), "{error}");
}

#[test]
fn one_of_options_can_extend_blueprints() {
    let mut app = app();
    let _h = load(&mut app, "saucers.bp.ron#mystery");

    let (mut turrets, mut rocks) = (0, 0);
    for _ in 0..100 {
        let e = spawn(&mut app, "saucers.bp.ron#mystery");
        match get::<Name>(&app, e).as_str() {
            "Turret" => {
                turrets += 1;
                // The turret blueprint has no Points, so the fixed Points(1) stays.
                assert_eq!(get::<Points>(&app, e), Points(1));
                assert_eq!(get::<Stats>(&app, e), Stats { hp: 3, armor: 1 });
                assert!((1.0..=2.0).contains(&get::<Speed>(&app, e).0));
            }
            "Rock" => {
                rocks += 1;
                assert_eq!(get::<Stats>(&app, e), Stats { hp: 10, armor: 9 });
                assert!((0.5..=2.0).contains(&get::<Speed>(&app, e).0));
                // The rock blueprint sets Points(5), applied after the fixed Points(1).
                assert_eq!(get::<Points>(&app, e), Points(5));
            }
            other => panic!("unexpected name {other}"),
        }
    }
    assert!(turrets > 0 && rocks > 0, "turrets {turrets}, rocks {rocks}");
}

#[test]
fn one_of_and_maybe_inline_in_components() {
    let mut app = app();
    let _h = load(&mut app, "saucers.bp.ron#inline_parts");

    let (mut stats, mut gold, mut bounties) = (0, 0, 0);
    let mut gems = Vec::new();
    for _ in 0..400 {
        let e = spawn(&mut app, "saucers.bp.ron#inline_parts");
        let world = app.world();
        assert_eq!(get::<Name>(&app, e).as_str(), "Inline");

        // Exactly one of Stats / Speed.
        let (has_stats, has_speed) = (world.get::<Stats>(e).is_some(), world.get::<Speed>(e).is_some());
        assert!(has_stats != has_speed, "stats {has_stats}, speed {has_speed}");
        if has_stats {
            stats += 1;
            assert_eq!(get::<Stats>(&app, e), Stats { hp: 50, armor: 0 });
        }

        // Gold alone, or Grey together with Points(7).
        match get::<Tint>(&app, e) {
            Tint::Gold => {
                gold += 1;
                assert!(world.get::<Points>(e).is_none());
            }
            Tint::Grey => assert_eq!(get::<Points>(&app, e), Points(7)),
            other => panic!("unexpected tint {other:?}"),
        }

        if let Some(bounty) = world.get::<Bounty>(e) {
            bounties += 1;
            assert_eq!(bounty.gold, 1);
            gems.push(bounty.gems);
        }
    }
    assert!((140..=260).contains(&stats), "stats {stats}");
    assert!((240..=360).contains(&gold), "gold {gold}");
    assert!((140..=260).contains(&bounties), "bounties {bounties}");
    assert!(gems.contains(&1) && gems.contains(&2), "{gems:?}");
}

// ---- Blueprint references in components ----

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
        let shots: Vec<String> = (0..5).map(|_| {
            let bullet = fire(&mut app, turret);
            fingerprint(&app, bullet)
        }).collect();
        assert!(shots.iter().all(|s| *s == shots[0]), "one turret, different bullets: {shots:#?}");
        per_turret.push(shots[0].clone());
    }
    assert_ne!(per_turret[0], per_turret[1], "two turrets fired identical bullets");
}

#[test]
fn plain_blueprint_handle_varies_per_shot() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#varied_turret");
    let turret = spawn(&mut app, "turrets.bp.ron#varied_turret");
    let shots: Vec<String> = (0..5).map(|_| {
        let bullet = fire(&mut app, turret);
        fingerprint(&app, bullet)
    }).collect();
    assert!(shots.iter().any(|s| *s != shots[0]), "all shots identical: {shots:#?}");
}

#[test]
fn frozen_blueprint_from_another_file() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#steady_other_file");
    let turret = spawn(&mut app, "turrets.bp.ron#steady_other_file");
    let speeds: Vec<f32> = (0..5).map(|_| {
        let bullet = fire(&mut app, turret);
        assert_eq!(get::<Name>(&app, bullet).as_str(), "Turret");
        get::<Speed>(&app, bullet).0
    }).collect();
    assert!((1.0..=2.0).contains(&speeds[0]));
    assert!(speeds.iter().all(|s| *s == speeds[0]), "{speeds:?}");
}

#[test]
fn frozen_blueprint_cycle_fails_to_load() {
    let mut app = app();
    let error = load_error(&mut app, "frozen_cycle.bp.ron#gun");
    assert!(error.contains("cycle"), "{error}");
}


#[test]
fn frozen_scene_is_freed_with_its_owner() {
    let mut app = app();
    let _h = load(&mut app, "turrets.bp.ron#steady_turret");
    let turret = spawn(&mut app, "turrets.bp.ron#steady_turret");

    // The turret holds its own frozen scene, not the file's `#bullet` blueprint.
    let frozen = get::<Ammo>(&app, turret).bullet;
    assert!(frozen.path().is_none(), "expected a frozen copy, got {:?}", frozen.path());
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
fn required_components_are_added() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#shielded_rock");
    let e = spawn(&mut app, "rocks.bp.ron#shielded_rock");
    assert!(app.world().get::<Marker>(e).is_some());
    // Required components never override values the blueprint sets.
    assert_eq!(get::<Speed>(&app, e).0, 3.0);
}

// ---- Recipes ----

#[test]
fn recipe_expands_and_explicit_components_override_it() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#recipe_rock");
    let e = spawn(&mut app, "rocks.bp.ron#recipe_rock");
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 30, armor: 3 });
    assert!((get::<Speed>(&app, e).0 - 10.0 / 3.0).abs() < 1e-6);
    assert_eq!(get::<Points>(&app, e), Points(99), "explicit component overrides the recipe");
    assert!(child_named(&app, e, "Shard").is_some(), "recipe bundles can spawn children");
}

#[test]
fn recipe_parameters_inherit_and_can_be_random() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#big_recipe_rock");
    let _h2 = load(&mut app, "rocks.bp.ron#random_recipe_rock");

    let e = spawn(&mut app, "rocks.bp.ron#big_recipe_rock");
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 100, armor: 10 });
    assert_eq!(get::<Points>(&app, e), Points(99), "inherited explicit override");

    let mut sizes = Vec::new();
    for _ in 0..50 {
        let e = spawn(&mut app, "rocks.bp.ron#random_recipe_rock");
        let stats = get::<Stats>(&app, e);
        // Derived values are consistent: one random size per spawn feeds all of them.
        assert_eq!(stats.hp, stats.armor * 10);
        assert_eq!(get::<Points>(&app, e), Points(stats.armor * 5));
        sizes.push(stats.armor);
    }
    assert!(sizes.iter().all(|s| (1..=4).contains(s)), "{sizes:?}");
    assert!(sizes.iter().any(|s| *s != sizes[0]), "{sizes:?}");
}

#[test]
fn recipe_as_a_one_of_option() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#maybe_recipe_rock");
    let (mut recipe, mut speed) = (0, 0);
    for _ in 0..100 {
        let e = spawn(&mut app, "rocks.bp.ron#maybe_recipe_rock");
        if app.world().get::<Stats>(e).is_some() {
            recipe += 1;
            assert_eq!(get::<Stats>(&app, e), Stats { hp: 20, armor: 2 });
        } else {
            speed += 1;
            assert_eq!(get::<Speed>(&app, e).0, 1.0);
        }
    }
    assert!(recipe > 0 && speed > 0, "recipe {recipe}, speed {speed}");
}

// ---- OneOf / Maybe / Fixed anywhere ----

#[test]
fn chosen_sets_merge_onto_base_values() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#merged");
    let e = spawn(&mut app, "variants.bp.ron#merged");
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 50, armor: 5 });
}

#[test]
fn one_of_whole_values_and_optional_components() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#optional");
    let (mut names, mut stats, mut speeds) = (Vec::new(), Vec::new(), 0);
    for _ in 0..200 {
        let e = spawn(&mut app, "variants.bp.ron#optional");
        names.push(get::<Name>(&app, e).as_str().to_string());
        stats.push(get::<Stats>(&app, e));
        if let Some(speed) = app.world().get::<Speed>(e) {
            assert_eq!(speed.0, 1.0);
            speeds += 1;
        }
    }
    assert!(names.iter().any(|n| n == "Ann") && names.iter().any(|n| n == "Bob"), "{names:?}");
    let options = [Stats { hp: 1, armor: 1 }, Stats { hp: 2, armor: 2 }];
    assert!(stats.iter().all(|s| options.contains(s)), "{stats:?}");
    assert!(options.iter().all(|o| stats.contains(o)), "{stats:?}");
    assert!((60..=140).contains(&speeds), "speeds {speeds}");
}

#[test]
fn random_children() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#crew");
    let (mut pilots, mut a, mut b) = (0, 0, 0);
    for _ in 0..200 {
        let e = spawn(&mut app, "variants.bp.ron#crew");
        if child_named(&app, e, "Pilot").is_some() {
            pilots += 1;
        }
        let (has_a, has_b) = (child_named(&app, e, "GunnerA").is_some(), child_named(&app, e, "GunnerB").is_some());
        assert!(has_a != has_b, "exactly one gunner");
        if has_a { a += 1 } else { b += 1 }
    }
    assert!((60..=140).contains(&pilots), "pilots {pilots}");
    assert!(a > 0 && b > 0, "gunners {a} / {b}");
}

#[test]
fn wrapper_names_as_plain_values() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#mode");
    let e = spawn(&mut app, "variants.bp.ron#mode");
    assert_eq!(get::<Mode>(&app, e), Mode::OneOf);
    assert_eq!(get::<Tint>(&app, e), Tint::Gold);
}

#[test]
fn maybe_on_a_field_is_an_error() {
    let mut app = app();
    let error = load_error(&mut app, "bad_syntax.bp.ron#field_maybe");
    assert!(error.contains("Maybe(..) only works on components"), "{error}");
    assert!(error.contains("2:"), "error has a line number: {error}");
}

#[test]
fn a_blueprint_cannot_be_maybe() {
    let mut app = app();
    let error = load_error(&mut app, "bad_top_maybe.bp.ron#sometimes");
    assert!(error.contains("only children can be Maybe"), "{error}");
}

#[test]
fn wrapped_child_redeclaration_replaces_inherited_randomness() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#fleet_1");
    let _h4 = load(&mut app, "variants.bp.ron#fleet_4");

    let (mut lights, mut lamps, mut radars) = (0, 0, 0);
    for _ in 0..400 {
        let e = spawn(&mut app, "variants.bp.ron#fleet_1");
        if let Some(light) = child_named(&app, e, "Light") {
            lights += 1;
            assert_eq!(get::<Stats>(&app, light), Stats { hp: 1, armor: 9 }, "inherited values kept");
        }
        if let Some(lamp) = child_named(&app, e, "Lamp") {
            lamps += 1;
            assert_eq!(get::<Stats>(&app, lamp), Stats { hp: 2, armor: 2 });
        }
        assert!(child_named(&app, e, "RadarA").is_none() && child_named(&app, e, "RadarB").is_none());
        if child_named(&app, e, "RadarC").is_some() {
            radars += 1;
        }
    }
    for (what, count) in [("light", lights), ("lamp", lamps), ("radar", radars)] {
        assert!((140..=260).contains(&count), "{what}: {count} of 400, expected about 200");
    }

    // Three more generations each redeclare the light as Maybe(0.5, ..): still about half.
    let deep = (0..400)
        .filter(|_| {
            let e = spawn(&mut app, "variants.bp.ron#fleet_4");
            child_named(&app, e, "Light").is_some()
        })
        .count();
    assert!((140..=260).contains(&deep), "light after 4 generations: {deep} of 400");
}

#[test]
fn inherited_same_file_reference_from_another_file() {
    let mut app = app();
    let _h = load(&mut app, "armory.bp.ron#gunship");
    let gunship = spawn(&mut app, "armory.bp.ron#gunship");
    let bullet = fire(&mut app, gunship);
    assert!(child_named(&app, bullet, "Trail").is_some(), "the bullet from turrets.bp.ron");
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
        (0..5).map(|_| {
            let e = spawn_from(app, scene.clone());
            get::<Speed>(app, e).0
        }).collect()
    };
    let frozen = speeds(&mut app, &arsenal[3]);
    assert!(frozen.iter().all(|s| *s == frozen[0]), "{frozen:?}");
    let plain = speeds(&mut app, &arsenal[2]);
    assert!(plain.iter().any(|s| *s != plain[0]), "{plain:?}");
}

