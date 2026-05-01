use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("xdeb-ng-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn make_deb(root: &Path, package: &str) -> PathBuf {
    let build = root.join("build");
    fs::create_dir_all(build.join("data/usr/bin")).unwrap();
    fs::write(build.join("data/usr/bin/test-tool"), b"#!/bin/sh\nexit 0\n").unwrap();
    fs::write(
        build.join("control"),
        format!(
            "Package: {package}\nVersion: 1.0-1\nArchitecture: all\nMaintainer: test <test@example.invalid>\nDescription: test package\n long description\n"
        ),
    )
    .unwrap();

    assert!(Command::new("tar")
        .arg("-cf")
        .arg("control.tar")
        .arg("control")
        .current_dir(&build)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("tar")
        .arg("-cf")
        .arg(build.join("data.tar"))
        .arg(".")
        .current_dir(build.join("data"))
        .status()
        .unwrap()
        .success());

    let deb = root.join(format!("{package}.deb"));
    assert!(Command::new("ar")
        .arg("-rc")
        .arg(&deb)
        .arg("control.tar")
        .arg("data.tar")
        .current_dir(&build)
        .status()
        .unwrap()
        .success());
    deb
}

fn make_deb_with_unsafe_symlink(root: &Path) -> PathBuf {
    let build = root.join("unsafe-build");
    fs::create_dir_all(build.join("data/usr/bin")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", build.join("data/usr/bin/bad-link")).unwrap();
    fs::write(
        build.join("control"),
        "Package: unsafe-symlink\nVersion: 1\nArchitecture: all\nMaintainer: test\nDescription: test\n",
    )
    .unwrap();
    assert!(Command::new("tar")
        .arg("-cf")
        .arg("control.tar")
        .arg("control")
        .current_dir(&build)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("tar")
        .arg("-cf")
        .arg(build.join("data.tar"))
        .arg(".")
        .current_dir(build.join("data"))
        .status()
        .unwrap()
        .success());
    let deb = root.join("unsafe-symlink.deb");
    assert!(Command::new("ar")
        .arg("-rc")
        .arg(&deb)
        .arg("control.tar")
        .arg("data.tar")
        .current_dir(&build)
        .status()
        .unwrap()
        .success());
    deb
}

#[test]
fn info_prints_package_metadata() {
    let root = temp_dir("info");
    let deb = make_deb(&root, "metadata-preview");
    let output = Command::new(env!("CARGO_BIN_EXE_xdeb"))
        .arg("info")
        .arg(&deb)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("name: metadata-preview"));
    assert!(stdout.contains("arch: noarch"));
}

#[test]
fn convert_dry_run_does_not_build_xbps() {
    let root = temp_dir("dry-run");
    let deb = make_deb(&root, "dry-run-package");
    let pkgroot = root.join("pkgroot");
    let output = Command::new(env!("CARGO_BIN_EXE_xdeb"))
        .env("XDEB_PKGROOT", &pkgroot)
        .arg("convert")
        .arg("--dry-run")
        .arg(&deb)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!pkgroot
        .join("binpkgs/dry-run-package-1.0.1_1.noarch.xbps")
        .exists());
}

#[test]
fn info_json_prints_machine_readable_metadata() {
    let root = temp_dir("json");
    let deb = make_deb(&root, "json-preview");
    let output = Command::new(env!("CARGO_BIN_EXE_xdeb"))
        .arg("info")
        .arg("--json")
        .arg(&deb)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"name\": \"json-preview\""));
}

#[test]
fn unsafe_symlink_is_rejected() {
    let root = temp_dir("unsafe-symlink");
    let deb = make_deb_with_unsafe_symlink(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_xdeb"))
        .arg("info")
        .arg(&deb)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsafe symlink"));
}

#[test]
fn invalid_package_name_is_rejected() {
    let root = temp_dir("bad-name");
    let deb = make_deb(&root, "bad name");
    let output = Command::new(env!("CARGO_BIN_EXE_xdeb"))
        .arg("info")
        .arg(&deb)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid package name"));
}
