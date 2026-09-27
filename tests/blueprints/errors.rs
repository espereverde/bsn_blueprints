//! Load errors: messages, kinds and positions.

use crate::support::*;

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

/// Loads a blueprint that is expected to fail and returns the loader's typed error.
fn blueprint_error(app: &mut App, path: &'static str) -> BlueprintError {
    let handle: Handle<ScenePatch> = app.world().resource::<AssetServer>().load(path);
    for _ in 0..1000 {
        app.update();
        let server = app.world().resource::<AssetServer>();
        if let Some(RecursiveDependencyLoadState::Failed(error)) = server.get_recursive_dependency_load_state(&handle) {
            let AssetLoadError::AssetLoaderError(error) = &*error else {
                panic!("{path}: not a loader error: {error}");
            };
            return error
                .error()
                .downcast_ref::<BlueprintError>()
                .unwrap_or_else(|| panic!("{path}: not a BlueprintError: {error}"))
                .clone();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("{path} was expected to fail");
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

#[test]
fn errors_are_typed_and_located() {
    let mut app = app();
    let at = |line, column| Some(Position { line, column });

    let error = blueprint_error(&mut app, "bad_syntax.bp.ron#field_maybe");
    assert_eq!(error.kind(), ErrorKind::Random, "{error}");
    assert_eq!(
        error.file().map(|f| f.to_string()).as_deref(),
        Some("bad_syntax.bp.ron")
    );
    assert_eq!(error.position(), at(2, 50), "{error}");
    assert!(error.message().starts_with("Maybe(..) only works"), "{error}");
    assert!(
        error.to_string().starts_with("bad_syntax.bp.ron:2:50: Maybe"),
        "{error}"
    );

    // Errors found after parsing still point at what caused them.
    let error = blueprint_error(&mut app, "incomplete.bp.ron#bad");
    assert_eq!(
        (error.kind(), error.position()),
        (ErrorKind::Type, at(3, 28)),
        "{error}"
    );

    let error = blueprint_error(&mut app, "bad_top_maybe.bp.ron#sometimes");
    assert_eq!(
        (error.kind(), error.position()),
        (ErrorKind::Random, at(2, 18)),
        "{error}"
    );

    let error = blueprint_error(&mut app, "missing_parent.bp.ron#orphan");
    assert_eq!(
        (error.kind(), error.position()),
        (ErrorKind::Inheritance, at(2, 26)),
        "{error}"
    );

    let error = blueprint_error(&mut app, "missing_frozen.bp.ron#gun");
    assert_eq!(
        (error.kind(), error.position()),
        (ErrorKind::Reference, at(2, 57)),
        "{error}"
    );

    // A cycle is reported at the `extends` that closes it (which one depends on where it starts).
    let error = blueprint_error(&mut app, "cycle.bp.ron#a");
    assert_eq!(error.kind(), ErrorKind::Inheritance, "{error}");
    assert!(
        matches!(error.position(), Some(Position { line: 2 | 3, .. })),
        "{error}"
    );
}

#[test]
fn a_failing_parent_file_reports_its_own_error() {
    let mut app = app();
    let error = blueprint_error(&mut app, "bad_parent.bp.ron#child");
    assert_eq!(error.kind(), ErrorKind::Type, "{error}");
    assert_eq!(
        error.file().map(|f| f.to_string()).as_deref(),
        Some("incomplete.bp.ron")
    );
    assert_eq!(error.position(), Some(Position { line: 3, column: 28 }), "{error}");
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
fn frozen_blueprint_cycle_fails_to_load() {
    let mut app = app();
    let error = load_error(&mut app, "frozen_cycle.bp.ron#gun");
    assert!(error.contains("cycle"), "{error}");
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
