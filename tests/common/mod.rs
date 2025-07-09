use assert_cmd::Command;
use assert_fs::TempDir;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::PathChild;
use std::path::Path;

pub fn setup_test_library(temp_dir: &TempDir) -> ChildPath {
    let library_dir = temp_dir.child("test_library");
    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("create").arg(library_dir.path()).assert().success();
    library_dir
}

pub fn get_test_photos_dir() -> &'static Path {
    Path::new("tests/fixtures")
}
