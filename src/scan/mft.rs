//! Fast NTFS scanner that reads the Master File Table directly.
//!
//! The volume is opened raw (`\\.\C:`, requires administrator rights), the
//! location of `$MFT` is taken from `FSCTL_GET_NTFS_VOLUME_DATA`, and the whole
//! table is read sequentially in large blocks. Every 1 KiB file record is
//! parsed in parallel for its name, parent directory and `$DATA` sizes; the
//! flat result is then linked into a tree. Compared with a directory walk this
//! avoids one syscall per directory and is typically an order of magnitude
//! faster on large volumes.
//!
//! Hard links are counted once (at the first non-DOS name), which matches the
//! actual space used on disk.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use anyhow::{Context, bail, ensure};
use crossbeam_channel::bounded;
use rayon::prelude::*;

use super::walk::Progress;
use crate::model::{Model, RawDir, RawFile};

const FSCTL_GET_NTFS_VOLUME_DATA: u32 = 0x0009_0064;
/// Record number of the root directory.
const ROOT_RECORD: u32 = 5;
/// Update sequence stride used by NTFS multi-sector protection.
const USA_STRIDE: usize = 512;
/// Bytes read from the volume per I/O request.
const READ_CHUNK: usize = 8 << 20;

const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

const REC_IN_USE: u16 = 0x0001;
const REC_DIRECTORY: u16 = 0x0002;

const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
const ATTR_FLAG_SPARSE: u16 = 0x8000;

const NS_DOS: u8 = 2;

#[repr(C)]
#[derive(Default, Debug)]
struct NtfsVolumeData {
    volume_serial_number: i64,
    number_sectors: i64,
    total_clusters: i64,
    free_clusters: i64,
    total_reserved: i64,
    bytes_per_sector: u32,
    bytes_per_cluster: u32,
    bytes_per_file_record_segment: u32,
    clusters_per_file_record_segment: u32,
    mft_valid_data_length: i64,
    mft_start_lcn: i64,
    mft2_start_lcn: i64,
    mft_zone_start: i64,
    mft_zone_end: i64,
}

/// `C:\` -> `Some('C')`; any other path (subdirectory, UNC) -> `None`.
pub fn volume_letter(path: &Path) -> Option<char> {
    let s = path.to_str()?.trim_end_matches(['\\', '/']);
    let b = s.as_bytes();
    (b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        .then(|| b[0].to_ascii_uppercase() as char)
}

/// Scan a whole NTFS volume through its MFT. Fails (so the caller can fall
/// back to a directory walk) when not elevated or not NTFS.
pub fn scan(root: &Path, progress: &Progress) -> anyhow::Result<Model> {
    let letter = volume_letter(root).context("MFT scan needs a volume root")?;
    let device = format!(r"\\.\{letter}:");
    let t = Instant::now();
    let mut vol = File::open(&device).with_context(|| format!("opening {device}"))?;
    let t_open = t.elapsed();
    let t = Instant::now();
    let vd = volume_data(&vol)?;
    let t_vd = t.elapsed();

    let cluster = vd.bytes_per_cluster as u64;
    let rec_size = vd.bytes_per_file_record_segment as usize;
    ensure!(
        cluster.is_power_of_two() && (512..=65536).contains(&rec_size),
        "unexpected NTFS geometry: cluster {cluster}, record {rec_size}"
    );

    let t = Instant::now();
    let runs = mft_runs(&mut vol, &vd)?;
    log::debug!(
        "MFT {letter}: open {t_open:?}, volume data {t_vd:?}, runs {:?} ({} extents)",
        t.elapsed(),
        runs.len()
    );
    let t = Instant::now();
    let mft_len = vd.mft_valid_data_length as u64;
    let n_records = (mft_len / rec_size as u64) as usize;
    let mut recs = vec![Rec::default(); n_records];
    log::debug!("MFT {letter}: alloc {:?}", t.elapsed());

    let t = Instant::now();
    read_and_parse(vol, &runs, cluster, mft_len, rec_size, &mut recs, progress)?;
    let t_read = t.elapsed();
    if progress.cancel.load(Relaxed) {
        bail!("scan cancelled");
    }

    let t = Instant::now();
    merge_extensions(&mut recs);
    let raw = build_tree(recs, letter);
    let t_tree = t.elapsed();
    let t = Instant::now();
    let model = Model::from_raw(raw, format!("{letter}:\\"), cluster);
    log::debug!(
        "MFT {letter}: {} MiB, read+parse {:?}, link {:?}, pack {:?}",
        mft_len >> 20,
        t_read,
        t_tree,
        t.elapsed()
    );
    Ok(model)
}

/// Stream the MFT in large blocks. A reader thread keeps the disk busy while
/// the previous block is parsed in parallel on the rayon pool.
fn read_and_parse(
    mut vol: File,
    runs: &[(u64, u64)],
    cluster: u64,
    mft_len: u64,
    rec_size: usize,
    recs: &mut [Rec],
    progress: &Progress,
) -> anyhow::Result<()> {
    // (first record index, buffer, valid bytes)
    type Block = (usize, Vec<u8>, usize);
    const BUFFERS: usize = 3;
    let (full_tx, full_rx) = bounded::<std::io::Result<Block>>(BUFFERS);
    let (free_tx, free_rx) = bounded::<Vec<u8>>(BUFFERS);
    for _ in 0..BUFFERS {
        free_tx.send(vec![0u8; READ_CHUNK]).expect("free list has room");
    }
    let n_records = recs.len();

    std::thread::scope(|s| {
        s.spawn(move || {
            let mut vcn_byte = 0u64; // byte offset within the MFT stream
            for &(lcn, clusters) in runs {
                let mut remaining = (clusters * cluster).min(mft_len.saturating_sub(vcn_byte));
                let mut disk = lcn * cluster;
                while remaining > 0 && !progress.cancel.load(Relaxed) {
                    let Ok(mut buf) = free_rx.recv() else { return };
                    let n = remaining.min(READ_CHUNK as u64) as usize;
                    let res = vol
                        .seek(SeekFrom::Start(disk))
                        .and_then(|_| vol.read_exact(&mut buf[..n]));
                    let first = (vcn_byte / rec_size as u64) as usize;
                    let msg = res.map(|_| (first, buf, n));
                    let failed = msg.is_err();
                    if full_tx.send(msg).is_err() || failed {
                        return;
                    }
                    disk += n as u64;
                    vcn_byte += n as u64;
                    remaining -= n as u64;
                }
            }
            // Dropping `full_tx` ends the consumer loop.
        });

        for msg in full_rx {
            let (first, mut buf, n) = msg?;
            if first < n_records {
                let end = (first + n / rec_size).min(n_records);
                recs[first..end]
                    .par_iter_mut()
                    .zip(buf[..n].par_chunks_mut(rec_size))
                    .for_each(|(r, raw)| *r = parse_record(raw));
                let (files, bytes) = recs[first..end]
                    .iter()
                    .filter(|r| r.in_use && r.base == 0 && !r.is_dir)
                    .fold((0u64, 0u64), |(f, b), r| (f + 1, b + r.size));
                progress.files.fetch_add(files, Relaxed);
                progress.bytes.fetch_add(bytes, Relaxed);
            }
            let _ = free_tx.send(buf);
        }
        Ok(())
    })
}

fn volume_data(vol: &File) -> anyhow::Result<NtfsVolumeData> {
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let mut vd = NtfsVolumeData::default();
    let mut returned = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            vol.as_raw_handle() as _,
            FSCTL_GET_NTFS_VOLUME_DATA,
            std::ptr::null(),
            0,
            (&mut vd as *mut NtfsVolumeData).cast(),
            size_of::<NtfsVolumeData>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("volume is not NTFS");
    }
    Ok(vd)
}

/// Data runs `(lcn, clusters)` of `$MFT` itself, in VCN order.
fn mft_runs(vol: &mut File, vd: &NtfsVolumeData) -> anyhow::Result<Vec<(u64, u64)>> {
    let cluster = vd.bytes_per_cluster as u64;
    let rec_size = vd.bytes_per_file_record_segment as usize;
    let mut rec0 = vec![0u8; rec_size];
    vol.seek(SeekFrom::Start(vd.mft_start_lcn as u64 * cluster))?;
    vol.read_exact(&mut rec0)?;
    ensure!(apply_fixups(&mut rec0), "corrupt $MFT record 0");

    // (lowest_vcn, runs) per $DATA extent.
    let mut extents: Vec<(u64, Vec<(u64, u64)>)> = Vec::new();
    let mut list_segments: Vec<u64> = Vec::new();
    for a in attributes(&rec0) {
        match a.kind {
            ATTR_DATA if a.non_resident && a.name_len == 0 => {
                extents.push((a.lowest_vcn(), decode_runs(a.mapping_pairs())?));
            }
            // Heavily fragmented MFT: further $DATA extents live in other
            // records, listed here. Only resident lists are supported.
            ATTR_ATTRIBUTE_LIST if !a.non_resident => {
                list_segments.extend(attribute_list_data_segments(a.value()));
            }
            _ => {}
        }
    }
    ensure!(!extents.is_empty(), "$MFT has no $DATA attribute");

    for seg in list_segments.into_iter().filter(|&s| s != 0) {
        let known = flatten(&extents);
        let off = seg * rec_size as u64;
        let Some(disk) = vcn_byte_to_disk(&known, off, cluster) else {
            bail!("$MFT extension record {seg} not reachable");
        };
        let mut rec = vec![0u8; rec_size];
        vol.seek(SeekFrom::Start(disk))?;
        vol.read_exact(&mut rec)?;
        if !apply_fixups(&mut rec) {
            continue;
        }
        for a in attributes(&rec) {
            if a.kind == ATTR_DATA && a.non_resident && a.name_len == 0 {
                let vcn = a.lowest_vcn();
                if !extents.iter().any(|(v, _)| *v == vcn) {
                    extents.push((vcn, decode_runs(a.mapping_pairs())?));
                }
            }
        }
    }
    Ok(flatten(&extents))
}

fn flatten(extents: &[(u64, Vec<(u64, u64)>)]) -> Vec<(u64, u64)> {
    let mut e: Vec<_> = extents.iter().collect();
    e.sort_by_key(|(vcn, _)| *vcn);
    e.into_iter().flat_map(|(_, r)| r.iter().copied()).collect()
}

fn vcn_byte_to_disk(runs: &[(u64, u64)], mut off: u64, cluster: u64) -> Option<u64> {
    for &(lcn, len) in runs {
        let bytes = len * cluster;
        if off < bytes {
            return Some(lcn * cluster + off);
        }
        off -= bytes;
    }
    None
}

/// Segment numbers of records holding `$DATA` extents, from a resident
/// `$ATTRIBUTE_LIST` value.
fn attribute_list_data_segments(v: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 0x20 <= v.len() {
        let kind = u32le(v, p);
        let len = u16le(v, p + 4) as usize;
        if len == 0 {
            break;
        }
        if kind == ATTR_DATA {
            out.push(u64le(v, p + 0x10) & 0xFFFF_FFFF_FFFF);
        }
        p += len;
    }
    out
}

/// Decode NTFS mapping pairs into absolute `(lcn, clusters)` runs. Sparse
/// runs (no offset) are skipped.
pub(crate) fn decode_runs(mut p: &[u8]) -> anyhow::Result<Vec<(u64, u64)>> {
    let mut runs = Vec::new();
    let mut lcn: i64 = 0;
    while let Some(&h) = p.first() {
        if h == 0 {
            break;
        }
        let len_n = (h & 0x0F) as usize;
        let off_n = (h >> 4) as usize;
        ensure!(len_n > 0 && len_n <= 8 && off_n <= 8, "bad run header {h:#x}");
        ensure!(p.len() > len_n + off_n, "truncated run list");
        let mut len: u64 = 0;
        for i in 0..len_n {
            len |= (p[1 + i] as u64) << (8 * i);
        }
        if off_n > 0 {
            let mut off: i64 = 0;
            for i in 0..off_n {
                off |= (p[1 + len_n + i] as i64) << (8 * i);
            }
            // Sign-extend.
            let shift = 64 - 8 * off_n as u32;
            off = (off << shift) >> shift;
            lcn += off;
            ensure!(lcn >= 0, "negative LCN in run list");
            runs.push((lcn as u64, len));
        }
        p = &p[1 + len_n + off_n..];
    }
    Ok(runs)
}

/// Parsed contents of one file record (or an extension record).
#[derive(Clone, Default, Debug)]
pub(crate) struct Rec {
    in_use: bool,
    is_dir: bool,
    /// Base record number for extension records, 0 for base records.
    base: u32,
    parent: u32,
    /// First non-DOS name (DOS 8.3 aliases are never kept).
    name: Option<Box<str>>,
    has_data: bool,
    size: u64,
    alloc: u64,
}

pub(crate) fn parse_record(raw: &mut [u8]) -> Rec {
    let mut r = Rec::default();
    if raw.len() < 0x30 || &raw[0..4] != b"FILE" || !apply_fixups(raw) {
        return r;
    }
    let flags = u16le(raw, 0x16);
    if flags & REC_IN_USE == 0 {
        return r;
    }
    r.in_use = true;
    r.is_dir = flags & REC_DIRECTORY != 0;
    r.base = (u64le(raw, 0x20) & 0xFFFF_FFFF_FFFF) as u32;

    for a in attributes(raw) {
        match a.kind {
            ATTR_FILE_NAME if !a.non_resident => {
                let v = a.value();
                if v.len() < 0x42 {
                    continue;
                }
                let ns = v[0x41];
                if ns == NS_DOS || r.name.is_some() {
                    continue;
                }
                let n = v[0x40] as usize;
                if v.len() < 0x42 + 2 * n {
                    continue;
                }
                let units = (0..n).map(|i| u16le(v, 0x42 + 2 * i));
                let name: String = char::decode_utf16(units)
                    .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                    .collect();
                r.parent = (u64le(v, 0) & 0xFFFF_FFFF_FFFF) as u32;
                r.name = Some(name.into_boxed_str());
            }
            ATTR_DATA if a.name_len == 0 => {
                if a.non_resident {
                    // Sizes are only valid in the first extent.
                    if a.lowest_vcn() != 0 {
                        continue;
                    }
                    let h = a.header;
                    if h.len() < 0x40 {
                        continue;
                    }
                    r.size = u64le(h, 0x30);
                    r.alloc = if a.flags & (ATTR_FLAG_COMPRESSED | ATTR_FLAG_SPARSE) != 0
                        && h.len() >= 0x48
                    {
                        u64le(h, 0x40)
                    } else {
                        u64le(h, 0x28)
                    };
                } else {
                    // Resident data lives inside the MFT record: no clusters.
                    r.size = a.value().len() as u64;
                    r.alloc = 0;
                }
                r.has_data = true;
            }
            _ => {}
        }
    }
    r
}

/// Fold names and sizes found in extension records into their base record.
fn merge_extensions(recs: &mut [Rec]) {
    let ext: Vec<usize> = (0..recs.len())
        .filter(|&i| recs[i].in_use && recs[i].base != 0)
        .collect();
    for i in ext {
        let base = recs[i].base as usize;
        if base >= recs.len() || !recs[base].in_use {
            continue;
        }
        let e = std::mem::take(&mut recs[i]);
        let b = &mut recs[base];
        if b.name.is_none() && e.name.is_some() {
            b.name = e.name;
            b.parent = e.parent;
        }
        if !b.has_data && e.has_data {
            b.has_data = true;
            b.size = e.size;
            b.alloc = e.alloc;
        }
    }
}

/// Link flat records into a nested tree rooted at record 5. Names are moved
/// out of `recs`, not copied.
fn build_tree(mut recs: Vec<Rec>, letter: char) -> RawDir {
    let n = recs.len();
    let valid = |i: usize| {
        let r = &recs[i];
        r.in_use && r.base == 0 && r.name.is_some()
    };
    // Children lists in CSR form.
    let mut counts = vec![0u32; n + 1];
    for i in 0..n {
        if i as u32 == ROOT_RECORD || !valid(i) {
            continue;
        }
        let p = recs[i].parent as usize;
        if p < n && recs[p].in_use && recs[p].is_dir {
            counts[p] += 1;
        }
    }
    let mut start = vec![0u32; n + 1];
    for i in 0..n {
        start[i + 1] = start[i] + counts[i];
    }
    let mut fill = start.clone();
    let mut kids = vec![0u32; start[n] as usize];
    for i in 0..n {
        if i as u32 == ROOT_RECORD || !valid(i) {
            continue;
        }
        let p = recs[i].parent as usize;
        if p < n && recs[p].in_use && recs[p].is_dir {
            kids[fill[p] as usize] = i as u32;
            fill[p] += 1;
        }
    }

    fn build(recs: &mut [Rec], start: &[u32], kids: &[u32], id: usize, name: String, depth: u32) -> RawDir {
        let mut d = RawDir {
            name,
            ..Default::default()
        };
        // Guard against corrupted parent links forming deep chains.
        if depth > 512 {
            return d;
        }
        for &c in &kids[start[id] as usize..start[id + 1] as usize] {
            let r = &mut recs[c as usize];
            let name = r.name.take().map(String::from).unwrap_or_default();
            if r.is_dir {
                d.subdirs
                    .push(build(recs, start, kids, c as usize, name, depth + 1));
            } else {
                d.files.push(RawFile {
                    name,
                    size: r.size,
                    alloc: r.alloc,
                });
            }
        }
        d
    }

    let root = ROOT_RECORD as usize;
    if root >= n {
        return RawDir {
            name: format!("{letter}:"),
            ..Default::default()
        };
    }
    build(&mut recs, &start, &kids, root, format!("{letter}:"), 0)
}

// --- record / attribute parsing helpers ------------------------------------

#[inline]
fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline]
fn u64le(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Apply the update sequence array. Returns `false` on a torn/corrupt record.
pub(crate) fn apply_fixups(rec: &mut [u8]) -> bool {
    if rec.len() < 8 {
        return false;
    }
    let usa_off = u16le(rec, 4) as usize;
    let usa_cnt = u16le(rec, 6) as usize;
    if usa_cnt == 0 || usa_off + 2 * usa_cnt > rec.len() || (usa_cnt - 1) * USA_STRIDE > rec.len() {
        return false;
    }
    let usn = [rec[usa_off], rec[usa_off + 1]];
    for i in 1..usa_cnt {
        let end = i * USA_STRIDE;
        if rec[end - 2..end] != usn {
            return false;
        }
        rec[end - 2] = rec[usa_off + 2 * i];
        rec[end - 1] = rec[usa_off + 2 * i + 1];
    }
    true
}

struct Attr<'a> {
    kind: u32,
    non_resident: bool,
    name_len: u8,
    flags: u16,
    /// The whole attribute (header + value / mapping pairs).
    header: &'a [u8],
}

impl<'a> Attr<'a> {
    fn value(&self) -> &'a [u8] {
        let h = self.header;
        if h.len() < 0x18 {
            return &[];
        }
        let len = u32le(h, 0x10) as usize;
        let off = u16le(h, 0x14) as usize;
        h.get(off..off.saturating_add(len)).unwrap_or(&[])
    }

    fn lowest_vcn(&self) -> u64 {
        if self.header.len() >= 0x18 {
            u64le(self.header, 0x10)
        } else {
            0
        }
    }

    fn mapping_pairs(&self) -> &'a [u8] {
        let h = self.header;
        if h.len() < 0x22 {
            return &[];
        }
        let off = u16le(h, 0x20) as usize;
        h.get(off..).unwrap_or(&[])
    }
}

fn attributes(rec: &[u8]) -> impl Iterator<Item = Attr<'_>> {
    let used = (u32le(rec, 0x18) as usize).min(rec.len());
    let mut p = u16le(rec, 0x14) as usize;
    std::iter::from_fn(move || {
        if p + 0x10 > used {
            return None;
        }
        let kind = u32le(rec, p);
        if kind == ATTR_END {
            return None;
        }
        let len = u32le(rec, p + 4) as usize;
        if len < 0x10 || p + len > used {
            return None;
        }
        let a = Attr {
            kind,
            non_resident: rec[p + 8] != 0,
            name_len: rec[p + 9],
            flags: u16le(rec, p + 0x0C),
            header: &rec[p..p + len],
        };
        p += len;
        Some(a)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_letters() {
        assert_eq!(volume_letter(Path::new(r"C:\")), Some('C'));
        assert_eq!(volume_letter(Path::new("d:")), Some('D'));
        assert_eq!(volume_letter(Path::new(r"D:\Projects")), None);
        assert_eq!(volume_letter(Path::new(r"\\server\share")), None);
    }

    #[test]
    fn runs_decode() {
        // len=0x18 clusters at lcn 0x5634; then len 0x10 at -0x10 relative;
        // then a sparse run (skipped); end.
        let p = [0x21, 0x18, 0x34, 0x56, 0x11, 0x10, 0xF0, 0x01, 0x05, 0x00];
        let r = decode_runs(&p).unwrap();
        assert_eq!(r, vec![(0x5634, 0x18), (0x5634 - 0x10, 0x10)]);
    }

    /// Build a minimal 1 KiB FILE record with the given attributes.
    fn record(flags: u16, base: u64, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut r = vec![0u8; 1024];
        r[0..4].copy_from_slice(b"FILE");
        r[4..6].copy_from_slice(&0x30u16.to_le_bytes()); // usa offset
        r[6..8].copy_from_slice(&3u16.to_le_bytes()); // usa count (1 + 2 sectors)
        r[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes()); // first attr
        r[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
        r[0x20..0x28].copy_from_slice(&base.to_le_bytes());
        let mut p = 0x38;
        for a in attrs {
            r[p..p + a.len()].copy_from_slice(a);
            p += a.len();
        }
        r[p..p + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        r[0x18..0x1C].copy_from_slice(&((p + 8) as u32).to_le_bytes());
        // Protect sector ends with USN 0xABCD, saving originals in the array.
        let usn = [0xCD, 0xAB];
        r[0x30..0x32].copy_from_slice(&usn);
        for i in 1..3 {
            let end = i * 512;
            let (a, b) = (r[end - 2], r[end - 1]);
            r[0x30 + 2 * i] = a;
            r[0x31 + 2 * i] = b;
            r[end - 2..end].copy_from_slice(&usn);
        }
        r
    }

    fn file_name_attr(parent: u64, name: &str, ns: u8) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut v = vec![0u8; 0x42 + 2 * units.len()];
        v[0..8].copy_from_slice(&parent.to_le_bytes());
        v[0x40] = units.len() as u8;
        v[0x41] = ns;
        for (i, u) in units.iter().enumerate() {
            v[0x42 + 2 * i..0x44 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
        resident(ATTR_FILE_NAME, &v)
    }

    fn resident(kind: u32, value: &[u8]) -> Vec<u8> {
        let len = (0x18 + value.len()).div_ceil(8) * 8;
        let mut a = vec![0u8; len];
        a[0..4].copy_from_slice(&kind.to_le_bytes());
        a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
        a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
        a[0x14..0x16].copy_from_slice(&0x18u16.to_le_bytes());
        a[0x18..0x18 + value.len()].copy_from_slice(value);
        a
    }

    fn non_resident_data(size: u64, alloc: u64, compressed: Option<u64>) -> Vec<u8> {
        let mut a = vec![0u8; 0x50];
        a[0..4].copy_from_slice(&ATTR_DATA.to_le_bytes());
        a[4..8].copy_from_slice(&0x50u32.to_le_bytes());
        a[8] = 1;
        if compressed.is_some() {
            a[0x0C..0x0E].copy_from_slice(&ATTR_FLAG_COMPRESSED.to_le_bytes());
        }
        a[0x20..0x22].copy_from_slice(&0x48u16.to_le_bytes());
        a[0x28..0x30].copy_from_slice(&alloc.to_le_bytes());
        a[0x30..0x38].copy_from_slice(&size.to_le_bytes());
        a[0x38..0x40].copy_from_slice(&size.to_le_bytes());
        a[0x40..0x48].copy_from_slice(&compressed.unwrap_or(0).to_le_bytes());
        a
    }

    #[test]
    fn parses_names_sizes_and_skips_dos_names() {
        let mut r = record(
            REC_IN_USE,
            0,
            &[
                file_name_attr(5 | (3 << 48), "LONGNA~1.TXT", NS_DOS),
                file_name_attr(5 | (3 << 48), "long name.txt", 1),
                non_resident_data(10_000, 12_288, None),
            ],
        );
        let p = parse_record(&mut r);
        assert!(p.in_use && !p.is_dir);
        assert_eq!(p.parent, 5);
        assert_eq!(p.name.as_deref(), Some("long name.txt"));
        assert_eq!((p.size, p.alloc), (10_000, 12_288));

        let mut r = record(
            REC_IN_USE,
            0,
            &[
                file_name_attr(5, "small", 3),
                resident(ATTR_DATA, b"hello"),
            ],
        );
        let p = parse_record(&mut r);
        assert_eq!((p.size, p.alloc), (5, 0));

        let mut r = record(
            REC_IN_USE,
            0,
            &[
                file_name_attr(5, "packed", 1),
                non_resident_data(1 << 20, 1 << 20, Some(64 << 10)),
            ],
        );
        let p = parse_record(&mut r);
        assert_eq!((p.size, p.alloc), (1 << 20, 64 << 10));
    }

    #[test]
    fn rejects_torn_and_free_records() {
        let mut r = record(REC_IN_USE, 0, &[file_name_attr(5, "x", 1)]);
        r[511] ^= 0xFF; // torn write
        assert!(!parse_record(&mut r).in_use);
        let mut r = record(0, 0, &[file_name_attr(5, "x", 1)]);
        assert!(!parse_record(&mut r).in_use);
    }

    #[test]
    fn builds_tree_with_extensions() {
        let dir = REC_IN_USE | REC_DIRECTORY;
        let mut raw: Vec<Vec<u8>> = (0..10).map(|_| vec![0u8; 1024]).collect();
        raw[5] = record(dir, 0, &[file_name_attr(5, ".", 3)]);
        raw[6] = record(dir, 0, &[file_name_attr(5, "Users", 1)]);
        raw[7] = record(REC_IN_USE, 0, &[file_name_attr(6, "big.bin", 1)]);
        // Extension record of 7 carrying its $DATA.
        raw[8] = record(REC_IN_USE, 7, &[non_resident_data(5000, 8192, None)]);
        raw[9] = record(REC_IN_USE, 0, &[
            file_name_attr(5, "root.txt", 1),
            resident(ATTR_DATA, b"abc"),
        ]);
        let mut recs: Vec<Rec> = raw.iter_mut().map(|r| parse_record(r)).collect();
        merge_extensions(&mut recs);
        let m = Model::from_raw(build_tree(recs, 'X'), "X:\\".into(), 4096);
        let root = m.node(0);
        assert_eq!(m.name(0), "X:");
        assert_eq!((root.files, root.dirs), (2, 1));
        assert_eq!(root.size, 5003);
        let first = m.children(0).next().unwrap();
        assert_eq!(m.name(first), "Users");
        assert_eq!(m.path(m.children(first).next().unwrap()), "X:\\Users\\big.bin");
    }
}
