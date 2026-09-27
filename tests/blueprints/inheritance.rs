//! Components, inheritance, and Bevy integration (`bsn!`, required components).

use crate::support::*;

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
        Placement {
            offset: Offset { x: 1.0, y: 5.0 },
            layer: 3,
            facing: Facing::Left
        }
    );
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
fn required_components_are_added() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#shielded_rock");
    let e = spawn(&mut app, "rocks.bp.ron#shielded_rock");
    assert!(app.world().get::<Marker>(e).is_some());
    // Required components never override values the blueprint sets.
    assert_eq!(get::<Speed>(&app, e).0, 3.0);
}
