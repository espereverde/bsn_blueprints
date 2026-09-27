//! `Range`, `OneOf`, `Maybe`, `Fixed` on values, components and sets.

use crate::support::*;

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
    assert!(
        tints.iter().all(|t| [Tint::Gold, Tint::Custom(3)].contains(t)),
        "{tints:?}"
    );
    assert!(
        tints.contains(&Tint::Gold) && tints.contains(&Tint::Custom(3)),
        "{tints:?}"
    );
    assert!(
        facings.contains(&Facing::Left) && facings.contains(&Facing::Right),
        "{facings:?}"
    );
    // `(y: 9.0)` merges onto the inherited offset (x: 1.0, y: 5.0).
    let corners = [Offset { x: 0.0, y: 0.0 }, Offset { x: 1.0, y: 9.0 }];
    assert!(offsets.iter().all(|o| corners.contains(o)), "{offsets:?}");
    assert!(
        offsets.iter().any(|o| *o != offsets[0]),
        "all offsets equal: {offsets:?}"
    );
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
    assert!(
        names.iter().any(|n| n == "Ann") && names.iter().any(|n| n == "Bob"),
        "{names:?}"
    );
    let options = [Stats { hp: 1, armor: 1 }, Stats { hp: 2, armor: 2 }];
    assert!(stats.iter().all(|s| options.contains(s)), "{stats:?}");
    assert!(options.iter().all(|o| stats.contains(o)), "{stats:?}");
    assert!((60..=140).contains(&speeds), "speeds {speeds}");
}

#[test]
fn wrapper_names_as_plain_values() {
    let mut app = app();
    let _h = load(&mut app, "variants.bp.ron#mode");
    let e = spawn(&mut app, "variants.bp.ron#mode");
    assert_eq!(get::<Mode>(&app, e), Mode::OneOf);
    assert_eq!(get::<Tint>(&app, e), Tint::Gold);
}
