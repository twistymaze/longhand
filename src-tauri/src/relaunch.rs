//! The restart after an update install.
//!
//! On macOS, tauri's restart runs `open -n` while this process still runs.
//! The new process is then a second instance, and the Dock gives it a second
//! tile: the tile that the user keeps in the Dock stays with this process,
//! which quits. So on macOS a helper process starts the new build only after
//! this process ends, and the new build gets the kept tile. Other platforms
//! use tauri's restart.

#[cfg(any(target_os = "macos", test))]
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", all(test, unix)))]
use std::{
    ffi::OsString,
    process::{Command, Stdio},
};

use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;

/// The helper script. `read` returns at end of file: the kernel closes the
/// write end of the pipe when this process ends. A zombie or a process that
/// is not yet reaped still answers `kill -0`, so the loop waits up to 5 s
/// for it to go. Then `open` starts the bundle. `-n` prevents an activation
/// of an instance that macOS has not removed yet. If `open` fails, the
/// helper starts the app binary directly, as tauri's restart does.
#[cfg(any(target_os = "macos", all(test, unix)))]
const HELPER_SCRIPT: &str = r#"read _
i=0
while kill -0 "$1" 2>/dev/null && [ "$i" -lt 50 ]; do
  sleep 0.1
  i=$((i + 1))
done
opener="$2"
bundle="$3"
binary="$4"
shift 4
"$opener" -n "$bundle" --args "$@" || exec "$binary" "$@"
"#;

/// The bundle that holds `binary`: `X.app` for `X.app/Contents/MacOS/X`.
/// `None` when the binary is not in a bundle, as in `tauri dev`.
#[cfg(any(target_os = "macos", test))]
fn bundle_root(binary: &Path) -> Option<PathBuf> {
    let macos_dir = binary.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    bundle
        .extension()
        .is_some_and(|ext| ext == "app")
        .then(|| bundle.to_path_buf())
}

/// The helper command: `opener` starts `bundle` with `args` after the
/// process `pid` ends and the helper's standard input closes. If `opener`
/// fails, `binary` starts with `args`.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn helper_command(
    opener: &Path,
    pid: u32,
    bundle: &Path,
    binary: &Path,
    args: &[OsString],
) -> Command {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(HELPER_SCRIPT)
        .arg("longhand-relaunch")
        .arg(pid.to_string())
        .arg(opener)
        .arg(bundle)
        .arg(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // A signal to the app's process group does not stop the helper.
        .process_group(0);
    command
}

/// Start the helper. `false` when the app is not in a bundle or the helper
/// cannot start.
#[cfg(target_os = "macos")]
fn start_helper(app: &AppHandle) -> bool {
    let env = app.env();
    let binary = match tauri::process::current_binary(&env) {
        Ok(binary) => binary,
        Err(e) => {
            log::error!("relaunch: cannot find the app binary: {e}");
            return false;
        }
    };
    let Some(bundle) = bundle_root(&binary) else {
        log::warn!("relaunch: {} is not in a bundle", binary.display());
        return false;
    };
    let args: Vec<_> = env.args_os.iter().skip(1).cloned().collect();
    let mut command = helper_command(
        Path::new("/usr/bin/open"),
        std::process::id(),
        &bundle,
        &binary,
        &args,
    );
    match command.spawn() {
        Ok(mut child) => {
            // The pipe stays open until this process ends: then the kernel
            // closes it, and the helper continues.
            if let Some(pipe) = child.stdin.take() {
                std::mem::forget(pipe);
            }
            true
        }
        Err(e) => {
            log::error!("relaunch: cannot start the helper: {e}");
            false
        }
    }
}

/// Quit through the event loop (`RunEvent::Exit` runs), then start the app
/// again.
pub fn request_relaunch(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    if start_helper(app) {
        app.exit(0);
        return;
    }
    app.request_restart();
}

/// Quit now without the event loop, then start the app again. For the main
/// thread before the event loop runs.
pub fn relaunch_now(app: &AppHandle) -> ! {
    #[cfg(target_os = "macos")]
    if start_helper(app) {
        app.cleanup_before_exit();
        std::process::exit(0);
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_root_finds_the_app_bundle_of_its_binary() {
        assert_eq!(
            bundle_root(Path::new(
                "/Applications/Longhand.app/Contents/MacOS/longhand"
            )),
            Some(PathBuf::from("/Applications/Longhand.app"))
        );
    }

    #[test]
    fn bundle_root_is_none_for_a_binary_outside_a_bundle() {
        assert_eq!(bundle_root(Path::new("/repo/target/debug/longhand")), None);
        assert_eq!(bundle_root(Path::new("/x/Contents/MacOS/longhand")), None);
        assert_eq!(bundle_root(Path::new("/x/Y.app/MacOS/longhand")), None);
    }

    /// A test script that writes its arguments to `record`, then exits with
    /// `status`. While the `early` file exists, it writes `early` instead.
    #[cfg(unix)]
    fn recording_script(dir: &Path, name: &str, early: &Path, status: i32) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let record = dir.join(format!("{name}.txt"));
        let script = dir.join(format!("{name}.sh"));
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ -e '{early}' ]; then echo early > '{record}'; \
                 else printf '%s\\n' \"$@\" > '{record}'; fi\nexit {status}\n",
                early = early.display(),
                record = record.display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (script, record)
    }

    /// Which condition the test releases first.
    #[cfg(unix)]
    enum Release {
        PipeFirst,
        ProcessFirst,
    }

    /// Run the helper against an old process that this test ends. The
    /// `early` file exists until the test releases the second condition, so
    /// a script that runs before then records `early`.
    #[cfg(unix)]
    fn run_helper(release: Release, opener_status: i32) -> (Option<String>, Option<String>) {
        let dir = tempfile::tempdir().unwrap();
        let early = dir.path().join("early");
        std::fs::write(&early, "").unwrap();
        let (opener, opened) = recording_script(dir.path(), "opener", &early, opener_status);
        let (binary, started) = recording_script(dir.path(), "binary", &early, 0);
        let mut old = std::process::Command::new("/bin/sleep")
            .arg("600")
            .spawn()
            .unwrap();
        let bundle = Path::new("/Applications/Longhand.app");
        let args = ["--longhand-login".into()];
        let mut helper = helper_command(&opener, old.id(), bundle, &binary, &args)
            .spawn()
            .unwrap();
        let pipe = helper.stdin.take().unwrap();

        let mut end_old = || {
            old.kill().unwrap();
            old.wait().unwrap();
        };
        match release {
            Release::PipeFirst => {
                drop(pipe);
                std::fs::remove_file(&early).unwrap();
                end_old();
            }
            Release::ProcessFirst => {
                end_old();
                std::fs::remove_file(&early).unwrap();
                drop(pipe);
            }
        }
        helper.wait().unwrap();
        let read = |path: &Path| std::fs::read_to_string(path).ok();
        (read(&opened), read(&started))
    }

    #[cfg(unix)]
    const OPEN_ARGS: &str = "-n\n/Applications/Longhand.app\n--args\n--longhand-login\n";

    #[cfg(unix)]
    #[test]
    fn the_helper_waits_for_the_old_process_after_the_pipe_closes() {
        let (opened, started) = run_helper(Release::PipeFirst, 0);
        assert_eq!(opened.as_deref(), Some(OPEN_ARGS));
        assert_eq!(started, None);
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_waits_for_the_pipe_after_the_old_process_ends() {
        let (opened, started) = run_helper(Release::ProcessFirst, 0);
        assert_eq!(opened.as_deref(), Some(OPEN_ARGS));
        assert_eq!(started, None);
    }

    #[cfg(unix)]
    #[test]
    fn the_helper_starts_the_binary_when_open_fails() {
        let (opened, started) = run_helper(Release::ProcessFirst, 1);
        assert_eq!(opened.as_deref(), Some(OPEN_ARGS));
        assert_eq!(started.as_deref(), Some("--longhand-login\n"));
    }
}
