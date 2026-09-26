//! Thin Win32 helpers: volume enumeration, cluster size, compressed sizes.

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveKind {
    Fixed,
    Removable,
    Remote,
    CdRom,
    RamDisk,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Drive {
    /// `C:\`
    pub root: String,
    pub label: String,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    pub fn display(&self) -> String {
        let letter = self.root.trim_end_matches('\\');
        let label = if self.label.is_empty() {
            match self.kind {
                DriveKind::Removable => "Removable",
                DriveKind::Remote => "Network",
                DriveKind::CdRom => "CD-ROM",
                DriveKind::RamDisk => "RAM disk",
                DriveKind::Fixed | DriveKind::Unknown => "Local Disk",
            }
        } else {
            self.label.as_str()
        };
        format!("{letter} ({label})")
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// Enumerate logical drives with label, type and capacity.
pub fn list_drives() -> Vec<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    // Values of DRIVE_* from winbase.h.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let wroot = wide(&root);
        let kind = match unsafe { GetDriveTypeW(wroot.as_ptr()) } {
            DRIVE_FIXED => DriveKind::Fixed,
            DRIVE_REMOVABLE => DriveKind::Removable,
            DRIVE_REMOTE => DriveKind::Remote,
            DRIVE_CDROM => DriveKind::CdRom,
            DRIVE_RAMDISK => DriveKind::RamDisk,
            _ => DriveKind::Unknown,
        };
        let mut label_buf = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                wroot.as_ptr(),
                label_buf.as_mut_ptr(),
                label_buf.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        let label = if ok != 0 {
            let len = label_buf.iter().position(|&c| c == 0).unwrap_or(0);
            String::from_utf16_lossy(&label_buf[..len])
        } else {
            // Not ready (empty CD drive etc.) — skip entirely.
            continue;
        };
        let mut free = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        unsafe {
            GetDiskFreeSpaceExW(wroot.as_ptr(), &mut free, &mut total, &mut total_free);
        }
        out.push(Drive {
            root,
            label,
            kind,
            total,
            free: total_free,
        });
    }
    out
}

/// Bytes per cluster for the volume containing `path` (4096 on failure).
pub fn cluster_size(path: &Path) -> u64 {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceW, GetVolumePathNameW};
    let wpath = wide_path(path);
    let mut vol = [0u16; 512];
    let ok = unsafe { GetVolumePathNameW(wpath.as_ptr(), vol.as_mut_ptr(), vol.len() as u32) };
    if ok == 0 {
        return 4096;
    }
    let mut spc = 0u32;
    let mut bps = 0u32;
    let mut free = 0u32;
    let mut total = 0u32;
    let ok = unsafe { GetDiskFreeSpaceW(vol.as_ptr(), &mut spc, &mut bps, &mut free, &mut total) };
    if ok != 0 && spc > 0 && bps > 0 {
        spc as u64 * bps as u64
    } else {
        4096
    }
}

/// Actual on-disk size of a compressed or sparse file, if obtainable.
pub fn compressed_size(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Foundation::{GetLastError, NO_ERROR};
    use windows_sys::Win32::Storage::FileSystem::{GetCompressedFileSizeW, INVALID_FILE_SIZE};
    let wpath = wide_path(path);
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(wpath.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE && unsafe { GetLastError() } != NO_ERROR {
        return None;
    }
    Some(((high as u64) << 32) | low as u64)
}

/// Whether the process runs with administrator rights (needed for MFT reads).
pub fn is_elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

/// Start this executable again with a UAC elevation prompt. Returns `false`
/// if the user declined or the launch failed.
pub fn relaunch_elevated(args: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let exe = wide_path(&exe);
    let verb = wide("runas");
    let params = wide(args);
    const SW_SHOWNORMAL: i32 = 1;
    let h = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value > 32 on success.
    h as isize > 32
}

/// Open `path` in Explorer: a directory is opened itself, a file is shown
/// selected in its parent folder.
pub fn open_in_explorer(path: &str, is_dir: bool) {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("explorer.exe");
    // raw_arg: Explorer parses its own command line, and std's quoting would
    // escape the trailing backslash of a drive root. Paths cannot contain
    // quotes on Windows, so plain quoting is safe.
    if is_dir {
        cmd.raw_arg(format!("\"{path}\""));
    } else {
        cmd.raw_arg(format!("/select,\"{path}\""));
    }
    if let Err(e) = cmd.spawn() {
        log::warn!("explorer.exe failed for {path}: {e}");
    }
}

/// Show the standard Explorer "Properties" dialog for a file, folder or drive.
/// The dialog runs on its own thread; this returns immediately.
pub fn show_properties(path: &str) -> bool {
    use windows_sys::Win32::UI::Shell::{SHOP_FILEPATH, SHObjectProperties};
    let wpath = wide(path);
    let ok = unsafe {
        SHObjectProperties(
            std::ptr::null_mut(),
            SHOP_FILEPATH as u32,
            wpath.as_ptr(),
            std::ptr::null(),
        )
    };
    ok != 0
}
