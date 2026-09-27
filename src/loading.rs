//! Loading blueprint files during a loading state, and moving on when they are ready.

use std::marker::PhantomData;
use std::ops::{Deref, Index};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::state::state::FreelyMutableState;

use crate::instance::BlueprintInstance;
use crate::loader::BlueprintFile;

/// Loads blueprint files when the app enters a loading state, and switches to the next state
/// once all of them (and everything they depend on) have loaded.
///
/// A single file is loaded into a [`Blueprints<T>`] resource, and a named set of files into a
/// [`BlueprintSet<T>`], `T` being any type naming it. The resources exist from the moment
/// loading starts; spawning from them before the files are ready just waits (see
/// [`BlueprintInstance`]).
///
/// ```ignore
/// struct Enemies;
/// struct Armory;
///
/// app.add_plugins(
///     BlueprintLoadingPlugin::new(AppState::Loading, AppState::Playing)
///         .load::<Enemies>("enemies.bp.ron")
///         .load_set::<Armory>([("weapons", "weapons.bp.ron"), ("bullets", "bullets.bp.ron")]),
/// )
/// .add_systems(OnEnter(AppState::Playing), |mut commands: Commands, enemies: Res<Blueprints<Enemies>>| {
///     commands.spawn_blueprint(&enemies, "ufo");
/// });
/// ```
///
/// Add one plugin per loading state, e.g. `Boot → Menu` for the menu's files and
/// `LevelLoading → Playing` for a level's. Entering a loading state loads only its own files.
/// Plugins for the same loading state are combined, so separate game plugins can each add
/// their files and the state switches once all of them have loaded; they must agree on the next
/// state.
///
/// [`BlueprintLoadingProgress<S>`] tells a loading screen how far along the current loading
/// state is. A file that fails to load is logged and counted as failed, and the state doesn't
/// change. Needs [`BlueprintPlugin`](crate::BlueprintPlugin) and Bevy's `StatesPlugin`.
pub struct BlueprintLoadingPlugin<S: FreelyMutableState> {
    loading: S,
    next: S,
    groups: Vec<Group>,
}

/// What to load in each loading state of `S`.
#[derive(Resource)]
struct LoadingPlans<S: FreelyMutableState> {
    plans: Vec<Plan<S>>,
}

/// The files to load in a loading state, handles to them once loading has started, and the
/// state after.
struct Plan<S> {
    loading: S,
    next: S,
    groups: Vec<Group>,
}

impl<S: FreelyMutableState> LoadingPlans<S> {
    fn plan_mut(&mut self, loading: &S) -> Option<&mut Plan<S>> {
        self.plans.iter_mut().find(|plan| plan.loading == *loading)
    }
}

/// Files that go into one resource.
#[derive(Clone)]
struct Group {
    files: Vec<FileToLoad>,
    insert: InsertGroup,
}

/// Inserts a group's resource, given `(name, handle)` for each of its files.
type InsertGroup = fn(&mut Commands, Vec<(String, Handle<BlueprintFile>)>);

#[derive(Clone)]
struct FileToLoad {
    name: String,
    path: String,
    handle: Option<Handle<BlueprintFile>>,
    failed: bool,
}

impl FileToLoad {
    fn new(name: String, path: String) -> Self {
        Self {
            name,
            path,
            handle: None,
            failed: false,
        }
    }
}

impl<S: FreelyMutableState> BlueprintLoadingPlugin<S> {
    /// Loads files on entering `loading`, then switches to `next`.
    pub fn new(loading: S, next: S) -> Self {
        Self {
            loading,
            next,
            groups: Vec::new(),
        }
    }

    /// Adds a file to load, into the resource [`Blueprints<T>`].
    pub fn load<T: Send + Sync + 'static>(mut self, path: impl Into<String>) -> Self {
        self.groups.push(Group {
            files: vec![FileToLoad::new(String::new(), path.into())],
            insert: |commands, mut files| {
                let (_, file) = files.pop().expect("one file");
                commands.insert_resource(Blueprints::<T>::new(file));
            },
        });
        self
    }

    /// Adds a set of files to load, by name, into the resource [`BlueprintSet<T>`]. Names can't
    /// contain `.` (it separates file and blueprint in [`BlueprintSet::instance`]).
    ///
    /// # Panics
    ///
    /// If a name is empty, contains `.`, or is given twice.
    pub fn load_set<T: Send + Sync + 'static>(
        mut self,
        files: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        let mut to_load: Vec<FileToLoad> = Vec::new();
        for (name, path) in files {
            let name = name.into();
            assert!(
                !name.is_empty() && !name.contains('.'),
                "blueprint set name `{name}` must be non-empty and without `.`"
            );
            assert!(
                to_load.iter().all(|file| file.name != name),
                "blueprint set name `{name}` is given twice"
            );
            to_load.push(FileToLoad::new(name, path.into()));
        }
        self.groups.push(Group {
            files: to_load,
            insert: |commands, files| {
                commands.insert_resource(BlueprintSet::<T> {
                    files: files.into_iter().collect(),
                    _marker: PhantomData,
                });
            },
        });
        self
    }
}

impl<S: FreelyMutableState> Plugin for BlueprintLoadingPlugin<S> {
    fn build(&self, app: &mut App) {
        let mut plans = app
            .world_mut()
            .get_resource_or_insert_with(|| LoadingPlans::<S> { plans: Vec::new() });
        if let Some(plan) = plans.plan_mut(&self.loading) {
            assert!(
                plan.next == self.next,
                "BlueprintLoadingPlugin: loading state {:?} already switches to {:?}; it can't also switch to {:?}",
                self.loading,
                plan.next,
                self.next
            );
            plan.groups.extend(self.groups.iter().cloned());
            return;
        }
        plans.plans.push(Plan {
            loading: self.loading.clone(),
            next: self.next.clone(),
            groups: self.groups.clone(),
        });
        app.init_resource::<BlueprintLoadingProgress<S>>()
            .add_systems(OnEnter(self.loading.clone()), start_loading::<S>)
            .add_systems(Update, check_loading::<S>.run_if(in_state(self.loading.clone())));
    }

    fn is_unique(&self) -> bool {
        false
    }
}

/// A blueprint file loaded by [`BlueprintLoadingPlugin::load`]; `T` names it. Dereferences to
/// the file's handle, so it can be passed to [`spawn_blueprint`](crate::BlueprintCommandsExt).
#[derive(Resource)]
pub struct Blueprints<T: Send + Sync + 'static> {
    file: Handle<BlueprintFile>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Send + Sync + 'static> Blueprints<T> {
    pub fn new(file: Handle<BlueprintFile>) -> Self {
        Self {
            file,
            _marker: PhantomData,
        }
    }

    pub fn handle(&self) -> &Handle<BlueprintFile> {
        &self.file
    }
}

impl<T: Send + Sync + 'static> Deref for Blueprints<T> {
    type Target = Handle<BlueprintFile>;

    fn deref(&self) -> &Handle<BlueprintFile> {
        &self.file
    }
}

/// Blueprint files loaded by [`BlueprintLoadingPlugin::load_set`], by name; `T` names the set.
///
/// ```ignore
/// fn shoot(mut commands: Commands, armory: Res<BlueprintSet<Armory>>) {
///     commands.spawn_blueprint(&armory["weapons"], "turret");    // file by name, then label
///     if let Some(bullet) = armory.instance("bullets.normal") {  // or "file.label"
///         commands.spawn(bullet);
///     }
/// }
/// ```
#[derive(Resource)]
pub struct BlueprintSet<T: Send + Sync + 'static> {
    files: HashMap<String, Handle<BlueprintFile>>,
    _marker: PhantomData<fn() -> T>,
}

impl<T: Send + Sync + 'static> BlueprintSet<T> {
    /// The file called `name`.
    pub fn file(&self, name: &str) -> Option<&Handle<BlueprintFile>> {
        self.files.get(name)
    }

    /// The names of the files in the set.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The blueprint `"file.label"`, ready to spawn: the file called `file`, the blueprint
    /// `label` in it. `None` if there is no `.` or no such file; a missing label is reported
    /// when spawning, like for any [`BlueprintInstance`].
    pub fn instance(&self, name: &str) -> Option<BlueprintInstance> {
        let (file, label) = name.split_once('.')?;
        Some(BlueprintInstance::new(self.file(file)?, label.to_string()))
    }
}

impl<T: Send + Sync + 'static> Index<&str> for BlueprintSet<T> {
    type Output = Handle<BlueprintFile>;

    /// The file called `name`.
    ///
    /// # Panics
    ///
    /// If the set has no such file.
    fn index(&self, name: &str) -> &Handle<BlueprintFile> {
        self.file(name).unwrap_or_else(|| {
            let mut names: Vec<_> = self.names().collect();
            names.sort_unstable();
            panic!(
                "no blueprint file `{name}` in BlueprintSet<{}>; it has {names:?}",
                std::any::type_name::<T>()
            )
        })
    }
}

/// How far [`BlueprintLoadingPlugin`] is in the current loading state of `S`: files loaded
/// (with their dependencies), failed, and in total. Reset on entering a loading state and
/// updated every frame while in it.
#[derive(Resource)]
pub struct BlueprintLoadingProgress<S: FreelyMutableState> {
    counts: Counts,
    _state: PhantomData<fn() -> S>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Counts {
    loaded: usize,
    failed: usize,
    total: usize,
}

impl<S: FreelyMutableState> Default for BlueprintLoadingProgress<S> {
    fn default() -> Self {
        Self {
            counts: Counts::default(),
            _state: PhantomData,
        }
    }
}

impl<S: FreelyMutableState> BlueprintLoadingProgress<S> {
    /// Files loaded, with everything they depend on.
    pub fn loaded(&self) -> usize {
        self.counts.loaded
    }

    /// Files that failed to load.
    pub fn failed(&self) -> usize {
        self.counts.failed
    }

    /// Files to load in this loading state.
    pub fn total(&self) -> usize {
        self.counts.total
    }

    /// The part loaded, from 0 to 1 (1 when there is nothing to load).
    pub fn fraction(&self) -> f32 {
        if self.counts.total == 0 {
            1.0
        } else {
            self.counts.loaded as f32 / self.counts.total as f32
        }
    }

    /// Whether every file has loaded.
    pub fn is_done(&self) -> bool {
        self.counts.loaded == self.counts.total
    }
}

impl<S: FreelyMutableState> std::fmt::Debug for BlueprintLoadingProgress<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("BlueprintLoadingProgress")
            .field("loaded", &self.counts.loaded)
            .field("failed", &self.counts.failed)
            .field("total", &self.counts.total)
            .finish()
    }
}

fn start_loading<S: FreelyMutableState>(
    mut commands: Commands,
    state: Res<State<S>>,
    mut plans: ResMut<LoadingPlans<S>>,
    mut progress: ResMut<BlueprintLoadingProgress<S>>,
    server: Res<AssetServer>,
) {
    let Some(plan) = plans.plan_mut(state.get()) else { return };
    let mut total = 0;
    for group in &mut plan.groups {
        let mut handles = Vec::with_capacity(group.files.len());
        for file in &mut group.files {
            let handle: Handle<BlueprintFile> = server.load(&file.path);
            handles.push((file.name.clone(), handle.clone()));
            file.handle = Some(handle);
            file.failed = false;
        }
        total += group.files.len();
        (group.insert)(&mut commands, handles);
    }
    progress.counts = Counts {
        total,
        ..default()
    };
}

fn check_loading<S: FreelyMutableState>(
    state: Res<State<S>>,
    mut plans: ResMut<LoadingPlans<S>>,
    mut progress: ResMut<BlueprintLoadingProgress<S>>,
    mut next_state: ResMut<NextState<S>>,
    server: Res<AssetServer>,
) {
    let Some(plan) = plans.plan_mut(state.get()) else { return };
    let mut now = Counts::default();
    for file in plan.groups.iter_mut().flat_map(|group| &mut group.files) {
        now.total += 1;
        let Some(handle) = &file.handle else { continue };
        if server.is_loaded_with_dependencies(handle) {
            now.loaded += 1;
        } else if server.recursive_dependency_load_state(handle).is_failed() {
            now.failed += 1;
            if !file.failed {
                file.failed = true;
                error!(
                    "blueprint file {} failed to load: {:?}",
                    file.path,
                    server.recursive_dependency_load_state(handle)
                );
            }
        }
    }
    if progress.counts != now {
        progress.counts = now;
    }
    if now.loaded == now.total {
        next_state.set(plan.next.clone());
    }
}
