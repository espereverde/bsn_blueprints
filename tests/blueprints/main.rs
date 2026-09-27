//! Integration tests, one module per feature. Each test names the blueprint it exercises;
//! the blueprints are in `assets/*.bp.ron`.
//!
//! One test binary with modules, rather than a binary per file: each binary links all of Bevy.

mod support;

mod children;
mod errors;
mod inheritance;
mod loading;
mod randomness;
mod recipes;
mod references;
mod spawning;
