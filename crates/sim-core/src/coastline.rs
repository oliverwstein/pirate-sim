//! Cosmetic land polygons + coastline polylines from Natural Earth 10m.
//!
//! Purely visual: `LandMap` remains the truth for collisions and pathing.
//! Two artifacts are loaded here:
//!
//! - `coastline.bin` — polylines for the coast (from `ne_10m_coastline`).
//! - `land_polys.bin` — pre-triangulated land mesh (from `ne_10m_land`),
//!   ready for direct rendering. Coordinates are NM-from-origin, matching
//!   the LandMap / WindGrid frame.
//!
//! Both files are produced by scripts in `tools/preprocess/`. If a file is
//! missing, the corresponding loader returns an empty struct — rendering
//! is optional.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::types::Position;

#[derive(Default)]
pub struct CoastlineMap {
    pub lines: Vec<Vec<Position>>,
}

impl CoastlineMap {
    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Self::from_bytes(&buf)
    }

    fn from_bytes(buf: &[u8]) -> io::Result<Self> {
        let mut cur = 0usize;
        let num_lines = read_u32(buf, &mut cur)? as usize;
        let mut lines = Vec::with_capacity(num_lines);
        for _ in 0..num_lines {
            let n = read_u32(buf, &mut cur)? as usize;
            let mut line = Vec::with_capacity(n);
            for _ in 0..n {
                let x = read_f32(buf, &mut cur)?;
                let y = read_f32(buf, &mut cur)?;
                line.push(Position::new(x, y));
            }
            lines.push(line);
        }
        Ok(Self { lines })
    }
}

/// Pre-triangulated land mesh: a flat vertex array and a triangle index
/// list. Each consecutive group of three indices forms one triangle.
#[derive(Default)]
pub struct LandMesh {
    pub vertices: Vec<Position>,
    pub indices: Vec<u32>,
}

impl LandMesh {
    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Self::from_bytes(&buf)
    }

    fn from_bytes(buf: &[u8]) -> io::Result<Self> {
        let mut cur = 0usize;
        let nv = read_u32(buf, &mut cur)? as usize;
        let mut vertices = Vec::with_capacity(nv);
        for _ in 0..nv {
            let x = read_f32(buf, &mut cur)?;
            let y = read_f32(buf, &mut cur)?;
            vertices.push(Position::new(x, y));
        }
        let ni = read_u32(buf, &mut cur)? as usize;
        let mut indices = Vec::with_capacity(ni);
        for _ in 0..ni {
            indices.push(read_u32(buf, &mut cur)?);
        }
        Ok(Self { vertices, indices })
    }
}

fn read_u32(buf: &[u8], cur: &mut usize) -> io::Result<u32> {
    if *cur + 4 > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "coastline u32",
        ));
    }
    let v = u32::from_le_bytes(buf[*cur..*cur + 4].try_into().unwrap());
    *cur += 4;
    Ok(v)
}

fn read_f32(buf: &[u8], cur: &mut usize) -> io::Result<f32> {
    if *cur + 4 > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "coastline f32",
        ));
    }
    let v = f32::from_le_bytes(buf[*cur..*cur + 4].try_into().unwrap());
    *cur += 4;
    Ok(v)
}
