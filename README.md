# bsn_blueprints

Data-driven game entities for **Bevy 0.19**, defined in `*.bp.ron` files that can be edited
without recompiling. Blueprints can inherit from each other, reference each other, and describe
random variation that is rolled every time an entity is spawned.

Each blueprint becomes a regular Bevy scene (`ScenePatch`), so it spawns through the stock scene
API and can be used from `bsn!`.

> **Status: early.** Everything described here works and is covered by tests, but the API may
> still change between commits, and the crate is not published on crates.io yet.

---

## Contents

- [Quick start](#quick-start)
- [The file format](#the-file-format)
  - [Files and blueprints](#files-and-blueprints)
  - [Components](#components)
  - [Inheritance](#inheritance)
  - [Children](#children)
  - [Randomness](#randomness)
  - [Asset handles and blueprint references](#asset-handles-and-blueprint-references)
  - [Recipes](#recipes)
- [How components are built](#how-components-are-built)
- [Using blueprints from Rust](#using-blueprints-from-rust)
  - [Spawning](#spawning)
  - [Loading state](#loading-state)
  - [Scenes and `bsn!`](#scenes-and-bsn)
- [Errors](#errors)
- [Performance](#performance)
- [Limitations and gotchas](#limitations-and-gotchas)
- [For developers](#for-developers)
- [License](#license)

---

## Quick start

```toml
[dependencies]
bevy = { version = "0.19", features = ["serialize"] }   # `serialize`: serde support for Name etc.
bsn_blueprints = { git = "https://github.com/espereverde/bsn_blueprints" }
# or, from a local checkout (e.g. a git submodule):
# bsn_blueprints = { path = "libs/bsn_blueprints" }
```

Needs Rust 1.95 or newer (Bevy 0.19's minimum).

```rust
use bevy::prelude::*;
use bsn_blueprints::{BlueprintCommandsExt, BlueprintPlugin};

#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Stats { hp: u32, armor: u32 }

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, BlueprintPlugin))
        .register_type::<Stats>()
        .add_systems(Startup, spawn_ufo)
        .run();
}

fn spawn_ufo(mut commands: Commands, server: Res<AssetServer>) {
    // Keep this handle (e.g. in a resource) to spawn more later; the file keeps its blueprints alive.
    let enemies = server.load("enemies.bp.ron");
    // Spawns as soon as the file has loaded.
    commands.spawn_blueprint(&enemies, "ufo").insert(Transform::from_xyz(0.0, 50.0, 0.0));
}
```

`assets/enemies.bp.ron`:

```ron
{
    "ufo": (
        components: {
            "Name": "Ufo",
            "Stats": (hp: Range(5, 10), armor: 1),
        },
    ),
}
```

Every type used in a blueprint must be **registered** (`register_type`) and reflect
`Component` (or [`Recipe`](#recipes)). Add `#[reflect(Default)]` to allow partial values.
With `MinimalPlugins`, that includes Bevy types like `Name`.

For a complete, runnable example (a loading state, spawning by name, a recipe, plain and
`Frozen` blueprint references), run `cargo run --example armory`; its blueprints are in
`assets/examples/`.

---

## The file format

Files are [RON](https://github.com/ron-rs/ron). The loader handles the `.bp.ron` extension.

### Files and blueprints

A file is a map of **label → blueprint**:

```ron
{
    "rock": ( components: { .. } ),
    "big_rock": ( extends: "#rock", components: { .. }, children: { .. } ),
}
```

A blueprint entry has three optional fields:

| Field | Meaning |
|---|---|
| `extends: "path"` | Start from another blueprint (`"#label"` in this file, `"other.bp.ron#label"` in another). |
| `components: ..` | Components to add or change. A map, or a list of maps and random sets (see below). |
| `children: { "name": .. }` | Child entities, by name. |

`()` is an empty blueprint.

### Components

```ron
components: {
    "Name": "Rock",                                  // types by short name...
    "my_game::Stats": (hp: 10, armor: 2),            // ...or full type path
    "Tint": Gold,                                    // enum variant
    "Speed": (1.0),                                  // tuple struct
    "Placement": (offset: (x: 1.0, y: 2.0)),         // nested struct
    "Loot": (items: ["gem", "coin"], bonus: Some(3)),// lists, Option
}
```

Values are read with Bevy's type registry, following the component's reflected shape: named
structs `(a: ..)` or `Name(a: ..)`, tuple structs `(..)` or `Name(..)`, enum variants `Variant`,
`Variant(..)`, `Variant(field: ..)`, `Some(..)` / `None`, lists `[..]`, maps `{..}`. Leaf types
(numbers, strings, `Name`, ...) use their serde format. A struct that has a serde form (like
`Vec3`) may use either form.

**Partial values.** Named-struct values only need the fields they set: `"Stats": (hp: 40)`
leaves `armor` as inherited, or as `Default` for a new component. Types without
`#[reflect(Default)]` must be complete the first time they appear.

**Repeating a component** in the same blueprint merges field by field; later wins per field.

### Inheritance

```ron
"big_rock": (
    extends: "#rock",
    components: { "Stats": (hp: 40) },            // armor comes from #rock
),
```

- Overrides are field-level, at any depth (`"Placement": (offset: (y: 5.0))` keeps `offset.x`).
- Parents can live in other files; each file is read once.
- Entry order in a file doesn't matter. Cycles are reported as errors.
- A field set in a child replaces any random value inherited for it.

### Children

```ron
"saucer": (
    components: { "Name": "Saucer" },
    children: {
        "turret": ( extends: "#turret", components: { "Stats": (armor: 4) } ),
        "light": (
            components: { "Name": "Light" },
            children: { "bulb": ( components: { "Name": "Bulb" } ) },
        ),
    },
),
"big_saucer": (
    extends: "#saucer",
    children: { "turret": ( components: { "Stats": (hp: 30) } ) },   // patch the inherited turret
),
```

Children are named so that derived blueprints can patch them; a new name adds a child.

### Randomness

Everything is fixed unless wrapped. The same wrappers work at every level:

| Wrapper | Field / value | Component | Set of components | Child | Blueprint |
|---|---|---|---|---|---|
| `OneOf([a, Weight(3, b), ..])` | pick one value | pick one value | pick one set | pick one | pick one |
| `Maybe(p, x)` | – (error) | add it or not | add it or not | spawn it or not | – (error) |
| `Range(min, max)` | numbers (inclusive) | – | – | – | – |
| `Fixed(x)` | `x` literally, even if named like a wrapper | | | | |

Choices are made **on every spawn**.

```ron
"saucer": OneOf([                                              // a blueprint that is a choice
    Weight(3, ( extends: "#saucer_base", components: { "Tint": Gold } )),
    (
        extends: "#saucer_base",
        components: [                                          // a list: sets applied in order
            {
                "Tint": Custom(1),
                "Name": OneOf(["Ann", "Bob"]),                 // strings
                "Stats": (hp: Range(5, 10), armor: OneOf([1, 3, 7])),
                "Placement": (offset: OneOf([(x: 0.0), (y: 9.0)])),   // partial structs
                "Shield": Maybe(0.25, (strength: 3)),          // an optional component
            },
            OneOf([ { "Gun": (kind: Laser) }, { "Gun": (kind: Bomb), "Points": (7) } ]),
            Maybe(0.5, { "Bounty": (gold: 10) }),
        ],
        children: {
            "pilot": Maybe(0.3, ( extends: "#pilot" )),
            "gunner": OneOf([ ( extends: "#gunner_a" ), ( extends: "#gunner_b" ) ]),
        },
    ),
]),
```

Rules:

- **Chosen values and sets merge.** A `OneOf` option or chosen set only changes the fields it
  sets: base `Stats(hp: 5, armor: 5)` plus a chosen `(hp: 50)` gives `hp: 50, armor: 5`.
  Chosen sets are applied after the fixed components.
- **Extending a random blueprint** applies your changes to every option.
- **Redeclaring an inherited child:** a plain entry patches it (every option, if it is random).
  A `OneOf` / `Maybe` replaces its randomness: the options build on the inherited child without
  its `Maybe`s (`"lamp": Maybe(0.5, ())` makes a child optional, keeping its values), or on
  nothing if the inherited child is a `OneOf`.
- A wrapper is recognized by name *and* shape (`OneOf([..])`, `Maybe(number, ..)`), so an enum
  variant called `OneOf` is still a plain value. `Fixed(..)` covers exact clashes.
- Nesting is free for blueprints, children and sets. Inside a value-level `OneOf`, each option
  must be a plain value (no `Range` / `OneOf` inside options).
- Random values can't go inside lists or maps.

### Asset handles and blueprint references

Handle fields take `Path("..")`, **relative to the blueprint file**:

```ron
"Skin": (Path("red.palette.ron")),       // next to this file
"Icon": (Path("/ui/icons/ufo.png")),     // relative to the assets folder
```

A `Handle<ScenePatch>` field refers to another blueprint, for game code to spawn later (a gun's
bullets, a spawner's enemies):

```rust
#[derive(Component, Reflect, Default, Clone)]
#[reflect(Component, Default)]
struct Ammo { bullet: Handle<ScenePatch> }
```

```ron
"varied_turret": ( components: { "Ammo": (bullet: Path("#bullet")) } ),          // new random bullet each shot
"steady_turret": ( components: { "Ammo": (bullet: Frozen(Path("#bullet"))) } ),  // one random bullet per turret
"armory": ( components: { "Arsenal": (bullets: [ Path("bullets.bp.ron#big"), Frozen(Path("#shell")) ]) } ),
```

- `Path("#bullet")` / `Path("other.bp.ron#bullet")`: every spawn from that handle makes new
  random choices.
- `Frozen(Path(..))`: when the owning component spawns, the field gets a copy of the blueprint
  with **every random choice made once** (values, sets, children, nested references), so that
  owner always spawns identical entities. The copy is freed with the owner.
- References to other files load each file once, however many of its blueprints are used; a
  blueprint only becomes ready when the files it refers to are loaded.

### Recipes

A recipe is a few parameters in the file, expanded into a bundle by Rust code:

```rust
use bsn_blueprints::{Recipe, ReflectRecipe};

#[derive(Reflect, Default)]
#[reflect(Recipe, Default)]
struct RockRecipe { size: u32 }

impl Recipe for RockRecipe {
    fn bundle(&self) -> impl Bundle + use<> {
        (
            Stats { hp: self.size * 10, armor: self.size },
            Speed(10.0 / self.size as f32),
            children![Name::new("Shard")],
        )
    }
}
```

```ron
"rock": ( components: { "RockRecipe": (size: Range(1, 4)), "Points": (99) } ),
```

- Written like a component, with inheritance, randomness and options like any other.
- Its bundle is inserted **before** the listed components, which override it.
- The recipe value itself is not added to the entity.
- One random `size` feeds every derived value, so they stay consistent.
- The bundle must own its data (`use<>`): clone what you need from `self`.

---

## How components are built

No constructor (`new()`) runs. Values are assembled through reflection:

- A new component starts from its `Default` impl (if it has `#[reflect(Default)]`), or is built
  field by field from the file. Leaf values use their serde impls.
- Overrides and random values write fields directly, by path.
- At spawn the stored value is copied and moved into the entity as is.

Consequences:

- **Private fields can be set** from a file; validation in `new()` is bypassed. Fields marked
  `#[reflect(ignore)]` can't be set (they keep their default).
- **Derived values are not computed** (a `Timer` built from `fps`, a normalized vector, a
  cache). Compute them in a **recipe**, a **component hook / observer** (`on_add`, `on_insert`,
  `On<Add, T>`), or a system reacting to `Added<T>`.
- Hooks, observers and `#[require(..)]` components **do** run: inserts go through Bevy's normal
  insert path.

---

## Using blueprints from Rust

### Spawning

```rust
use bsn_blueprints::{BlueprintCommandsExt, BlueprintEntityCommandsExt, BlueprintFile, BlueprintInstance};

// Load the file once and keep the handle; the file keeps its blueprints alive.
let enemies: Handle<BlueprintFile> = server.load("enemies.bp.ron");

// Spawn by label, loaded or not (these three are equivalent):
commands.spawn_blueprint(&enemies, "ufo").insert(Transform::default());
commands.spawn((BlueprintInstance::new(&enemies, "ufo"), Transform::default()));
commands.spawn(Transform::default()).insert_blueprint(&enemies, "ufo");
```

A `BlueprintInstance` spawns as soon as its file (and every blueprint file it refers to) has
loaded; if the file is already loaded, an instance added during `Update` is complete by the end of
that frame. The blueprint's components replace ones the entity already has; others are kept. If
the file fails to load or has no such label, the error is logged and the `BlueprintInstance`
removed. `BlueprintPlugin` needs Bevy's `ScenePlugin` (part of `DefaultPlugins`).

### Loading state

`BlueprintLoadingPlugin` loads files when the app enters a loading state and
switches to the next one when all of them (and everything they depend on) have loaded. Each file
goes into a `Blueprints<T>` resource, and a named set of files into a `BlueprintSet<T>`, `T`
being any type that names it:

```rust
use bsn_blueprints::{BlueprintCommandsExt, BlueprintLoadingPlugin, BlueprintLoadingProgress, BlueprintSet, Blueprints};

struct Enemies;
struct Armory;

app.add_plugins(
    BlueprintLoadingPlugin::new(AppState::Loading, AppState::Playing)
        .load::<Enemies>("enemies.bp.ron")
        .load_set::<Armory>([("weapons", "weapons.bp.ron"), ("bullets", "bullets.bp.ron")]),
)
.add_systems(OnEnter(AppState::Playing), |mut commands: Commands, enemies: Res<Blueprints<Enemies>>| {
    commands.spawn_blueprint(&enemies, "ufo");
});

fn shoot(mut commands: Commands, armory: Res<BlueprintSet<Armory>>) {
    commands.spawn_blueprint(&armory["weapons"], "turret");    // file by name (panics if unknown), then label
    if let Some(bullet) = armory.instance("bullets.normal") {  // or "file.label"
        commands.spawn(bullet);
    }
}

// A loading screen can show `Res<BlueprintLoadingProgress<AppState>>`:
// `loaded()`, `failed()`, `total()`, `fraction()`, `is_done()`.
```

Set names can't contain `.`. The resources exist as soon as loading starts. A file that fails to
load is logged and counted in `failed()`, and the app stays in the loading state. Needs Bevy's
`StatesPlugin` (part of `DefaultPlugins`).

Add one plugin per loading state (`Boot → Menu` for the menu's files, `LevelLoading → Playing`
for a level's): entering a loading state loads only its own files, and the progress is reset.
Plugins for the *same* loading state are combined, so separate game plugins can each add their
files; the state switches once all of them have loaded. They must agree on the next state (a
conflict panics when the plugin is added).

### Scenes and `bsn!`

The scenes themselves, once the file is loaded:

```rust
let files = world.resource::<Assets<BlueprintFile>>();
let ufo: Handle<ScenePatch> = files.get(&enemies).unwrap().get("ufo").unwrap().clone();
for label in files.get(&enemies).unwrap().labels() { /* ... */ }
commands.spawn(ScenePatchInstance(ufo.clone()));          // waits for the scene if needed
world.spawn_scene(CachedSceneAsset::from("enemies.bp.ron#ufo"))?;   // only once it's ready
```

**Prefer `BlueprintInstance` / `BlueprintFile::get` over loading `"file.bp.ron#label"` paths.**
Bevy 0.19 loads the whole file again for every label requested before the file has loaded;
requesting thousands of labels that way can exhaust memory.

A `bsn!` scene can inherit a blueprint and add components:
`bsn! { :"enemies.bp.ron#ufo" Marker }`. A component set in `bsn!` replaces the blueprint's
value of that component as a whole (field-level merging only happens inside blueprint files).

---

## Errors

Load errors name the file and position, and say what is expected:

```
enemies.bp.ron:3:24: unknown or ambiguous type `Nope`; is it registered?
enemies.bp.ron:2:49: no field `colour` in `game::Stats`; expected one of ["hp", "armor"]
enemies.bp.ron:4:32: unknown variant `Purple` of `game::Tint`; expected one of ["Grey", "Gold", "Custom"]
enemies.bp.ron:1:38: `u32`: invalid type: floating point `2.5`, expected u32
enemies.bp.ron:2:50: Maybe(..) only works on components, sets of components and children; use OneOf(..) for values
rocks.bp.ron:3:21: inheritance cycle in rocks.bp.ron: a -> b -> a
```

Errors found after parsing (inheritance, incomplete components, missing blueprints) point at
what caused them: the `extends`, the `Frozen(..)` path, or the component's name. If a parent file
fails to load, a file that extends it reports the parent's own error, located in the parent.

The loader's error type is `BlueprintError`, with `kind()` (`Read`, `Syntax`, `Type`, `Random`,
`Inheritance`, `Reference`), `file()`, `position()` and `message()`. Bevy's asset server wraps it;
to get it back from a failed load:

```rust
use bevy::asset::{AssetLoadError, LoadState};
use bsn_blueprints::BlueprintError;

if let LoadState::Failed(error) = asset_server.load_state(&handle)
    && let AssetLoadError::AssetLoaderError(error) = &*error
    && let Some(error) = error.error().downcast_ref::<BlueprintError>()
{
    warn!("{:?} at {:?}: {}", error.kind(), error.position(), error.message());
}
```

Random fields are test-sampled once at load time, so bad paths and types fail when loading, not
when spawning. That sample is thrown away; every spawn still makes its own random choices.

---

## Performance

Release build, measured with the examples (see [For developers](#for-developers)); results
vary about ±10% between runs.

**Spawning** (per entity):

| | |
|---|---|
| Plain Rust `world.spawn((5 components))` | ~66 ns |
| Bevy scene with the same 5 components | ~410 ns |
| Blueprint, 5 fixed components | ~680 ns |
| Blueprint, 5 components, 3 random | ~820 ns |
| Blueprint with random values, a `OneOf` set, a `Maybe` set and a child | ~2.0 µs |
| Frozen copy of that | ~1.7 µs |

A child entity costs roughly 0.9 µs. Spawned entities are plain components: no per-frame cost.

**Loading** (blueprints with inheritance chains, random values and sets, fixed and random
children): about 100–140 µs per blueprint. A 3.5 MB file with 5,000 blueprints loads in about
0.7 s; about 40% of that is RON parsing, which also briefly peaks at ~500 MB for a file that size.

---

## Limitations and gotchas

- Random values can't go inside lists, maps, or `OneOf` options.
- `bsn!` overrides replace whole components (see above).
- Blueprints don't wait for non-blueprint assets they reference (images, ...); those appear when
  loaded, as usual in Bevy.
- Labeled paths to non-blueprint assets (`Path("ship.gltf#Mesh0")`) use Bevy's normal loading:
  many distinct labels of one unloaded file load it repeatedly.
- Cycles across files (`a.bp.ron` extends/freezes `b.bp.ron`, which does the same back) are not
  detected.
- A turret with a `Frozen` bullet keeps its copy for life; hot-reloading the bullet file only
  affects owners spawned afterwards.
- Large files: parsing memory peaks at roughly 150× the file size, briefly.

---

## For developers

### Pipeline

```
*.bp.ron ──ron2──▶ syntax tree ──parse──▶ RawNode / RawEntry / RawItem / RawComponent
                                             │   (partial values, random specs, refs, by path)
                                             ▼
                                       flatten (per file)
                            extends, children, random sets resolved against each
                            blueprint's final components, frozen targets
                                             │
                                             ▼
                          Node = Fixed(Blueprint) | OneOf | Maybe     ──▶ BlueprintFile asset
                                             │                              + one labeled ScenePatch
                                             ▼                                per blueprint
                   BlueprintScene::resolve: fixed children → related scenes,
                   one bundle template per blueprint (or per random root)
                                             │
                                             ▼  every spawn
             choose (sets, random children) → sample random fields → fill frozen/refs
             → one batched insert (insert_by_ids) → spawn random children directly
```

### Modules

| Module | Responsibility |
|---|---|
| `lib.rs` | Format docs, `BlueprintPlugin`, public exports. |
| `loading.rs` | `BlueprintLoadingPlugin`, `Blueprints<T>`, `BlueprintSet<T>`, `BlueprintLoadingProgress<S>`. |
| `instance.rs` | `BlueprintInstance`, `spawn_blueprint` / `insert_blueprint`, the system that spawns instances once their file has loaded. |
| `loader.rs` | `BlueprintLoader`, `BlueprintFile` (+ `get`/`labels`), loading parent files once, labeled scenes and their dependencies. |
| `parse/mod.rs` | Raw structure (`RawNode`, `RawEntry`, `RawItem`, `RawComponent`), wrapper recognition (`OneOf`/`Maybe`/`Fixed`/`Weight`), entries, components. |
| `parse/value.rs` | `ValueReader`: builds partial reflected values from the tree with the registry; records random / frozen / reference fields by path; handle fields. |
| `parse/expr.rs` | `ron2` helpers, positioned errors, serde `Deserializer` over expressions for leaf values. |
| `error.rs` | `BlueprintError`, `ErrorKind`, `Position`. |
| `flatten.rs` | Inheritance: nodes, children (incl. the wrapper redeclaration rule), random sets resolved per blueprint, cycles. |
| `blueprint.rs` | `Node`, `Blueprint`, `Part`/`PartOption`, `BlueprintComponent`, patching, random/frozen/ref fields, referenced files. |
| `random.rs` | `RandomSpec`, `RandomField` (pre-parsed paths), weighted choice, numeric ranges. |
| `spawn.rs` | Scene integration, templates, per-spawn choices, batched insert (the crate's only `unsafe`). |
| `freeze.rs` | Frozen copies and spawn-time field values. |
| `recipe.rs` | `Recipe` trait and `ReflectRecipe` type data. |
| `reflect_utils.rs` | Cloning reflected values, leaf paths, path overlap. |

### Key design decisions

- **`ron2` instead of `ron`.** `ron` drops enum/struct names when the target type isn't known,
  so wrappers could only be detected in a few places, through tricks. `ron2` parses to a syntax
  tree with every name and position, so wrappers work anywhere and errors have positions.
- **Our own reflect reader.** Values are built from the tree with `TypeInfo`, producing *partial*
  dynamic values (Bevy's deserializer converts everything with `FromReflect`, which fills missing
  fields with defaults). Only leaves go through serde.
- **Components as a bundle template with `Output = ()`.** Component templates would be keyed by
  the component type and need per-type code; ours inserts reflected values itself. Downside:
  `bsn!` field patches can't merge into blueprint components.
- **Batched insert with `insert_by_ids`** (one archetype move, values moved rather than rebuilt
  by `ReflectComponent::insert`). Guarded by a concrete-type check. Bevy's own bundle writer
  panics on duplicate components, which would break `bsn!` overrides, so we don't use it.
- **Random options are prepared at load, chosen at spawn.** Each `OneOf` option is merged with
  the base value when flattening, and random sets are merged into each blueprint's final
  components (redone for every derived blueprint). Each spawn then only picks an option and
  copies it; nothing is merged at spawn time.
- **Inheritance is flattened in the loader** (same-file in memory, other files through an
  immediate nested load), so scene resolution order doesn't matter.
- **No labeled loads from the loader** for blueprint references (same file: `get_label_handle`;
  other files: load the file, fill the field on spawn from `BlueprintFile::get`).

### Bevy 0.19 quirks we ran into

- Loading `"file#label"` for a label without a handle force-loads the whole file again.
  Thousands of such requests froze a 62 GB machine. Load files, not labels.
- `TypedReflectDeserializer` always converts through `FromReflect` when possible (fills defaults).
- `Arc<T>` implements `Reflect` itself: call reflect methods on `*arc`, not on the `Arc`.
- `SceneComponent` props aren't reflected, so they can't be used from data files.
- The scene bundle writer panics on duplicate components.
- There is no first-party `.bsn` asset loader yet (planned upstream).

### Tests and benchmarks

```sh
cargo test                                            # 47 integration tests (tests/blueprints/)
cargo run --example armory                            # the example game (headless, prints what happens)
cargo run --release --example bench                   # spawn cost
cargo run --release --example load_bench 100 1000     # load time; files go to assets/gen/
```

CI (`.github/workflows/ci.yml`) runs on stable: `cargo fmt --check` (`rustfmt.toml`: 120 columns),
clippy with warnings as errors, the tests, `cargo doc` with warnings as errors, and the `armory`
example; and `cargo check` on the minimum Rust version (1.95). The crate doesn't pin a toolchain;
inside Lunar Watch, the game's `rust-toolchain.toml` applies.

Integration tests are one binary (`tests/blueprints/main.rs`) with a module per feature:
`inheritance`, `randomness`, `children`, `references`, `recipes`, `errors`, `spawning`,
`loading`. A binary per file would link all of Bevy once per file. `support.rs` has the test
"game" types, the app harness and shared helpers; helpers used by one feature live in its module.
Run one feature with `cargo test loading::`.

Tests use `assets/*.bp.ron`; each test names the blueprint it exercises. Statistical tests use
wide margins (~6σ) and have been run repeatedly (60× without failure).

**Run the load benchmark with a memory cap** when trying large sizes or changing the loader, e.g.:

```sh
CARGO_MANIFEST_DIR=$PWD systemd-run --user --scope -E CARGO_MANIFEST_DIR=$PWD \
    -p MemoryMax=4G -p MemorySwapMax=0 target/release/examples/load_bench 5000
```

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
