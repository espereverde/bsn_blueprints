//! Load time of large blueprint files. Run with `cargo run --release --example load_bench`.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::scene::{ScenePatch, ScenePlugin};
use bsn_blueprints::{BlueprintFile, BlueprintPlugin};

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
    Custom(u8),
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
struct Placement {
    offset: Offset,
    layer: u8,
}

#[derive(Reflect, Default, Clone)]
#[reflect(Default)]
struct Offset {
    x: f32,
    y: f32,
}

/// One blueprint: inheritance, random values, random sets, and fixed and random children.
fn blueprint(i: usize) -> String {
    let extends = if i.is_multiple_of(10) {
        String::new()
    } else {
        format!("extends: \"#bp_{}\",", i - 1)
    };
    format!(
        r#"
    "bp_{i}": (
        {extends}
        components: [
            {{
                "Name": "Unit {i}",
                "Stats": (hp: Range(1, 100), armor: {armor}),
                "Speed": ({speed}.5),
                "Tint": OneOf([Gold, Grey, Custom({custom})]),
                "Placement": (offset: (x: {i}.0, y: 2.0), layer: 3),
            }},
            Maybe(0.3, {{ "Bounty": (gold: {i}, gems: OneOf([1, 2, 3])) }}),
            OneOf([ {{ "Points": (1) }}, Weight(2, {{ "Points": (Range(10, 20)) }}) ]),
        ],
        children: {{
            "turret": ( components: {{ "Name": "Turret", "Stats": (hp: 3, armor: 1) }} ),
            "light": Maybe(0.5, ( components: {{ "Name": "Light" }} )),
        }},
    ),"#,
        armor = i % 7,
        speed = i % 50,
        custom = i % 255
    )
}

fn main() {
    println!(
        "{:>6} {:>9} {:>11} {:>12} {:>13} {:>14}",
        "blueprints", "file", "ron2 parse", "ron parse", "full load", "per blueprint"
    );
    // Sizes from the command line, e.g. `load_bench 100 1000`.
    let sizes: Vec<usize> = std::env::args().skip(1).map(|a| a.parse().expect("size")).collect();
    for n in sizes {
        let text = format!("{{{}\n}}\n", (0..n).map(blueprint).collect::<String>());
        let file = format!("gen/large_{n}.bp.ron");
        std::fs::write(format!("{}/assets/{file}", env!("CARGO_MANIFEST_DIR")), &text).unwrap();

        eprintln!("[{n}] start: {} MB", rss_mb());
        let ron2_parse = best_of(5, || {
            std::hint::black_box(ron2::ast::parse_document(&text).unwrap());
        });
        eprintln!("[{n}] after ron2 parse: {} MB", rss_mb());
        let ron_parse = best_of(5, || {
            std::hint::black_box(ron::from_str::<ron::Value>(&text).unwrap());
        });
        eprintln!("[{n}] after ron parse: {} MB", rss_mb());
        let repeats: usize = std::env::var("REPEATS").ok().and_then(|r| r.parse().ok()).unwrap_or(1);
        let mut load = Duration::MAX;
        for run in 0..repeats {
            let start = Instant::now();
            load_all(&file, n);
            let elapsed = start.elapsed();
            load = load.min(elapsed);
            eprintln!(
                "[{n}] load {run}: {:.1} ms, peak so far {} MB, current {} MB",
                ms(elapsed),
                rss_mb(),
                current_mb()
            );
        }

        println!(
            "{n:>10} {:>7} KB {:>8.1} ms {:>9.1} ms {:>10.1} ms {:>11.1} µs",
            text.len() / 1024,
            ms(ron2_parse),
            ms(ron_parse),
            ms(load),
            load.as_secs_f64() * 1e6 / n as f64,
        );
    }
}

/// Loads the file in a fresh app, until every blueprint is flattened and its scene resolved.
fn load_all(file: &str, n: usize) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), ScenePlugin, BlueprintPlugin))
        .register_type::<Name>()
        .register_type::<Stats>()
        .register_type::<Speed>()
        .register_type::<Tint>()
        .register_type::<Points>()
        .register_type::<Bounty>()
        .register_type::<Placement>();
    app.update();
    let server = app.world().resource::<AssetServer>().clone();
    let root: Handle<BlueprintFile> = server.load(file.to_string());
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(60), "load did not finish in 60 s");
        app.update();
        if server.load_state(&root).is_failed() {
            panic!("load failed: {:?}", server.load_state(&root));
        }
        let files = app.world().resource::<Assets<BlueprintFile>>();
        let patches = app.world().resource::<Assets<ScenePatch>>();
        if let Some(loaded) = files.get(&root) {
            assert_eq!(loaded.labels().count(), n);
            if loaded.labels().all(|l| {
                patches
                    .get(loaded.get(l).unwrap())
                    .is_some_and(|p| p.resolved.is_some())
            }) {
                return;
            }
        }
    }
}

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Resident memory of this process (MB), peak so far.
fn rss_mb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024
}

/// Resident memory of this process right now (MB).
fn current_mb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() / 1024
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
