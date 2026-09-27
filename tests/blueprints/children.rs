//! Child entities: nesting, patching inherited children, random children.

use crate::support::*;

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
fn random_children() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#crew");
    let (mut pilots, mut a, mut b) = (0, 0, 0);
    for _ in 0..200 {
        let e = spawn(&mut app, "variants.bp.ron#crew");
        if child_named(&app, e, "Pilot").is_some() {
            pilots += 1;
        }
        let (has_a, has_b) = (
            child_named(&app, e, "GunnerA").is_some(),
            child_named(&app, e, "GunnerB").is_some(),
        );
        assert!(has_a != has_b, "exactly one gunner");
        if has_a { a += 1 } else { b += 1 }
    }
    assert!((60..=140).contains(&pilots), "pilots {pilots}");
    assert!(a > 0 && b > 0, "gunners {a} / {b}");
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
            assert_eq!(
                get::<Stats>(&app, light),
                Stats { hp: 1, armor: 9 },
                "inherited values kept"
            );
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
        assert!(
            (140..=260).contains(&count),
            "{what}: {count} of 400, expected about 200"
        );
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
