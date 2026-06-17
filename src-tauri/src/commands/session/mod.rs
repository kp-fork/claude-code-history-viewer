//! Session commands module
//!
//! This module contains all session-related Tauri commands organized into submodules:
//! - `load`: Session and message loading functions
//! - `search`: Message search functions
//! - `edits`: File edit tracking and restore functions
//! - `rename`: Native session renaming functions
//! - `delete`: Session deletion

mod delete;
mod edits;
mod load;
mod rename;
mod search;

// Re-export all commands
pub use delete::*;
pub use edits::*;
pub use load::*;
pub use rename::*;
pub use search::*;

/// Reject session file paths that fall outside the on-disk roots used by
/// the supported providers. Defends `WebUI` handlers (which accept untrusted
/// HTTP input) against being pointed at arbitrary `.jsonl` files on the host.
///
/// Desktop builds do not need this guard — those paths flow from
/// `scan_projects` / `load_sessions` output, never raw user input.
#[cfg(feature = "webui-server")]
pub(crate) fn is_safe_session_path(path: &std::path::Path) -> Result<(), String> {
    use std::path::PathBuf;

    fn strip_windows_prefix(p: &std::path::Path) -> PathBuf {
        let s = p.to_string_lossy();
        s.strip_prefix(r"\\?\")
            .map(PathBuf::from)
            .unwrap_or_else(|| p.to_path_buf())
    }

    let home_raw = dirs::home_dir().ok_or("Could not find home directory")?;
    let home = home_raw.canonicalize().unwrap_or_else(|_| home_raw.clone());
    let home = strip_windows_prefix(&home);

    let allowed: [PathBuf; 7] = [
        home.join(".claude").join("projects"),
        home.join(".codex").join("sessions"),
        home.join(".gemini"),
        home.join(".local").join("share").join("opencode"),
        home.join(".cline").join("tasks"),
        home.join(".cursor"),
        home.join(".codebuddy").join("projects"),
    ];

    // Canonicalize each allowlist entry so the comparison below is like-for-like
    // with the canonicalized candidate. Without this, a symlinked provider root
    // (e.g. `~/.claude -> ~/.claude-store`, common in container / persistent-volume
    // setups) makes the candidate resolve to the symlink target while the literal
    // allowlist entry does not — so `starts_with` fails and valid sessions are
    // wrongly rejected (#355). Entries that do not exist fall back to the literal
    // path, preserving the confinement guarantee for unused provider roots.
    let allowed: Vec<PathBuf> = allowed
        .into_iter()
        .map(|d| {
            let resolved = d.canonicalize().unwrap_or(d);
            strip_windows_prefix(&resolved)
        })
        .collect();

    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| format!("Path canonicalization error: {e}"))?
    } else {
        path.parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(path.file_name().unwrap_or_default()))
            .ok_or_else(|| "Invalid path".to_string())?
    };
    let canonical = strip_windows_prefix(&canonical);

    if allowed.iter().any(|d| canonical.starts_with(d)) {
        Ok(())
    } else {
        Err("Session path not in allowed provider directories".to_string())
    }
}

#[cfg(all(test, feature = "webui-server", unix))]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::os::unix::fs::symlink;

    /// Run `body` with `$HOME` temporarily pointed at `home`, restoring it after.
    /// Serialized because `is_safe_session_path` resolves the home dir from the
    /// process environment (other suites also override `HOME`).
    fn with_home<T>(home: &std::path::Path, body: impl FnOnce() -> T) -> T {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", home);
        let out = body();
        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    // Regression test for #355: when `~/.claude` is itself a symlink, the
    // candidate path canonicalizes to the symlink target, so the allowlist
    // entries must be canonicalized too or valid sessions are rejected.
    #[test]
    #[serial]
    fn accepts_session_under_symlinked_claude_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path();
        let store_projects = home.join(".claude-store").join("projects").join("proj");
        std::fs::create_dir_all(&store_projects).expect("mk store");
        symlink(home.join(".claude-store"), home.join(".claude")).expect("symlink .claude");
        let session = store_projects.join("session.jsonl");
        std::fs::write(&session, b"{}").expect("write session");

        // Access via the symlinked path; canonicalize() resolves it to the store.
        let via_symlink = home
            .join(".claude")
            .join("projects")
            .join("proj")
            .join("session.jsonl");
        let res = with_home(home, || is_safe_session_path(&via_symlink));
        assert!(
            res.is_ok(),
            "session under a symlinked .claude root should be allowed: {res:?}"
        );
    }

    #[test]
    #[serial]
    fn rejects_session_outside_allowlist() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude").join("projects")).expect("mk claude");
        let outside = home.join("not-a-provider");
        std::fs::create_dir_all(&outside).expect("mk outside");
        let session = outside.join("session.jsonl");
        std::fs::write(&session, b"{}").expect("write session");

        let res = with_home(home, || is_safe_session_path(&session));
        assert!(
            res.is_err(),
            "a path outside every provider root must be rejected"
        );
    }
}
