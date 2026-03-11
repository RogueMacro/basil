use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const BUILD_DIR: &str = "build";
pub const STDLIB_DIR: &str = "stdlib";
pub const STDLIB_FILE: &str = "lib.bl";

pub fn target_mod(module: impl AsRef<Path>) -> Result<PathBuf, io::Error> {
    let target_dir = Path::new(BUILD_DIR);
    if !target_dir.exists() {
        fs::create_dir(target_dir)?;
    }

    Ok(target_dir.join(module.as_ref()))
}

pub fn stdlib() -> PathBuf {
    PathBuf::from_iter([STDLIB_DIR, STDLIB_FILE])
}
