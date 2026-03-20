/*
 * @Author             : Felix
 * @Email              : 307253927@qq.com
 * @Date               : 2026-03-20 17:32:37
 * @LastEditors        : Felix
 * @LastEditTime       : 2026-03-20 17:47:44
 */
pub use crate::config::openfang_home_dir;
use std::path::{Path, PathBuf};

pub fn openfang_skills_home_dir() -> PathBuf {
    openfang_home_dir().join("skills")
}

/// Check if the src path starts with the parent path.
/// if the src path starts with the parent path, return true.
/// if the src path does not start with the parent path, return false.
pub fn is_path_start_with_parent<S, P>(src: S, parent: P) -> bool
where
    S: AsRef<Path>,
    P: AsRef<Path>,
{
    let canonical_parent = match parent.as_ref().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };
    let canonical_path = match src.as_ref().canonicalize() {
        Ok(path) => path,
        Err(_) => return false,
    };

    canonical_path.starts_with(canonical_parent)
}

/// Check if the path is in the home directory.
/// if the path is in the home directory, return true.
/// if the path is not exist, return false.
pub fn is_in_home_dir<P: AsRef<Path>>(path: P) -> bool {
    is_path_start_with_parent(path, openfang_home_dir())
}

/// Check if the path is in the skills_home directory.
/// if the path is in the skills_home directory, return true.
/// if the path is not exist, return false.
pub fn is_in_skills_home_dir<P: AsRef<Path>>(path: P) -> bool {
    is_path_start_with_parent(path, openfang_skills_home_dir())
}

#[test]
fn test_is_in_home_dir() {
    let skills_home_dir = openfang_skills_home_dir();
    println!("home_dir: {:?}", skills_home_dir);
    assert!(
        is_in_home_dir(skills_home_dir),
        "Path1 is in home directory"
    );
}
