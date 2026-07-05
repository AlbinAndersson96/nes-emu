use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set by cargo"));
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out — walk up 3 levels
    // to reach target/<profile>/, where the final binary lands.
    let target_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR must have at least 3 ancestors")
        .to_path_buf();

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo"));
    let src = manifest_dir.join("assets").join("keybindings.toml");
    let dest = target_dir.join("keybindings.toml");

    println!("cargo:rerun-if-changed={}", src.display());

    // Never overwrite a user's edited bindings on rebuild — only seed once.
    if !dest.exists() {
        fs::copy(&src, &dest)
            .expect("failed to copy default keybindings.toml next to the built binary");
    }
}
