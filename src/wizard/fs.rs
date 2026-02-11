use std::fs;
use std::path::Path;

pub(super) fn create_dir_all(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|err| format!("Failed to create directory {}: {}", path.display(), err))
}

pub(super) fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map(|_| ()).map_err(|err| {
        format!(
            "Failed to copy {} -> {}: {}",
            from.display(),
            to.display(),
            err
        )
    })
}

pub(super) fn write(path: &Path, content: &str) -> Result<(), String> {
    fs::write(path, content).map_err(|err| format!("Failed to write {}: {}", path.display(), err))
}

pub(super) fn read_to_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|err| format!("Failed to read {}: {}", path.display(), err))
}

pub(super) fn exists(path: &Path) -> bool {
    path.exists()
}

pub(super) fn is_file(path: &Path) -> bool {
    path.is_file()
}
