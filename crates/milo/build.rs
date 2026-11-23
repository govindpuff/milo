use std::process::Command;

fn main() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    let out = Command::new("clang")
        .arg("--print-runtime-dir")
        .output()
        .expect("clang not found; run ./setup.sh to install the Xcode Command Line Tools");
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
