// E2E tests for the photosort CLI commands
use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::*;

mod common;
use common::{get_test_photos_dir, setup_test_library};

#[test]
fn test_create_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = temp_dir.child("new_library");

    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("create")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created library at"));

    assert!(library_dir.join("library.db").exists());
    assert!(library_dir.join("images").exists());
    assert!(library_dir.join("videos").exists());
}

#[test]
fn test_import_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);
    let photo_dir = get_test_photos_dir();

    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("import")
        .arg(photo_dir)
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("images imported"));

    // Verify files were copied to the new structure
    // Images should be in images/YYYY/MM-DD/
    assert!(library_dir.child("images").exists());
}

#[test]
fn test_info_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);

    // Check info on an empty library
    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("info")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("0 images"));

    // Import photos and check again
    let photo_dir = get_test_photos_dir();
    let mut import_cmd = Command::cargo_bin("photosort").unwrap();
    import_cmd
        .arg("import")
        .arg(photo_dir)
        .arg(library_dir.path())
        .assert()
        .success();

    let mut info_cmd_after_import = Command::cargo_bin("photosort").unwrap();
    info_cmd_after_import
        .arg("info")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("images"));
}

#[test]
fn test_stats_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);
    let photo_dir = get_test_photos_dir();

    // Import photos first
    let mut import_cmd = Command::cargo_bin("photosort").unwrap();
    import_cmd
        .arg("import")
        .arg(photo_dir)
        .arg(library_dir.path())
        .assert()
        .success();

    // Check stats
    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("stats")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Images:"))
        .stdout(predicate::str::contains("Videos:"))
        .stdout(predicate::str::contains("Sidecars:"));
}

#[test]
fn test_scan_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);

    // Scan command (currently just prints "not yet implemented")
    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("scan")
        .arg(library_dir.path())
        .assert()
        .success();
}

#[test]
fn test_dry_run_import() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);
    let photo_dir = get_test_photos_dir();

    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("import")
        .arg(photo_dir)
        .arg(library_dir.path())
        .arg("--dry-run")
        .assert()
        .success()
        .stdout(predicate::str::contains("DRY RUN"));

    // Library should still be empty after dry run
    let mut info_cmd = Command::cargo_bin("photosort").unwrap();
    info_cmd
        .arg("info")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("0 images"));
}
