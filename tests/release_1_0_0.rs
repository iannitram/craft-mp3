//! Locked tests for issue #11: the crate is version 1.0.0, unpublished,
//! with a CHANGELOG line.

use std::fs;
use std::path::Path;

fn read(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap()
}

#[test]
fn package_version_is_1_x() {
    // 1.0.0 was the first release; later 1.x releases keep the same API line.
    assert!(env!("CARGO_PKG_VERSION").starts_with("1."));
}

#[test]
fn cargo_toml_stays_unpublished() {
    assert!(read("Cargo.toml").contains("publish = false"));
}

#[test]
fn changelog_has_the_1_0_0_line() {
    assert!(read("CHANGELOG.md").contains(" 1.0.0: first release."));
}
