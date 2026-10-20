use std::fs;
use std::io;
use std::path::Path;

/// Publish a complete copy before any loader, editor, logger or watcher uses it.
/// An existing destination is the marker that initialization already happened.
pub(super) fn prepare(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.try_exists()? {
        return Ok(());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    if source.try_exists()?
        && fs::canonicalize(parent)?.starts_with(fs::canonicalize(source)?)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "tacterm configuration directory must not be inside the Rio import source",
        ));
    }
    let staging = tempfile::Builder::new()
        .prefix(".tacterm-import-")
        .tempdir_in(parent)?;
    if source.try_exists()? {
        copy_directory(source, staging.path(), true)?;
    }
    // Another process may have initialized the destination while we copied.
    if destination.try_exists()? {
        return Ok(());
    }
    match fs::rename(staging.path(), destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.try_exists()? => Ok(()),
        Err(error) => Err(error),
    }
}

fn copy_directory(source: &Path, destination: &Path, root: bool) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if root && (entry.file_name() == "log" || entry.file_name() == "install-backups")
        {
            continue;
        }
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            fs::create_dir(&target)?;
            copy_directory(&entry.path(), &target, false)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target)?;
        } else {
            // Do not silently omit linked settings or leave links back into Rio.
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "cannot import linked or special file: {}",
                    entry.path().display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_assets_once_without_touching_rio_or_copying_logs() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("rio");
        let target = root.path().join("tacterm");
        fs::create_dir_all(source.join("themes/nested")).unwrap();
        fs::create_dir(source.join("log")).unwrap();
        fs::create_dir(source.join("install-backups")).unwrap();
        fs::write(source.join("install-backups/old.exe"), "old executable").unwrap();
        fs::write(source.join("config.toml"), "theme = 'custom'").unwrap();
        fs::write(source.join("themes/nested/custom.toml"), "[colors]").unwrap();
        fs::write(source.join("log/rio.log"), "old log").unwrap();
        prepare(&source, &target).unwrap();
        assert_eq!(
            fs::read(target.join("config.toml")).unwrap(),
            fs::read(source.join("config.toml")).unwrap()
        );
        assert!(target.join("themes/nested/custom.toml").is_file());
        assert!(!target.join("log").exists());
        assert!(!target.join("install-backups").exists());
        fs::remove_file(target.join("config.toml")).unwrap();
        prepare(&source, &target).unwrap();
        assert!(!target.join("config.toml").exists());
        assert!(source.join("config.toml").is_file());
    }

    #[test]
    fn existing_empty_directory_prevents_import() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("tacterm");
        fs::create_dir(&target).unwrap();
        // Even an unreadable/invalid source is irrelevant after initialization.
        let source = root.path().join("rio");
        fs::write(&source, "not a directory").unwrap();
        prepare(&source, &target).unwrap();
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }

    #[test]
    fn missing_source_initializes_independent_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("rio");
        let target = root.path().join("nested/tacterm");
        prepare(&source, &target).unwrap();
        assert!(target.is_dir());
        fs::create_dir(&source).unwrap();
        fs::write(source.join("config.toml"), "theme = 'later'").unwrap();
        prepare(&source, &target).unwrap();
        assert!(!target.join("config.toml").exists());
    }

    #[test]
    fn failed_import_leaves_no_destination_and_can_be_retried() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("rio");
        let target = root.path().join("tacterm");
        fs::write(&source, "not a directory").unwrap();
        assert!(prepare(&source, &target).is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        fs::remove_file(&source).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(source.join("config.toml"), "theme = 'retry'").unwrap();
        prepare(&source, &target).unwrap();
        assert!(target.join("config.toml").is_file());
    }

    #[test]
    fn rejects_destination_inside_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("rio");
        fs::create_dir(&source).unwrap();
        assert!(prepare(&source, &source.join("tacterm")).is_err());
        assert_eq!(fs::read_dir(source).unwrap().count(), 0);
    }
}
