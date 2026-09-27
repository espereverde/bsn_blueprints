//! Recipes: parameters expanded into bundles by Rust code.

use crate::support::*;

#[test]
fn recipe_expands_and_explicit_components_override_it() {
    let mut app = app();
    let _h = load(&mut app, "rocks.bp.ron#recipe_rock");
    let e = spawn(&mut app, "rocks.bp.ron#recipe_rock");
    assert_eq!(get::<Stats>(&app, e), Stats { hp: 30, armor: 3 });
    assert!((get::<Speed>(&app, e).0 - 10.0 / 3.0).abs() < 1e-6);
    assert_eq!(
        get::<Points>(&app, e),
        Points(99),
        "explicit component overrides the recipe"
    );
    assert!(
        child_named(&app, e, "Shard").is_some(),
        "recipe bundles can spawn children"
    );
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
