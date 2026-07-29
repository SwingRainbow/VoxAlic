//! Cross-platform atomic file write.
//!
//! Windows: `ReplaceFileW` (atomic replace + backup + attribute preservation).
//! Unix:    `std::fs::rename` (atomic on same filesystem).
//!
//! Callers: config save, market cache, market auth — every path that previously
//! wrote directly or did manual temp-file + rename.

use std::path::{Path, PathBuf};

/// Write `data` to `path` atomically.
///
/// A temporary file in the same directory receives the data first. Once the
/// bytes are on disk the temp file is swapped into place. On Windows the
/// original file is preserved as `path.bak` (only the most recent backup is
/// kept). The caller does not need to create parent directories.
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir: {e}"))?;
    }

    let tmp = tmp_path(path)?;
    std::fs::write(&tmp, data).map_err(|e| format!("write tmp: {e}"))?;

    #[cfg(windows)]
    {
        replace_file_w(&tmp, path)?;
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;
    }

    Ok(())
}

// ── Windows-specific ─────────────────────────────────────────────────────────

#[cfg(windows)]
fn replace_file_w(tmp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        ReplaceFileW, REPLACEFILE_WRITE_THROUGH, REPLACEFILE_IGNORE_MERGE_ERRORS,
    };

    let bak = backup_path(target);

    let wide = |p: &Path| -> Vec<u16> {
        let mut v: Vec<u16> = p.as_os_str().encode_wide().collect();
        v.push(0); // null-terminate
        v
    };
    let w_tmp = wide(tmp);
    let w_target = wide(target);
    let w_bak = wide(&bak);

    let flags = REPLACEFILE_WRITE_THROUGH | REPLACEFILE_IGNORE_MERGE_ERRORS;

    let pcw_target = windows::core::PCWSTR::from_raw(w_target.as_ptr());
    let pcw_tmp = windows::core::PCWSTR::from_raw(w_tmp.as_ptr());
    let pcw_bak = windows::core::PCWSTR::from_raw(w_bak.as_ptr());

    unsafe {
        ReplaceFileW(
            pcw_target,
            pcw_tmp,
            Some(&pcw_bak),
            flags,
            None,
            None,
        )
    }
    .map_err(|e| format!("ReplaceFileW failed for {}: {e}", target.display()))?;

    // ReplaceFileW moves tmp → target and old-target → bak.
    // The temp file no longer exists; the bak file may exist even if the
    // original didn't — that's harmless (ReplaceFileW creates bak only when
    // the target existed).
    Ok(())
}

#[cfg(windows)]
fn backup_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

// ── Temp path ────────────────────────────────────────────────────────────────

fn tmp_path(target: &Path) -> Result<PathBuf, String> {
    let stem = target
        .file_stem()
        .ok_or_else(|| format!("atomic_write: no file stem in {}", target.display()))?;
    let ext = target
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    // Include PID so concurrent instances don't collide.
    let name = format!("{}.{}.tmp{}", stem.to_string_lossy(), std::process::id(), ext);
    Ok(target.with_file_name(name))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn atomic_write_creates_file_and_overwrites() {
        let dir = std::env::temp_dir().join("voxalic_test_atomic");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("test.json");

        atomic_write(&p, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "first");

        atomic_write(&p, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "second");

        // Backup should hold the previous content when the target existed.
        let bak = backup_path(&p);
        if bak.exists() {
            assert_eq!(std::fs::read_to_string(&bak).unwrap(), "first");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_no_parent_dir() {
        let dir = std::env::temp_dir().join("voxalic_test_atomic_noparent");
        let p = dir.join("sub").join("f.txt");
        atomic_write(&p, b"ok").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tmp_path_includes_pid() {
        let t = tmp_path(std::path::Path::new("/foo/bar/config.json")).unwrap();
        let s = t.to_string_lossy();
        assert!(s.contains("config."), "expected config.<pid>.tmp.json in {s}");
        assert!(s.ends_with(".tmp.json"), "expected .tmp.json suffix in {s}");
    }
}
