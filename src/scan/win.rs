//! Thin Win32 helpers: volume enumeration, cluster size, compressed sizes,
//! directory listing.

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
    /// File system name, e.g. `NTFS`, `FAT32`, `exFAT`.
    pub fs: String,
    pub kind: DriveKind,
    pub total: u64,
    pub free: u64,
}

impl Drive {
    pub fn display(&self) -> String {
        let letter = self.root.trim_end_matches('\\');
        let label = if self.label.is_empty() {
            // As Explorer names them.
            match self.kind {
                DriveKind::Removable => tr!("Removable Disk", "Съёмный диск"),
                DriveKind::Remote => tr!("Network Drive", "Сетевой диск"),
                DriveKind::CdRom => tr!("CD Drive", "Дисковод CD"),
                DriveKind::RamDisk => tr!("RAM Disk", "RAM-диск"),
                DriveKind::Fixed | DriveKind::Unknown => tr!("Local Disk", "Локальный диск"),
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

/// Roots of the logical drives (`C:\`), from a bitmask: instant, no I/O.
pub fn drive_roots() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i) as char))
        .collect()
}

/// Label, type, file system and capacity of the drive at `root`; `None` if
/// it is not ready (empty card reader, etc.). This touches the volume, so a
/// disconnected network drive can take seconds.
pub fn drive_info(root: &str) -> Option<Drive> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
    // Values of DRIVE_* from winbase.h.
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;
    let wroot = wide(root);
    let kind = match unsafe { GetDriveTypeW(wroot.as_ptr()) } {
        DRIVE_FIXED => DriveKind::Fixed,
        DRIVE_REMOVABLE => DriveKind::Removable,
        DRIVE_REMOTE => DriveKind::Remote,
        DRIVE_CDROM => DriveKind::CdRom,
        DRIVE_RAMDISK => DriveKind::RamDisk,
        _ => DriveKind::Unknown,
    };
    let mut label_buf = [0u16; 261];
    let mut fs_buf = [0u16; 261];
    let ok = unsafe {
        GetVolumeInformationW(
            wroot.as_ptr(),
            label_buf.as_mut_ptr(),
            label_buf.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_buf.as_mut_ptr(),
            fs_buf.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let from_buf = |b: &[u16]| {
        let len = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf16_lossy(&b[..len])
    };
    let space = disk_space(Path::new(root)).unwrap_or_default();
    Some(Drive {
        root: root.to_string(),
        label: from_buf(&label_buf),
        fs: from_buf(&fs_buf),
        kind,
        total: space.total,
        free: space.free,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskSpace {
    pub total: u64,
    pub free: u64,
}

/// Capacity and free bytes of the volume (or network share) holding `path`.
pub fn disk_space(path: &Path) -> Option<DiskSpace> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wpath = wide_path(path);
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(wpath.as_ptr(), &mut available, &mut total, &mut free) };
    (ok != 0).then_some(DiskSpace { total, free })
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

/// Repaint the caption and frame of window `hwnd` after its dark-mode
/// attribute changed: Windows 10 keeps the old colours until the next
/// non-client repaint (a resize, a change of focus). Flipping the caption
/// to the other activation state and back forces one. The functions come
/// from user32 directly, to spare another `windows-sys` feature.
pub fn refresh_caption(hwnd: isize) {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetActiveWindow() -> isize;
    }
    const WM_NCACTIVATE: u32 = 0x0086;
    unsafe {
        let active = GetActiveWindow() == hwnd;
        SendMessageW(hwnd, WM_NCACTIVATE, usize::from(!active), 0);
        SendMessageW(hwnd, WM_NCACTIVATE, usize::from(active), 0);
    }
}

/// Windows' name for files with extension `ext` (without the dot), as the
/// Type column of Explorer shows it: `MP4 Video`, `Текстовый документ`, in
/// the language of Windows.
pub fn type_description(ext: &str) -> Option<String> {
    use windows_sys::Win32::UI::Shell::{ASSOCF_NOTRUNCATE, ASSOCSTR_FRIENDLYDOCNAME, AssocQueryStringW};
    let assoc: Vec<u16> = format!(".{ext}").encode_utf16().chain(Some(0)).collect();
    let mut buf = vec![0u16; 260];
    let mut len = buf.len() as u32;
    let hr = unsafe {
        AssocQueryStringW(
            ASSOCF_NOTRUNCATE,
            ASSOCSTR_FRIENDLYDOCNAME,
            assoc.as_ptr(),
            std::ptr::null(),
            buf.as_mut_ptr(),
            &mut len,
        )
    };
    if hr != 0 {
        return None;
    }
    // The length counts the terminating zero.
    let text = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1).min(buf.len())]);
    (!text.trim().is_empty()).then_some(text)
}

/// Whether Windows shows its interface in Russian (primary language of
/// the user's UI language, `LANG_RUSSIAN`).
pub fn ui_language_is_russian() -> bool {
    const LANG_RUSSIAN: u16 = 0x19;
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    id & 0x3FF == LANG_RUSSIAN
}

/// Offset of local time from UTC in seconds, as Windows applies it now
/// (daylight saving included), for showing dates the way Explorer does.
pub fn utc_offset() -> i64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
    let ticks = (crate::model::unix_now() as u64 + 11_644_473_600) * 10_000_000;
    let utc = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut local = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    if unsafe { FileTimeToLocalFileTime(&utc, &mut local) } == 0 {
        return 0;
    }
    let local = ((local.dwHighDateTime as u64) << 32) | local.dwLowDateTime as u64;
    (local as i64 - ticks as i64) / 10_000_000
}

/// One entry of a directory listing.
#[derive(Debug)]
pub struct DirEntry {
    pub name: String,
    pub attrs: u32,
    /// Reparse tag when `attrs` has `FILE_ATTRIBUTE_REPARSE_POINT`.
    pub reparse_tag: u32,
    /// Logical size (end of file).
    pub size: u64,
    /// Last write time in Unix seconds, 0 if unknown.
    pub modified: u32,
}

impl DirEntry {
    /// Symbolic link, junction or another name-surrogate reparse point: the
    /// same rule as `std::fs::FileType::is_symlink`.
    pub fn is_link(&self) -> bool {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        const NAME_SURROGATE_BIT: u32 = 0x2000_0000;
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && self.reparse_tag & NAME_SURROGATE_BIT != 0
    }

    pub fn is_dir(&self) -> bool {
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0 && !self.is_link()
    }
}

/// Bytes requested per directory query.
const LIST_BUF_BYTES: usize = 64 << 10;

/// Append the entries of directory `path` (without `.` and `..`) to `out`.
///
/// `GetFileInformationByHandleEx(FileFullDirectoryInfo)` returns up to
/// `LIST_BUF_BYTES` of entries per call, where `read_dir` fetches a few KiB per
/// `FindNextFileW`. On an error the entries read so far are kept.
pub fn list_dir(path: &Path, out: &mut Vec<DirEntry>) -> std::io::Result<()> {
    use std::cell::RefCell;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileFullDirectoryInfo, GetFileInformationByHandleEx,
    };

    thread_local! {
        // u64 elements keep the 8-byte alignment the entries require.
        static BUF: RefCell<Vec<u64>> = RefCell::new(vec![0; LIST_BUF_BYTES / 8]);
    }

    let dir = std::fs::OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    BUF.with_borrow_mut(|buf| loop {
        let ok = unsafe {
            GetFileInformationByHandleEx(
                dir.as_raw_handle(),
                FileFullDirectoryInfo,
                buf.as_mut_ptr().cast(),
                LIST_BUF_BYTES as u32,
            )
        };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            return match err.raw_os_error() {
                Some(code) if code == ERROR_NO_MORE_FILES as i32 => Ok(()),
                _ => Err(err),
            };
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), LIST_BUF_BYTES) };
        parse_full_dir_info(bytes, out);
    })
}

/// Decode a buffer of chained `FILE_FULL_DIR_INFO` records.
fn parse_full_dir_info(buf: &[u8], out: &mut Vec<DirEntry>) {
    use std::mem::offset_of;
    use windows_sys::Win32::Storage::FileSystem::FILE_FULL_DIR_INFO as Info;

    let u32_at = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    let name_off = offset_of!(Info, FileName);
    let mut pos = 0usize;
    loop {
        if pos + name_off > buf.len() {
            return;
        }
        let name_len = u32_at(pos + offset_of!(Info, FileNameLength)) as usize;
        let name_start = pos + name_off;
        let Some(name_bytes) = buf.get(name_start..name_start + name_len) else {
            return;
        };
        let units = name_bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c));
        let name: String = char::decode_utf16(units)
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        if name != "." && name != ".." {
            out.push(DirEntry {
                name,
                attrs: u32_at(pos + offset_of!(Info, FileAttributes)),
                // For reparse points the EA size field carries the tag.
                reparse_tag: u32_at(pos + offset_of!(Info, EaSize)),
                size: u64_at(pos + offset_of!(Info, EndOfFile)),
                modified: crate::model::unix_from_filetime(u64_at(pos + offset_of!(Info, LastWriteTime))),
            });
        }
        let next = u32_at(pos + offset_of!(Info, NextEntryOffset)) as usize;
        if next == 0 {
            return;
        }
        pos += next;
    }
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

/// Move `path` to the Recycle Bin through the shell, which shows its own
/// progress and error dialogs and, for an item that cannot be recycled
/// (too large, a network drive), asks before deleting it for good. Blocks
/// until done, so call it off the UI thread. `Err` says why it stopped;
/// either way, check what is left on disk.
pub fn recycle(path: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };
    // pFrom is a list of paths ending with an empty one: two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        // Confirmation is the app's own; the shell still warns before a
        // permanent delete.
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if op.fAnyOperationsAborted != 0 {
        Err("cancelled".into())
    } else if code != 0 {
        Err(format!("error {code:#x}"))
    } else {
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;
    use windows_sys::Win32::Storage::FileSystem::FILE_FULL_DIR_INFO as Info;

    /// 2024-01-01 00:00 UTC as a FILETIME, the write time of every entry.
    const JAN_2024: u64 = 133_485_408_000_000_000;

    /// Chain `FILE_FULL_DIR_INFO` records the way the file system does:
    /// 8-byte aligned, the last one with `NextEntryOffset == 0`.
    fn records(entries: &[(&str, u32, u32, u64)]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut last = 0;
        for &(name, attrs, ea, size) in entries {
            let start = buf.len();
            let name: Vec<u16> = name.encode_utf16().collect();
            let len = offset_of!(Info, FileName) + 2 * name.len();
            buf.resize(start + len.next_multiple_of(8), 0);
            let mut put = |off: usize, bytes: &[u8]| {
                buf[start + off..start + off + bytes.len()].copy_from_slice(bytes)
            };
            put(offset_of!(Info, EndOfFile), &size.to_le_bytes());
            put(offset_of!(Info, LastWriteTime), &JAN_2024.to_le_bytes());
            put(offset_of!(Info, FileAttributes), &attrs.to_le_bytes());
            put(offset_of!(Info, FileNameLength), &(2 * name.len() as u32).to_le_bytes());
            put(offset_of!(Info, EaSize), &ea.to_le_bytes());
            for (i, u) in name.iter().enumerate() {
                put(offset_of!(Info, FileName) + 2 * i, &u.to_le_bytes());
            }
            if start > 0 {
                let next = (start - last) as u32;
                buf[last..last + 4].copy_from_slice(&next.to_le_bytes());
            }
            last = start;
        }
        buf
    }

    #[test]
    fn parses_directory_listing() {
        const DIR: u32 = 0x10;
        const REPARSE: u32 = 0x400;
        const JUNCTION: u32 = 0xA000_0003;
        const APPEXECLINK: u32 = 0x8000_001B;
        let buf = records(&[
            (".", DIR, 0, 0),
            ("..", DIR, 0, 0),
            ("a.txt", 0x20, 0, 5),
            ("Документы", DIR, 0, 0),
            ("link", DIR | REPARSE, JUNCTION, 0),
            ("app.exe", REPARSE, APPEXECLINK, 0),
        ]);
        let mut out = Vec::new();
        parse_full_dir_info(&buf, &mut out);
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "Документы", "link", "app.exe"]);
        assert_eq!(out[0].size, 5);
        assert_eq!(out[0].modified, 1_704_067_200);
        assert!(!out[0].is_dir() && !out[0].is_link());
        assert!(out[1].is_dir() && !out[1].is_link());
        // A junction is a link, not a directory to descend into.
        assert!(out[2].is_link() && !out[2].is_dir());
        // App execution aliases are not name surrogates: plain files.
        assert!(!out[3].is_link() && !out[3].is_dir());
    }

    #[test]
    fn truncated_buffer_stops_cleanly() {
        let buf = records(&[("first", 0x20, 0, 1), ("second", 0x20, 0, 2)]);
        let mut out = Vec::new();
        parse_full_dir_info(&buf[..buf.len() - 4], &mut out);
        assert_eq!(out.len(), 1);
    }
}
