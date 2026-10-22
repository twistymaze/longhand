//! Apps whose audio Longhand records (app audio, KTD1, KTD2).
//!
//! An app is identified by an ID that stays the same when the app quits
//! and starts again:
//!
//! - macOS: the bundle ID of the outermost `.app` that holds the process.
//!   A helper inside a bundle (Chrome Helper, the Teams and Zoom helpers)
//!   gets the ID of the app that holds it.
//! - Windows: the executable name in lowercase, without `.exe`.
//!
//! [`app_of`] maps one audio process to an app ID. Some system processes
//! play the sound of an app that does not hold them; fixed rules map them
//! (FaceTime, WebKit). The mapping of the outermost app and of the FaceTime
//! processes is from anarlog's `crates/detect/src/list/macos.rs` (see
//! PROVENANCE.md).

/// An app on the app list or in a note's app set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct App {
    /// The ID (see the module documentation).
    pub id: String,
    /// The name that the user sees.
    pub name: String,
    /// The path of the app bundle (macOS) or the executable (Windows).
    /// Longhand reads the icon from it, also when the app does not run.
    pub path: String,
}

/// What the audio system knows about one process that can play audio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessInfo {
    /// The macOS Core Audio process object. 0 on Windows.
    pub object: u32,
    pub pid: u32,
    /// The process's own bundle ID (macOS) or app ID (Windows).
    pub bundle_id: Option<String>,
    /// The ID of the outermost app that holds the process (macOS), or the
    /// app ID of the process's root executable (Windows).
    pub outer_app_id: Option<String>,
    /// The ID of the app that is responsible for the process, when the
    /// system knows one (macOS WebKit processes).
    pub responsible_app_id: Option<String>,
}

/// The app ID of FaceTime.
pub const FACETIME_ID: &str = "com.apple.FaceTime";

/// System processes that play the sound of FaceTime calls (from anarlog's
/// `APPLE_CALL_DAEMON_IDS`).
const FACETIME_PROCESSES: &[&str] = &[
    "com.apple.avconferenced",
    "com.apple.TelephonyUtilities.callservicesd",
];

/// WebKit processes play the web audio of Safari and of other apps that
/// show web pages. They are not inside the app's bundle, so the app that
/// is responsible for them owns them.
const WEBKIT_PREFIX: &str = "com.apple.WebKit.";

/// The app ID that owns the sound of `process`, or `None` when no app is
/// known.
pub fn app_of(process: &ProcessInfo) -> Option<String> {
    let own = process.bundle_id.as_deref();
    if let Some(id) = own
        && FACETIME_PROCESSES.contains(&id)
    {
        return Some(FACETIME_ID.to_owned());
    }
    if let Some(id) = own
        && id.starts_with(WEBKIT_PREFIX)
    {
        return process.responsible_app_id.clone();
    }
    process
        .outer_app_id
        .clone()
        .or_else(|| process.bundle_id.clone())
}

/// The processes that play the sound of the apps with the IDs in `apps`.
/// The process with `own_pid` (Longhand) is never in the result. Each
/// process that maps to no app is logged once, by the caller's `log`
/// (the macOS check runs every 500 ms).
pub fn processes_of<'a>(
    apps: &[String],
    processes: &'a [ProcessInfo],
    own_pid: u32,
) -> Vec<&'a ProcessInfo> {
    processes
        .iter()
        .filter(|p| p.pid != own_pid)
        .filter(|p| app_of(p).is_some_and(|id| apps.contains(&id)))
        .collect()
}

/// The macOS process objects of [`processes_of`], sorted and without
/// duplicates, so two lists of the same processes are equal.
pub fn process_objects(apps: &[String], processes: &[ProcessInfo], own_pid: u32) -> Vec<u32> {
    let mut objects: Vec<u32> = processes_of(apps, processes, own_pid)
        .iter()
        .map(|p| p.object)
        .collect();
    objects.sort_unstable();
    objects.dedup();
    objects
}

/// The outermost `.app` bundle in `path`, for example
/// `/Applications/Google Chrome.app` for a Chrome Helper executable.
pub fn outermost_app(path: &str) -> Option<&str> {
    let end = path
        .find(".app/")
        .map(|i| i + 4)
        .or_else(|| path.ends_with(".app").then_some(path.len()))?;
    Some(&path[..end])
}

/// The Windows app ID of an executable path or name: the file name in
/// lowercase, without `.exe`.
pub fn windows_app_id(path_or_name: &str) -> String {
    let name = path_or_name
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(path_or_name)
        .to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_owned()
}

/// One common call app: its ID on macOS and on Windows (KTD13). Browsers
/// are not in the table, because a browser records every tab.
pub struct CallApp {
    pub macos_id: &'static str,
    pub windows_id: &'static str,
}

/// The common call apps that "Add Call Apps" adds (R3, KTD13).
pub const CALL_APPS: &[CallApp] = &[
    CallApp {
        macos_id: "com.microsoft.teams2",
        windows_id: "ms-teams",
    },
    CallApp {
        macos_id: "us.zoom.xos",
        windows_id: "zoom",
    },
    CallApp {
        macos_id: "com.hnc.Discord",
        windows_id: "discord",
    },
    CallApp {
        macos_id: "com.tinyspeck.slackmacgap",
        windows_id: "slack",
    },
    CallApp {
        macos_id: "Cisco-Systems.Spark",
        windows_id: "webexhost",
    },
    CallApp {
        macos_id: FACETIME_ID,
        windows_id: "",
    },
];

/// The app ID of Teams on Windows (KTD5).
pub const WINDOWS_TEAMS_ID: &str = "ms-teams";

/// Whether `id` is a call app on this platform.
pub fn is_call_app(id: &str) -> bool {
    CALL_APPS.iter().any(|app| {
        let platform_id = if cfg!(target_os = "windows") {
            app.windows_id
        } else {
            app.macos_id
        };
        !platform_id.is_empty() && platform_id == id
    })
}

/// The running call apps that are not in `listed` (R3).
pub fn call_apps_to_add(running: &[App], listed: &[App]) -> Vec<App> {
    running
        .iter()
        .filter(|app| is_call_app(&app.id))
        .filter(|app| !listed.iter().any(|l| l.id == app.id))
        .cloned()
        .collect()
}

/// The first Windows build with per-process loopback capture (KTD4).
pub const FIRST_WINDOWS_BUILD: u32 = 20_348;

/// Whether a Windows build supports app audio (R17).
pub fn windows_build_supported(build: u32) -> bool {
    build >= FIRST_WINDOWS_BUILD
}

/// Whether this computer supports app audio (R17). macOS always does;
/// Windows needs build 20348 or later.
pub fn supported() -> bool {
    #[cfg(target_os = "windows")]
    {
        crate::windows_apps::build_number().is_some_and(windows_build_supported)
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        false
    }
}

/// The running apps that have a window, without Longhand, sorted by name
/// (R2, KTD12).
pub fn running_apps() -> Vec<App> {
    #[cfg(target_os = "macos")]
    let mut apps = macos::running_apps();
    #[cfg(target_os = "windows")]
    let mut apps = crate::windows_apps::running_apps();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut apps: Vec<App> = Vec::new();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The icon of the app at `path` as PNG bytes, or `None` when it cannot be
/// read (the UI then shows a generic icon).
pub fn icon_png(path: &str) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos::icon_png(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

/// The audio processes that exist now, with their app information.
pub fn audio_processes() -> Vec<ProcessInfo> {
    #[cfg(target_os = "macos")]
    {
        macos::audio_processes()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
pub(crate) mod macos {
    use cidre::core_audio as ca;
    use objc2_app_kit::{
        NSApplicationActivationPolicy, NSBitmapImageFileType, NSBitmapImageRep,
        NSRunningApplication, NSWorkspace,
    };
    use objc2_foundation::{NSBundle, NSDictionary, NSSize, NSString};

    use super::{App, ProcessInfo, outermost_app};

    /// The executable path of a process.
    fn exe_path(pid: u32) -> Option<String> {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is valid for its full length.
        let n =
            unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        String::from_utf8(buf).ok()
    }

    /// The bundle ID of the bundle at `path`.
    fn bundle_id_at(path: &str) -> Option<String> {
        let bundle = NSBundle::bundleWithPath(&NSString::from_str(path))?;
        Some(bundle.bundleIdentifier()?.to_string())
    }

    /// The ID of the outermost app that holds the process.
    fn outer_app_id(pid: u32) -> Option<String> {
        let path = exe_path(pid)?;
        bundle_id_at(outermost_app(&path)?)
    }

    type ResponsibleFn = unsafe extern "C" fn(libc::pid_t) -> libc::pid_t;

    /// The private `responsibility_get_pid_responsible_for_pid`, loaded at
    /// run time. `None` when the system does not have it.
    fn responsible_fn() -> Option<ResponsibleFn> {
        static FN: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
        let address = *FN.get_or_init(|| {
            let name = c"responsibility_get_pid_responsible_for_pid";
            // SAFETY: dlsym with RTLD_DEFAULT and a valid C string.
            let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
            if symbol.is_null() {
                log::warn!(
                    "responsibility_get_pid_responsible_for_pid is missing; \
                     WebKit sound maps to no app"
                );
                None
            } else {
                Some(symbol as usize)
            }
        });
        // SAFETY: the symbol has this signature on every macOS that has it.
        address.map(|a| unsafe { std::mem::transmute::<usize, ResponsibleFn>(a) })
    }

    fn responsible_app_id(pid: u32) -> Option<String> {
        let f = responsible_fn()?;
        // SAFETY: the function takes any PID and returns a PID or -1.
        let responsible = unsafe { f(pid as libc::pid_t) };
        if responsible <= 0 || responsible as u32 == pid {
            return None;
        }
        outer_app_id(responsible as u32)
    }

    pub(crate) fn audio_processes() -> Vec<ProcessInfo> {
        let processes = match ca::System::processes() {
            Ok(processes) => processes,
            Err(e) => {
                log::warn!("could not list the audio processes: {e:?}");
                return Vec::new();
            }
        };
        processes
            .iter()
            .filter_map(|p| {
                let pid = u32::try_from(p.pid().ok()?).ok()?;
                let bundle_id = p.bundle_id().ok().map(|id| id.to_string());
                let webkit = bundle_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with(super::WEBKIT_PREFIX));
                Some(ProcessInfo {
                    object: p.0.0,
                    pid,
                    outer_app_id: outer_app_id(pid),
                    responsible_app_id: if webkit {
                        responsible_app_id(pid)
                    } else {
                        None
                    },
                    bundle_id,
                })
            })
            .collect()
    }

    pub(crate) fn running_apps() -> Vec<App> {
        let own = std::process::id() as i32;
        let workspace = NSWorkspace::sharedWorkspace();
        workspace
            .runningApplications()
            .iter()
            .filter(|app| app.activationPolicy() == NSApplicationActivationPolicy::Regular)
            .filter(|app| app.processIdentifier() != own)
            .filter_map(|app| app_info(&app))
            .collect()
    }

    fn app_info(app: &NSRunningApplication) -> Option<App> {
        let id = app.bundleIdentifier()?.to_string();
        let path = app.bundleURL()?.path()?.to_string();
        let name = app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_else(|| id.clone());
        Some(App { id, name, path })
    }

    /// The icon of the app at `path`, 64 by 64 points, as PNG.
    pub(crate) fn icon_png(path: &str) -> Option<Vec<u8>> {
        if !std::path::Path::new(path).exists() {
            return None;
        }
        let workspace = NSWorkspace::sharedWorkspace();
        let image = workspace.iconForFile(&NSString::from_str(path));
        image.setSize(NSSize::new(64.0, 64.0));
        let tiff = image.TIFFRepresentation()?;
        let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
        let properties = NSDictionary::new();
        // SAFETY: an empty property dictionary is valid for PNG.
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        Some(png.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, bundle: &str, outer: Option<&str>) -> ProcessInfo {
        ProcessInfo {
            object: pid + 1000,
            pid,
            bundle_id: Some(bundle.to_owned()),
            outer_app_id: outer.map(str::to_owned),
            responsible_app_id: None,
        }
    }

    #[test]
    fn a_helper_inside_a_bundle_belongs_to_the_outer_app() {
        let helper = process(10, "com.google.Chrome.helper", Some("com.google.Chrome"));
        assert_eq!(app_of(&helper).as_deref(), Some("com.google.Chrome"));
    }

    #[test]
    fn the_facetime_call_service_belongs_to_facetime() {
        let daemon = process(11, "com.apple.avconferenced", None);
        assert_eq!(app_of(&daemon).as_deref(), Some(FACETIME_ID));
        let calls = process(12, "com.apple.TelephonyUtilities.callservicesd", None);
        assert_eq!(app_of(&calls).as_deref(), Some(FACETIME_ID));
    }

    #[test]
    fn the_facetime_service_is_recorded_only_when_facetime_is_in_the_set() {
        let processes = vec![process(11, "com.apple.avconferenced", None)];
        let with = processes_of(&[FACETIME_ID.to_owned()], &processes, 1);
        assert_eq!(with.len(), 1);
        let without = processes_of(&["us.zoom.xos".to_owned()], &processes, 1);
        assert!(without.is_empty());
    }

    #[test]
    fn a_webkit_process_belongs_to_its_responsible_app() {
        let mut safari = process(13, "com.apple.WebKit.GPU", None);
        safari.responsible_app_id = Some("com.apple.Safari".to_owned());
        assert_eq!(app_of(&safari).as_deref(), Some("com.apple.Safari"));
        let mut other = process(14, "com.apple.WebKit.GPU", None);
        other.responsible_app_id = Some("com.example.Reader".to_owned());
        assert_eq!(app_of(&other).as_deref(), Some("com.example.Reader"));
    }

    #[test]
    fn a_webkit_process_with_no_responsible_app_belongs_to_no_app() {
        let orphan = process(15, "com.apple.WebKit.WebContent", Some("com.apple.WebKit"));
        assert_eq!(app_of(&orphan), None);
    }

    #[test]
    fn a_process_with_no_bundle_and_no_app_belongs_to_no_app() {
        let unknown = ProcessInfo {
            object: 1,
            pid: 16,
            ..ProcessInfo::default()
        };
        assert_eq!(app_of(&unknown), None);
    }

    #[test]
    fn two_apps_give_the_union_of_their_processes_without_longhand() {
        let processes = vec![
            process(20, "com.google.Chrome.helper", Some("com.google.Chrome")),
            process(21, "us.zoom.xos", Some("us.zoom.xos")),
            process(22, "com.hnc.Discord", Some("com.hnc.Discord")),
            process(99, "net.ewal.longhand", Some("com.google.Chrome")),
        ];
        let apps = ["com.google.Chrome".to_owned(), "us.zoom.xos".to_owned()];
        let pids: Vec<u32> = processes_of(&apps, &processes, 99)
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(pids, vec![20, 21]);
    }

    #[test]
    fn process_objects_change_when_a_process_starts_or_quits_but_not_with_order() {
        let apps = ["us.zoom.xos".to_owned(), "com.hnc.Discord".to_owned()];
        let zoom = process(21, "us.zoom.xos", Some("us.zoom.xos"));
        let discord = process(22, "com.hnc.Discord", Some("com.hnc.Discord"));
        let one = process_objects(&apps, std::slice::from_ref(&zoom), 99);
        let both = process_objects(&apps, &[zoom.clone(), discord.clone()], 99);
        let reversed = process_objects(&apps, &[discord, zoom], 99);
        assert_eq!(one, vec![1021]);
        assert_eq!(both, vec![1021, 1022]);
        assert_ne!(one, both, "a process that starts is a change");
        assert_eq!(both, reversed, "the order of the processes is not a change");
        assert!(process_objects(&apps, &[], 99).is_empty());
    }

    #[test]
    fn the_outermost_app_of_a_nested_helper() {
        let path = "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/1/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper";
        assert_eq!(outermost_app(path), Some("/Applications/Google Chrome.app"));
        assert_eq!(
            outermost_app("/Applications/Zoom.app"),
            Some("/Applications/Zoom.app")
        );
        assert_eq!(outermost_app("/usr/sbin/coreaudiod"), None);
    }

    #[test]
    fn a_windows_executable_gives_its_lowercase_name() {
        assert_eq!(windows_app_id("MS-Teams.exe"), "ms-teams");
        assert_eq!(
            windows_app_id(r"C:\Program Files\Zoom\bin\Zoom.exe"),
            "zoom"
        );
    }

    fn app(id: &str) -> App {
        App {
            id: id.to_owned(),
            name: id.to_owned(),
            path: String::new(),
        }
    }

    #[test]
    fn call_apps_to_add_are_running_call_apps_not_on_the_list() {
        let teams = if cfg!(target_os = "windows") {
            "ms-teams"
        } else {
            "com.microsoft.teams2"
        };
        let zoom = if cfg!(target_os = "windows") {
            "zoom"
        } else {
            "us.zoom.xos"
        };
        let running = vec![app(teams), app(zoom), app("com.google.Chrome")];
        let listed = vec![app(zoom)];
        let added: Vec<String> = call_apps_to_add(&running, &listed)
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(added, vec![teams.to_owned()]);
        assert!(call_apps_to_add(&[app("com.google.Chrome")], &[]).is_empty());
    }

    #[test]
    fn windows_build_20348_and_later_support_app_audio() {
        assert!(windows_build_supported(20_348));
        assert!(windows_build_supported(26_200));
        assert!(!windows_build_supported(19_045));
    }
}
