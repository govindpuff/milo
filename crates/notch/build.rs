use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const TOOLCHAIN: &str = "If this is a toolchain error: MiloNotch needs Swift 6.2 or newer (Xcode 26 or the Command Line Tools, xcode-select --install). If the repository was moved, delete swift/MiloNotch/.build and build again.";

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("failed to run {cmd:?}: {e}\n{TOOLCHAIN}"));
    if !out.status.success() {
        panic!(
            "{cmd:?} failed\n{}\n{}\n{TOOLCHAIN}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn swift_build(pkg: &Path) -> Command {
    let mut cmd = Command::new("swift");
    cmd.args(["build", "-c", "release", "--product", "MiloNotch", "--package-path"])
        .arg(pkg)
        .env_remove("MACOSX_DEPLOYMENT_TARGET")
        .env_remove("SDKROOT");
    cmd
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let pkg = manifest.join("../../swift/MiloNotch").canonicalize().expect("swift/MiloNotch not found");
    println!("cargo:rerun-if-changed={}", pkg.join("Package.swift").display());
    println!("cargo:rerun-if-changed={}", pkg.join("Package.resolved").display());
    println!("cargo:rerun-if-changed={}", pkg.join("Sources").display());

    run(&mut swift_build(&pkg));
    let bin = run(swift_build(&pkg).arg("--show-bin-path"));

    let sdk = run(Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]));
    let swiftc = PathBuf::from(run(Command::new("xcrun").args(["--find", "swiftc"])));
    let toolchain_lib = swiftc.parent().unwrap().parent().unwrap().join("lib/swift/macosx");

    println!("cargo:rustc-link-search=native={bin}");
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-search=native={}", toolchain_lib.display());
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=static:+whole-archive=MiloNotch");
    for fw in ["AppKit", "SwiftUI", "Carbon", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={fw}");
    }
}
