//! Windows: the running apps with a window, the process table, and the
//! Windows build number (R2, R17, KTD4, KTD12).

use ::windows::Wdk::System::SystemServices::RtlGetVersion;
use ::windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use ::windows::Win32::System::SystemInformation::OSVERSIONINFOW;
use ::windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible,
};
use ::windows::core::{BOOL, PWSTR};

use crate::apps::{App, windows_app_id};
use crate::windows_tree::{ProcessEntry, ProcessTable};

/// The Windows build number, or `None` when it cannot be read.
pub(crate) fn build_number() -> Option<u32> {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a valid OSVERSIONINFOW with its size set.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() {
        Some(info.dwBuildNumber)
    } else {
        log::warn!("RtlGetVersion failed: {status:?}");
        None
    }
}

/// The full executable path of a process.
pub(crate) fn exe_path(pid: u32) -> Option<String> {
    // SAFETY: a query-only handle; it is closed below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer and length are valid.
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    };
    // SAFETY: the handle came from OpenProcess.
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// All running processes, with their parents and app IDs (KTD4, KTD5).
/// An empty table when the snapshot fails.
pub(crate) fn process_table() -> ProcessTable {
    // SAFETY: a process snapshot; the handle is closed below.
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(snapshot) => snapshot,
        Err(e) => {
            log::warn!("the process snapshot failed: {e}");
            return ProcessTable::default();
        }
    };
    let mut entries = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` is a valid PROCESSENTRY32W with its size set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        let len = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        entries.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            app_id: windows_app_id(&String::from_utf16_lossy(&entry.szExeFile[..len])),
        });
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the handle came from CreateToolhelp32Snapshot.
    let _ = unsafe { CloseHandle(snapshot) };
    ProcessTable::new(entries)
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the address of the Vec that `running_apps` owns
    // for the length of EnumWindows.
    let pids = unsafe { &mut *(lparam.0 as *mut Vec<u32>) };
    // SAFETY: hwnd is a top-level window that EnumWindows gives.
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    // A window with an owner is a dialog or a tool window of another one.
    let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|owner| !owner.is_invalid());
    if visible && !owned {
        let mut pid = 0u32;
        // SAFETY: pid is a valid out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid != 0 {
            pids.push(pid);
        }
    }
    BOOL(1)
}

/// The processes that own a visible top-level window, as apps, one entry
/// for each app ID, without Longhand.
pub(crate) fn running_apps() -> Vec<App> {
    let mut pids: Vec<u32> = Vec::new();
    // SAFETY: the callback reads lparam as `&mut Vec<u32>`, which lives
    // until EnumWindows returns.
    if let Err(e) = unsafe { EnumWindows(Some(collect), LPARAM(&mut pids as *mut _ as isize)) } {
        log::warn!("EnumWindows failed: {e}");
    }
    let own = std::process::id();
    let mut apps: Vec<App> = Vec::new();
    for pid in pids {
        if pid == own {
            continue;
        }
        let Some(path) = exe_path(pid) else {
            continue;
        };
        let id = windows_app_id(&path);
        if apps.iter().any(|a| a.id == id) {
            continue;
        }
        let name = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        apps.push(App { id, name, path });
    }
    apps
}
