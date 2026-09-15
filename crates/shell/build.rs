use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    pkg_config::Config::new()
        .atleast_version("1.24")
        .probe("libnm")
        .expect("libnm development files are required");
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for path in [
        "gresources.xml",
        "data/theme/way-shell-dark.css",
        "data/theme/way-shell-light.css",
        "data/org.ldelossa.way-shell.gschema.xml",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    assert!(
        Command::new("glib-compile-resources")
            .current_dir(&root)
            .arg("gresources.xml")
            .arg("--target")
            .arg(out.join("way-shell.gresource"))
            .status()
            .expect("glib-compile-resources is required")
            .success(),
        "resource compilation failed"
    );
    // Test-only schema source: no process-wide environment changes or dconf writes.
    let schemas = out.join("test-schemas");
    fs::create_dir_all(&schemas).unwrap();
    fs::copy(
        root.join("data/org.ldelossa.way-shell.gschema.xml"),
        schemas.join("org.ldelossa.way-shell.gschema.xml"),
    )
    .unwrap();
    assert!(
        Command::new("glib-compile-schemas")
            .arg("--strict")
            .arg(&schemas)
            .status()
            .expect("glib-compile-schemas is required")
            .success(),
        "schema compilation failed"
    );
    println!(
        "cargo:rustc-env=WAY_SHELL_TEST_SCHEMAS={}",
        schemas.display()
    );
}
