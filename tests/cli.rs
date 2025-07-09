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
        .stdout(predicate::str::contains("Successfully created library"));

    assert!(library_dir.join("library.db").exists());
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
        // There are 5 images and 1 sidecar file, so 5 photos total.
        .stdout(predicate::str::contains(
            "Import complete. Library now has 5 photos.",
        ));

    // Verify that some files were copied
    assert!(
        library_dir
            .child("2024/05-17")
            .child("IMG_8040.HEIC")
            .exists()
    );
    assert!(
        library_dir
            .child("2024/05-21")
            .child("_DSC6936-2.NEF")
            .exists()
    );
    assert!(
        library_dir
            .child("2024/05-21")
            .child("_DSC6936-2.xmp")
            .exists()
    );
}

#[test]
fn test_info_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);

    // First, check info on an empty library
    let mut cmd = Command::cargo_bin("photosort").unwrap();
    cmd.arg("info")
        .arg(library_dir.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("has 0 photos and 0 sidecars."),
        );

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
        // There should be 5 photos after import.
        .stdout(
            predicate::str::contains("has 5 photos"),
        );
}

#[test]
fn test_update_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let library_dir = setup_test_library(&temp_dir);
    let photo_dir = get_test_photos_dir();

    // Import photos
    let mut import_cmd = Command::cargo_bin("photosort").unwrap();
    import_cmd
        .arg("import")
        .arg(photo_dir)
        .arg(library_dir.path())
        .assert()
        .success();

    // Delete a photo and run update
    let photo_to_delete = library_dir.child("2024/05-17").child("IMG_8040.HEIC");
    assert!(photo_to_delete.exists());
    std::fs::remove_file(photo_to_delete.path()).unwrap();

    let mut update_cmd = Command::cargo_bin("photosort").unwrap();
    update_cmd
        .arg("update")
        .arg(library_dir.path())
        .assert()
        .success()
        // After deleting one photo, there should be 4 left.
        .stdout(predicate::str::contains(
            "Update complete. Library now has 4 photos.",
        ));
}

#[test]
fn test_sync_command() {
    let temp_dir = assert_fs::TempDir::new().unwrap();
    let source_library_dir = setup_test_library(&temp_dir);
    let target_library_dir = temp_dir.child("target_library");

    let photo_dir = get_test_photos_dir();

    // Import photos into the source library
    let mut import_cmd = Command::cargo_bin("photosort").unwrap();
    import_cmd
        .arg("import")
        .arg(photo_dir)
        .arg(source_library_dir.path())
        .assert()
        .success();

    // Create the target library
    let mut create_cmd = Command::cargo_bin("photosort").unwrap();
    create_cmd
        .arg("create")
        .arg(target_library_dir.path())
        .assert()
        .success();

    // Sync the libraries
    let mut sync_cmd = Command::cargo_bin("photosort").unwrap();
    sync_cmd
        .arg("sync")
        .arg(source_library_dir.path())
        .arg(target_library_dir.path())
        .assert()
        .success()
        // Target library should have 5 photos after the sync.
        .stdout(predicate::str::contains(
            "Sync complete. Target library now has 5 photos.",
        ));

    // Verify that files were copied to the target library
    assert!(
        target_library_dir
            .child("2024/05-19")
            .child("IMG_E8109.JPG")
            .exists()
    );
    assert!(
        target_library_dir
            .child("2024/05-22")
            .child("_DSCE7023.JPG")
            .exists()
    );
}
