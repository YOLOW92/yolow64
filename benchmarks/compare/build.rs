use std::env;
use std::path::PathBuf;
use std::process::Command;

fn find_dir(env_name: &str, default: &str) -> Option<PathBuf> {
    env::var_os(env_name)
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(default)))
        .filter(|dir| dir.is_dir())
}

fn run_make(dir: &PathBuf, args: &[&str], envs: &[(&str, &str)]) -> bool {
    let mut command = Command::new("make");
    command.args(args).current_dir(dir);
    for (key, value) in envs {
        command.env(key, value);
    }
    match command.status() {
        Ok(status) if status.success() => true,
        Ok(_) => {
            println!("cargo:warning=build failed in {}", dir.display());
            false
        }
        Err(err) => {
            println!(
                "cargo:warning=failed to run make in {}: {err}",
                dir.display()
            );
            false
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for var in ["TURBO_BASE64_DIR", "AKLOMP_BASE64_DIR", "FASTBASE64_DIR"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rustc-check-cfg=cfg(turbo_base64)");
    println!("cargo:rustc-check-cfg=cfg(aklomp_base64)");
    println!("cargo:rustc-check-cfg=cfg(fastbase64)");

    // Turbo-Base64 is GPL-3.0 and is intentionally kept outside the repository.
    if let Some(dir) = find_dir("TURBO_BASE64_DIR", "/tmp/Turbo-Base64")
        && dir.join("makefile").is_file()
        && run_make(&dir, &["-s", "-j8", "libtb64.a"], &[])
    {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static=tb64");
        println!("cargo:rustc-cfg=turbo_base64");
    }

    // aklomp/base64 (BSD-2-Clause). Build the AVX2 codec plus the scalar core.
    if let Some(dir) = find_dir("AKLOMP_BASE64_DIR", "/tmp/base64-aklomp") {
        let flags = [
            ("AVX2_CFLAGS", "-mavx2"),
            ("SSSE3_CFLAGS", "-mssse3"),
            ("SSE41_CFLAGS", "-msse4.1"),
            ("SSE42_CFLAGS", "-msse4.2"),
            ("AVX_CFLAGS", "-mavx"),
        ];
        if dir.join("Makefile").is_file()
            && run_make(&dir, &["-s", "-j8", "lib/libbase64.o"], &flags)
        {
            let archive = dir.join("libbase64-aklomp.a");
            let object = dir.join("lib/libbase64.o");
            let status = Command::new("ar")
                .args(["cr", archive.to_str().unwrap(), object.to_str().unwrap()])
                .status();
            if matches!(status, Ok(s) if s.success()) {
                println!("cargo:rustc-link-search=native={}", dir.display());
                println!("cargo:rustc-link-lib=static=base64-aklomp");
                println!("cargo:rustc-cfg=aklomp_base64");
            }
        }
    }

    // lemire/fastbase64 (BSD). The Makefile builds AVX2 variants with -march=native.
    if let Some(dir) = find_dir("FASTBASE64_DIR", "/tmp/fastbase64-lemire")
        && dir.join("Makefile").is_file()
        && run_make(&dir, &["-s", "-j8", "libfastbase64.a", "SIMD=AVX2"], &[])
    {
        println!("cargo:rustc-link-search=native={}", dir.display());
        println!("cargo:rustc-link-lib=static=fastbase64");
        println!("cargo:rustc-cfg=fastbase64");
    }
}
