//! The error type for loading and spawning blueprints.

use std::fmt;

use bevy::asset::AssetPath;
use ron2::error::Span;

/// Something wrong in a blueprint file, found while loading it (or, rarely, while spawning).
///
/// The [message](Self::message) says what is wrong and what was expected; [`kind`](Self::kind)
/// classifies it, and [`file`](Self::file) and [`position`](Self::position) locate it. It is
/// displayed as `file:line:col: message`, leaving out what is unknown.
///
/// Bevy's asset server wraps loading errors in its own types. To get this one back:
///
/// ```ignore
/// use bevy::asset::{AssetLoadError, LoadState};
///
/// if let LoadState::Failed(error) = asset_server.load_state(&handle)
///     && let AssetLoadError::AssetLoaderError(error) = &*error
///     && let Some(error) = error.error().downcast_ref::<BlueprintError>()
/// {
///     warn!("{:?} at {:?}: {}", error.kind(), error.position(), error.message());
/// }
/// ```
#[derive(Clone)]
pub struct BlueprintError(Box<Inner>);

// Boxed, so results that may fail stay small.
#[derive(Clone)]
struct Inner {
    kind: ErrorKind,
    file: Option<AssetPath<'static>>,
    position: Option<Position>,
    message: String,
}

/// What kind of problem a [`BlueprintError`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The file could not be read, or is not UTF-8.
    Read,
    /// Not valid RON, or not shaped like a blueprint file: an unknown entry field, a duplicate
    /// blueprint or child, a missing map or string.
    Syntax,
    /// A value doesn't fit its type: an unknown or unregistered type, an unknown field or
    /// variant, a wrong shape, an incomplete component, a value that can't be copied.
    Type,
    /// A random wrapper is misused: `Maybe` on a value or at the top of a blueprint, random
    /// values inside lists, a bad weight, chance or range, `Range` on a non-number.
    Random,
    /// An `extends` can't be resolved: an inheritance cycle, a missing blueprint, a parent file
    /// that failed to load.
    Inheritance,
    /// An asset path or blueprint reference (`Path(..)`, `Frozen(..)`) is invalid or can't be
    /// resolved.
    Reference,
}

/// A position in a blueprint file, both counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

impl BlueprintError {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self(Box::new(Inner {
            kind,
            file: None,
            position: None,
            message: message.into(),
        }))
    }

    /// Places the error at `span`, unless it already has a (more precise) position.
    pub(crate) fn at(mut self, span: &Span) -> Self {
        self.0.position.get_or_insert(Position {
            line: span.start.line,
            column: span.start.col,
        });
        self
    }

    /// Places the error in `file`, unless it already came from another file.
    pub(crate) fn in_file(mut self, file: &AssetPath) -> Self {
        self.0.file.get_or_insert_with(|| file.clone_owned());
        self
    }

    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The blueprint file the error is in; `None` for errors found while spawning.
    pub fn file(&self) -> Option<&AssetPath<'static>> {
        self.0.file.as_ref()
    }

    /// Where in the file the error is, when known.
    pub fn position(&self) -> Option<Position> {
        self.0.position
    }

    /// What is wrong, without the location.
    pub fn message(&self) -> &str {
        &self.0.message
    }
}

impl fmt::Display for BlueprintError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Inner {
            file, position, message, ..
        } = &*self.0;
        if let Some(file) = file {
            write!(f, "{file}:")?;
        }
        if let Some(position) = position {
            write!(f, "{position}:")?;
        }
        if file.is_some() || position.is_some() {
            f.write_str(" ")?;
        }
        f.write_str(message)
    }
}

// Bevy reports loader errors with `Debug`; keep them readable there too.
impl fmt::Debug for BlueprintError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for BlueprintError {}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}
