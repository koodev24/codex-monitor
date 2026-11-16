//! Durable, private file writes.
//!
//! Auth snapshots and usage stores are rewritten while Codex or another app
//! instance may be reading them. A plain `fs::write` can expose a truncated
//! JSON file if the process dies mid-write, and default permissions can leave
//! refresh tokens readable by other users. Every write here goes to a unique
//! sibling temp file, is flushed to disk, then renamed over the target.

#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn ensure_dir(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(path)
}

/// Create an application-owned directory and keep it private to the user.
/// Never call this for a directory the user chose (Desktop, Documents, ...).
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    ensure_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn temp_path(target: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    target.with_file_name(format!(
        ".{name}.{}.{counter}.{nanos}.tmp",
        std::process::id()
    ))
}

/// Follow a symlink at `path` so an atomic replace updates the linked file
/// instead of replacing the link with a regular file. Codex itself follows
/// symlinked credential files, so the monitor must write the same bytes to the
/// same inode.
fn resolve_symlink(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path),
        _ => Ok(path.to_path_buf()),
    }
}

/// True when `path` has more than one hard link. Replacing it atomically would
/// leave the other names pointing at stale credentials, so callers that write
/// the live Codex session refuse instead.
#[cfg(unix)]
pub fn has_multiple_links(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path)
        .map(|meta| meta.nlink() > 1)
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn has_multiple_links(_path: &Path) -> bool {
    false
}

fn sync_parent(parent: &Path) {
    #[cfg(unix)]
    {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = parent;
}

/// Write `bytes` to `path` atomically. On Unix the temp file is created with
/// `mode` (0600 for credentials), so the data is never briefly world-readable.
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let resolved = resolve_symlink(path)?;
    let path = resolved.as_path();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    ensure_dir(parent)?;
    let tmp = temp_path(path);
    let result = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        sync_parent(parent);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn copy_atomic(source: &Path, target: &Path, mode: u32) -> io::Result<()> {
    let bytes = fs::read(source)?;
    write_atomic(target, &bytes, mode)
}
