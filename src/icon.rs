//! Procedurally drawn application icon: a small sunburst in the chart's
//! colours. Dependency-free so that `build.rs` can include this file to
//! produce the `.ico` embedded in the executable, while the app uses the same
//! pixels for its window icon.

use std::f32::consts::TAU;

/// One ring of the icon: radii as fractions of the half-size, and the arcs
/// `(start, end)` in turns (0 = top, clockwise) with their colour.
struct Ring {
    r_in: f32,
    r_out: f32,
    arcs: &'static [(f32, f32, [u8; 3])],
}

const CENTER: [u8; 3] = [118, 118, 118];

const RINGS: [Ring; 3] = [
    Ring {
        r_in: 0.30,
        r_out: 0.62,
        arcs: &[
            (0.00, 0.58, [232, 52, 48]),
            (0.595, 0.83, [245, 98, 66]),
            (0.845, 0.985, [214, 66, 60]),
        ],
    },
    Ring {
        r_in: 0.64,
        r_out: 0.82,
        arcs: &[
            (0.00, 0.34, [244, 128, 60]),
            (0.355, 0.55, [240, 150, 70]),
            (0.61, 0.76, [246, 170, 72]),
        ],
    },
    Ring {
        r_in: 0.84,
        r_out: 0.98,
        arcs: &[(0.00, 0.20, [240, 196, 60]), (0.22, 0.30, [236, 214, 90])],
    },
];

/// Colour of a point given in normalised polar coordinates, if covered.
fn sample(r: f32, turn: f32) -> Option<[u8; 3]> {
    if r < 0.28 {
        return Some(CENTER);
    }
    for ring in &RINGS {
        if r >= ring.r_in && r < ring.r_out {
            for &(a0, a1, c) in ring.arcs {
                if turn >= a0 && turn < a1 {
                    return Some(c);
                }
            }
            return None;
        }
    }
    None
}

/// Straight (non-premultiplied) RGBA pixels, `size * size * 4` bytes, row
/// major from the top. 4x4 supersampling gives smooth edges.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    let half = size as f32 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0u32; 3];
            let mut hits = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32 - half;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32 - half;
                    let r = (px * px + py * py).sqrt() / half;
                    let mut turn = px.atan2(-py) / TAU;
                    if turn < 0.0 {
                        turn += 1.0;
                    }
                    if let Some(c) = sample(r, turn) {
                        acc[0] += c[0] as u32;
                        acc[1] += c[1] as u32;
                        acc[2] += c[2] as u32;
                        hits += 1;
                    }
                }
            }
            let avg = |v: u32| v.checked_div(hits).map(|a| a as u8);
            if let (Some(r), Some(g), Some(b)) = (avg(acc[0]), avg(acc[1]), avg(acc[2])) {
                let i = (y as usize * n + x as usize) * 4;
                out[i..i + 4].copy_from_slice(&[r, g, b, (hits * 255 / (SS * SS)) as u8]);
            }
        }
    }
    out
}

/// Encode the icon as a multi-resolution `.ico` (32-bit BMP entries).
#[allow(dead_code)] // used by build.rs only
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp_entry(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]); // reserved, type = icon
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// BITMAPINFOHEADER + bottom-up BGRA pixels + 1-bpp AND mask.
fn bmp_entry(size: u32) -> Vec<u8> {
    let px = rgba(size);
    let n = size as usize;
    let mask_stride = n.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + n * n * 4 + mask_stride * n);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&(2 * size as i32).to_le_bytes()); // XOR + AND
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]); // compression .. colours important
    for y in (0..n).rev() {
        for x in 0..n {
            let i = (y * n + x) * 4;
            out.extend_from_slice(&[px[i + 2], px[i + 1], px[i], px[i + 3]]);
        }
    }
    // Alpha channel carries transparency; an all-zero AND mask is correct.
    out.resize(out.len() + mask_stride * n, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape() {
        let px = rgba(32);
        assert_eq!(px.len(), 32 * 32 * 4);
        // Centre is the opaque grey disc, corners are transparent.
        let c = (16 * 32 + 16) * 4;
        assert_eq!(&px[c..c + 4], &[118, 118, 118, 255]);
        assert_eq!(px[3], 0);
        let ico = ico(&[16, 32]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
    }
}
