//! Binary STL converter for wireframe graphics.
//!
//! This library can read a binary STL model and produce a minimized set of
//! edges needed to represent the model as a transparent wireframe.
//!
//! As STL uses an unordered bag-of-triangles approach with no connectivity
//! information, we perform some basic quantization and regularization on the
//! mesh before producing output. This has the side effect of reducing the
//! amount of drawing required.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::{self, Read, Seek, Write};

use byteorder::{LittleEndian, ReadBytesExt};
use math::Vec3;
use ordered_float::OrderedFloat;

/// Quantization factor. Coordinates are multiplied by this before rounding, so
/// we preserve about one fractional decimal digit per trailing zero in this
/// number.
const Q: f32 = 10.;

type Of32 = OrderedFloat<f32>;
type Vec3of = Vec3<Of32>;

/// Forces the point onto a quantized grid, to merge points that are nearby
/// but not exactly identical.
fn quantize(p: Vec3of) -> Vec3of {
    fn q(v: Of32) -> Of32 {
        OrderedFloat((v.0 * Q).round() / Q)
    }

    Vec3(q(p.0), q(p.1), q(p.2))
}

/// Loads a point from binary STL representation.
fn read_point(mut input: impl Read) -> io::Result<Vec3of> {
    Ok(Vec3(
        OrderedFloat(input.read_f32::<LittleEndian>()?),
        OrderedFloat(input.read_f32::<LittleEndian>()?),
        OrderedFloat(input.read_f32::<LittleEndian>()?),
    ))
}

/// An edge in the connectivity graph. Each edge connects two vertices, which
/// are referenced by unique IDs.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Debug)]
struct Edge(usize, usize);

impl Edge {
    /// Make sure the edge connects a lower-numbered point to a higher-numbered
    /// point. This makes detecting duplicate edges much easier.
    fn normalize(self) -> Self {
        if self.0 < self.1 {
            self
        } else {
            Edge(self.1, self.0)
        }
    }

    /// If this edge connects a vertex to itself, there's no need to draw it.
    fn is_trivial(&self) -> bool {
        self.0 == self.1
    }
}

/// Process binary STL file from `input` and produce a `Wireframe` structure
/// describing the results.
fn wireframe_munge(mut input: impl Read + Seek) -> io::Result<Wireframe> {
    // The first 80 bytes of a binary STL file are a text header. Skip it.
    input.seek(io::SeekFrom::Current(80))?;
    // Next we have the triangle count.
    let tri_count = input.read_u32::<LittleEndian>()?;
    eprintln!("tri_count = {}", tri_count);

    // Point-to-ID mapping.
    let mut unique_points: HashMap<Vec3of, usize> = HashMap::default();
    // Points ordered by ID.
    let mut ordered_points = vec![];
    // Edges we've discovered to be non-trivial.
    let mut unique_edges: HashSet<Edge> = HashSet::default();
    // Diagnostic counters.
    let mut trivial_edges = 0;
    let mut duplicate_edges = 0;

    for _ in 0..tri_count {
        // The first 3 floats are the normal vector for the triangle, which we
        // don't use. Skip it.
        input.seek(io::SeekFrom::Current(3 * 4))?;

        let mut indices = [0; 3];
        for index in indices.iter_mut() {
            // Read the next triangle vertex and quantize it before we can
            // mistakenly use it raw.
            let mut p = quantize(read_point(&mut input)?);
            // This Z-shift drops the rook into the XY plane so that its center
            // of mass is near the origin. It's a side effect of the model
            // having been designed for 3D printing, and could be automated by
            // centering the mesh. (TODO)
            (p.2).0 -= 20.;
            // Record the existing index for `p` or assign a new one.
            *index = *unique_points.entry(p).or_insert_with(|| {
                let n = ordered_points.len();
                ordered_points.push(p);
                n
            });
        }

        // The final two bytes are an "attributes" field that has no meaning to
        // us. Skip it.
        input.seek(io::SeekFrom::Current(2))?;

        // Generate three edges between the three vertex indices.
        for &(si, ei) in &[(0, 1), (1, 2), (2, 0)] {
            let edge = Edge(indices[si], indices[ei]).normalize();

            if edge.is_trivial() {
                trivial_edges += 1;
                continue;
            }

            if !unique_edges.insert(edge) {
                duplicate_edges += 1;
                continue;
            }
        }
    }

    eprintln!("points.len: {}", ordered_points.len());
    eprintln!("edges.len: {}", unique_edges.len());
    eprintln!("trivial_edges: {}", trivial_edges);
    eprintln!("duplicate_edges: {}", duplicate_edges);

    let mut unique_edges: Vec<_> = unique_edges.into_iter().collect();
    unique_edges.sort_unstable();

    Ok(Wireframe {
        trivial_edges,
        duplicate_edges,
        points: ordered_points,
        edges: unique_edges,
    })
}

struct Wireframe {
    pub trivial_edges: usize,
    pub duplicate_edges: usize,
    pub points: Vec<Vec3of>,
    pub edges: Vec<Edge>,
}

/// Reads a binary STL file on `input` and produces Rust code representing its
/// vertices and connectivity on `output`.
pub fn generate_wireframe(
    input: impl Read + Seek,
    mut output: impl Write,
) -> io::Result<()> {
    let munged = wireframe_munge(input)?;

    writeln!(output, "// {} trivial edges removed", munged.trivial_edges)?;
    writeln!(
        output,
        "// {} duplicate edges removed",
        munged.duplicate_edges
    )?;
    writeln!(output, "use math::{{Vec3, Vec3f}};")?;

    writeln!(
        output,
        "pub const VERTEX_COUNT: usize = {};",
        munged.points.len()
    )?;
    writeln!(output, "pub static VERTICES: [Vec3f; VERTEX_COUNT] = [")?;
    for p in munged.points {
        writeln!(output, "    Vec3({}f32, {}f32, {}f32),", p.0, p.1, p.2)?;
    }
    writeln!(output, "];")?;

    writeln!(
        output,
        "pub static EDGES: [(u16, u16); {}] = [",
        munged.edges.len()
    )?;
    for Edge(start, end) in munged.edges {
        writeln!(output, "    ({}, {}),", start, end)?;
    }
    writeln!(output, "];")?;

    Ok(())
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
struct Tri(usize, usize, usize);

impl Tri {
    /// Note that this flips the vertex ordering, because our solid renderer
    /// assumes counter-clockwise ordering because I'm inconsistent.
    fn new(a: usize, b: usize, c: usize) -> Self {
        if a < b && a < c {
            Tri(c, b, a)
        } else if b < a && b < c {
            Tri(a, c, b)
        } else {
            Tri(b, a, c)
        }
    }
}

fn center_cloud(points: &mut [Vec3of]) {
    let mut center = Vec3(0., 0., 0.);

    for p in points.iter() {
        center = center + Vec3((p.0).0, (p.1).0, (p.2).0);
    }

    let n = points.len() as f32;
    let center = Vec3(center.0 / n, center.1 / n, center.2 / n);

    for p in points {
        let shifted = Vec3((p.0).0, (p.1).0, (p.2).0) - center;
        *p = Vec3(
            OrderedFloat(shifted.0),
            OrderedFloat(shifted.1),
            OrderedFloat(shifted.2),
        );
    }
}

/// Process binary STL file from `input` and produce a `Solid` structure
/// describing the results.
fn solid_munge(mut input: impl Read + Seek) -> io::Result<Solid> {
    // The first 80 bytes of a binary STL file are a text header. Skip it.
    input.seek(io::SeekFrom::Current(80))?;
    // Next we have the triangle count.
    let tri_count = input.read_u32::<LittleEndian>()?;
    eprintln!("tri_count = {}", tri_count);

    // Point-to-ID mapping.
    let mut unique_points: HashMap<Vec3of, usize> = HashMap::default();
    // Points ordered by ID.
    let mut ordered_points = vec![];
    // Triangles we've discovered to be non-trivial.
    let mut unique_tris: HashSet<Tri> = HashSet::default();
    // Diagnostic counters.
    let mut trivial_tris = 0;
    let mut duplicate_tris = 0;

    for _ in 0..tri_count {
        // The first 3 floats are the normal vector for the triangle, which we
        // don't use. Skip it.
        input.seek(io::SeekFrom::Current(3 * 4))?;

        let mut indices = [0; 3];
        for index in indices.iter_mut() {
            // Read the next triangle vertex and quantize it before we can
            // mistakenly use it raw.
            let p = quantize(read_point(&mut input)?);
            // Record the existing index for `p` or assign a new one.
            *index = *unique_points.entry(p).or_insert_with(|| {
                let n = ordered_points.len();
                ordered_points.push(p);
                n
            });
        }

        // The final two bytes are an "attributes" field that has no meaning to
        // us. Skip it.
        input.seek(io::SeekFrom::Current(2))?;

        if indices[0] == indices[1] && indices[0] == indices[2] {
            trivial_tris += 1;
            continue;
        }

        // Record the triangle.
        if !unique_tris.insert(Tri::new(indices[0], indices[1], indices[2])) {
            duplicate_tris += 1;
        }
    }

    eprintln!("points.len: {}", ordered_points.len());
    eprintln!("edges.len: {}", unique_tris.len());
    eprintln!("trivial_tris: {}", trivial_tris);
    eprintln!("duplicate_tris: {}", duplicate_tris);

    let mut unique_tris: Vec<_> = unique_tris
        .into_iter()
        .enumerate()
        .map(|(i, t)| (t, i as u8))
        .collect();
    unique_tris.sort_unstable();

    center_cloud(&mut ordered_points);

    Ok(Solid {
        trivial_tris,
        duplicate_tris,
        points: ordered_points,
        tris: unique_tris,
    })
}

struct Solid {
    pub trivial_tris: usize,
    pub duplicate_tris: usize,
    pub points: Vec<Vec3of>,
    pub tris: Vec<(Tri, u8)>,
}

/// Reads a binary STL file on `input` and produces Rust code representing its
/// vertices and connectivity on `output`.
pub fn generate_solid(
    input: impl Read + Seek,
    mut output: impl Write,
) -> io::Result<()> {
    let munged = solid_munge(input)?;

    writeln!(output, "// {} trivial tris removed", munged.trivial_tris)?;
    writeln!(
        output,
        "// {} duplicate tris removed",
        munged.duplicate_tris
    )?;
    writeln!(output, "use math::{{Vec3, Vec3f}};")?;
    writeln!(output, "use crate::render::Tri;")?;

    writeln!(
        output,
        "pub const VERTEX_COUNT: usize = {};",
        munged.points.len()
    )?;
    writeln!(output, "pub static VERTICES: [Vec3f; VERTEX_COUNT] = [")?;
    for p in munged.points {
        writeln!(output, "    Vec3({}f32, {}f32, {}f32),", p.0, p.1, p.2)?;
    }
    writeln!(output, "];")?;

    writeln!(output, "pub static TRIS: [Tri; {}] = [", munged.tris.len())?;
    for (Tri(a, b, c), color) in munged.tris {
        writeln!(output, "    Tri {{")?;
        writeln!(output, "        vertex_indices: [{}, {}, {}],", a, b, c)?;
        writeln!(output, "        color: {},", color + 28)?;
        writeln!(output, "    }},")?;
    }
    writeln!(output, "];")?;

    Ok(())
}
