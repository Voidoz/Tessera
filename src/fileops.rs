//! Filesystem helpers shared by drops, renames, and file actions.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Check that a user-supplied name is a single, valid path component and
/// return it trimmed.
pub fn validated_name(input: &str) -> Result<&str, String> {
    let name = input.trim();
    if name.is_empty() {
        return Err("The name cannot be empty.".to_owned());
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err("That is not a valid file name.".to_owned());
    }
    Ok(name)
}

/// Pick a name in `directory` that does not collide, adding ` (1)`, ` (2)`, …
pub fn unique_destination(directory: &Path, name: &OsStr) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..10_000 {
        let candidate = directory.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem} (copy){extension}"))
}

/// Copy a file, a directory tree, or a symlink.
pub fn copy_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source)?;
        std::os::unix::fs::symlink(target, destination)
    } else if metadata.is_dir() {
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, destination).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tessera-fileops-{name}-{unique}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn validation_accepts_plain_names_and_trims_space() {
        assert_eq!(validated_name("  notes.txt  "), Ok("notes.txt"));
        assert_eq!(validated_name("a.txt"), Ok("a.txt"));
    }

    #[test]
    fn validation_rejects_empty_or_path_like_names() {
        assert_eq!(
            validated_name("   "),
            Err("The name cannot be empty.".to_owned())
        );
        for name in [".", "..", "a/b", "a\0b"] {
            assert_eq!(
                validated_name(name),
                Err("That is not a valid file name.".to_owned())
            );
        }
    }

    #[test]
    fn destination_gets_a_numbered_suffix_on_collision() {
        let dir = temp_dir("collide");
        std::fs::write(dir.join("note.txt"), "one").expect("write");
        std::fs::write(dir.join("note (1).txt"), "two").expect("write");

        let destination = unique_destination(&dir, OsStr::new("note.txt"));
        assert_eq!(destination, dir.join("note (2).txt"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn copies_files_and_directory_trees() {
        let source = temp_dir("source");
        let nested = source.join("folder");
        std::fs::create_dir(&nested).expect("create subfolder");
        std::fs::write(source.join("file.txt"), "hello").expect("write file");
        std::fs::write(nested.join("inner.txt"), "inner").expect("write inner");

        let destination = temp_dir("destination").join("copy");
        copy_path(&source, &destination).expect("copy");

        assert_eq!(
            std::fs::read_to_string(destination.join("file.txt")).expect("read"),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("folder/inner.txt")).expect("read"),
            "inner"
        );

        let _ = std::fs::remove_dir_all(source);
        let _ = std::fs::remove_dir_all(destination.parent().expect("parent"));
    }
}
