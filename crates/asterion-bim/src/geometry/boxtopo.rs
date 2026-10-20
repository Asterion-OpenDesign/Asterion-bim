//! Topology counts of a box with boxes removed from it (face, edge and vertex counts of a wall's solid once openings
//! and cuts have been taken out of it).
//!
//! OBSERVED (public API on the solid of a wall).
//! * the edges and the vertices are those of the solid (its polyhedron: a vertex is a corner of the solid, an edge runs
//!   between two vertices);
//! * the faces are reported as the six faces of the original box, each counted once however many pieces a cut leaves of it,
//!   plus one face for every connected patch of the faces the cuts create (a wall with a door reports 9: the six plus the
//!   two jambs and the head). ENGINE CHOICE: the patches whose normal is vertical are merged per plane even when they are
//!   not connected; that is the one reading that fits all six observations (a door, a groove, a notch, two overlapping
//!   grooves, two separate grooves, and a door crossed by a horizontal and a vertical reveal).
//!
//! The solid is a grid: the coordinates of the box and of every removed box cut it into cells, a cell is filled when it
//! is inside the box and in no removed box.

use std::collections::BTreeMap;

/// An axis-aligned box `[u0, u1, v0, v1, z0, z1]`.
pub type Aabb = [f64; 6];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub faces: usize,
    pub edges: usize,
    pub vertices: usize,
}

fn axis_coords(outer: Aabb, removed: &[Aabb], a: usize) -> Vec<f64> {
    let mut v = vec![outer[2 * a], outer[2 * a + 1]];
    for r in removed {
        for c in [r[2 * a], r[2 * a + 1]] {
            if c > outer[2 * a] + 1e-9 && c < outer[2 * a + 1] - 1e-9 {
                v.push(c);
            }
        }
    }
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    v
}

/// The counts of `outer` minus `removed`.
pub fn counts(outer: Aabb, removed: &[Aabb]) -> Counts {
    let xs = [axis_coords(outer, removed, 0), axis_coords(outer, removed, 1), axis_coords(outer, removed, 2)];
    let n = [xs[0].len() - 1, xs[1].len() - 1, xs[2].len() - 1];
    let mid = |a: usize, i: usize| 0.5 * (xs[a][i] + xs[a][i + 1]);
    let mut filled = vec![false; n[0] * n[1] * n[2]];
    let idx = |i: usize, j: usize, k: usize| (i * n[1] + j) * n[2] + k;
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                let p = [mid(0, i), mid(1, j), mid(2, k)];
                let cut = removed.iter().any(|r| (0..3).all(|a| p[a] > r[2 * a] + 1e-12 && p[a] < r[2 * a + 1] - 1e-12));
                filled[idx(i, j, k)] = !cut;
            }
        }
    }
    let fill = |i: i64, j: i64, k: i64| -> bool {
        if i < 0 || j < 0 || k < 0 || i >= n[0] as i64 || j >= n[1] as i64 || k >= n[2] as i64 {
            false
        } else {
            filled[idx(i as usize, j as usize, k as usize)]
        }
    };

    // ---- faces: connected patches of the boundary faces that are not on a face of the original box
    // key: (axis, plane index, side) ; a patch is a set of unit faces connected through shared edges
    let mut unit: BTreeMap<(usize, usize, i8, usize, usize), usize> = BTreeMap::new();
    let mut parent: Vec<usize> = Vec::new();
    for i in 0..n[0] {
        for j in 0..n[1] {
            for k in 0..n[2] {
                if !filled[idx(i, j, k)] {
                    continue;
                }
                let c = [i as i64, j as i64, k as i64];
                for a in 0..3 {
                    for side in [-1i64, 1] {
                        let mut nb = c;
                        nb[a] += side;
                        if fill(nb[0], nb[1], nb[2]) {
                            continue;
                        }
                        let plane = if side < 0 { c[a] } else { c[a] + 1 } as usize;
                        let on_outer = (plane == 0 && side < 0) || (plane == n[a] && side > 0);
                        if on_outer {
                            continue;
                        }
                        let (b1, b2) = match a {
                            0 => (1, 2),
                            1 => (0, 2),
                            _ => (0, 1),
                        };
                        let key = (a, plane, side as i8, c[b1] as usize, c[b2] as usize);
                        let id = parent.len();
                        parent.push(id);
                        unit.insert(key, id);
                    }
                }
            }
        }
    }
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let nx = p[c];
            p[c] = r;
            c = nx;
        }
        r
    }
    let keys: Vec<_> = unit.keys().copied().collect();
    for (a, plane, side, p1, p2) in keys {
        let me = unit[&(a, plane, side, p1, p2)];
        for (d1, d2) in [(1usize, 0usize), (0, 1)] {
            if let Some(o) = unit.get(&(a, plane, side, p1 + d1, p2 + d2)) {
                let (x, y) = (find(&mut parent, me), find(&mut parent, *o));
                parent[x] = y;
            }
        }
    }
    // the vertical-normal patches of one plane are one patch
    let mut by_plane: BTreeMap<(usize, i8), usize> = BTreeMap::new();
    for (key, id) in &unit {
        if key.0 == 2 {
            match by_plane.get(&(key.1, key.2)) {
                Some(first) => {
                    let (x, y) = (find(&mut parent, *id), find(&mut parent, *first));
                    parent[x] = y;
                }
                None => {
                    by_plane.insert((key.1, key.2), *id);
                }
            }
        }
    }
    let mut roots = std::collections::BTreeSet::new();
    for id in 0..parent.len() {
        let r = find(&mut parent, id);
        roots.insert(r);
    }
    let faces = roots.len();

    // ---- edges and vertices of the polyhedron
    // a unit edge along axis `a` starting at lattice point `p`
    let edge_true = |a: usize, p: [i64; 3]| -> bool {
        let (b1, b2) = match a {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let mut f = [false; 4];
        let mut m = 0;
        for (s1, o1) in [(0usize, -1i64), (1, 0)] {
            for (s2, o2) in [(0usize, -1i64), (1, 0)] {
                let mut c = p;
                c[b1] += o1;
                c[b2] += o2;
                let v = fill(c[0], c[1], c[2]);
                f[s1 * 2 + s2] = v;
                if v {
                    m += 1;
                }
            }
        }
        match m {
            1 | 3 => true,
            2 => f[0] == f[3], // the two filled cells are diagonal
            _ => false,
        }
    };
    let mut unit_edges = 0usize;
    let mut deg: BTreeMap<[i64; 3], Vec<usize>> = BTreeMap::new();
    for i in 0..=n[0] as i64 {
        for j in 0..=n[1] as i64 {
            for k in 0..=n[2] as i64 {
                let p = [i, j, k];
                for a in 0..3 {
                    if p[a] >= n[a] as i64 {
                        continue;
                    }
                    if edge_true(a, p) {
                        unit_edges += 1;
                        let mut q = p;
                        q[a] += 1;
                        deg.entry(p).or_default().push(a);
                        deg.entry(q).or_default().push(a);
                    }
                }
            }
        }
    }
    let mut vertices = 0usize;
    let mut merges = 0usize;
    for axes in deg.values() {
        let straight = axes.len() == 2 && axes[0] == axes[1];
        if straight {
            merges += 1;
        } else {
            vertices += 1;
        }
    }
    Counts { faces, edges: unit_edges - merges, vertices }
}

/// What `cuts` add to the counts of `outer` minus `openings`: `(faces, edges, vertices)` more.
pub fn cut_delta(outer: Aabb, openings: &[Aabb], cuts: &[Aabb]) -> Counts {
    let base = counts(outer, openings);
    let all: Vec<Aabb> = openings.iter().chain(cuts).copied().collect();
    let with = counts(outer, &all);
    Counts { faces: with.faces.saturating_sub(base.faces), edges: with.edges.saturating_sub(base.edges), vertices: with.vertices.saturating_sub(base.vertices) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: Aabb = [0.0, 6.0, -0.1, 0.1, 0.0, 3.0];

    #[test]
    fn a_box_and_a_wall_with_a_door_have_the_observed_counts() {
        assert_eq!(counts(WALL, &[]), Counts { faces: 0, edges: 12, vertices: 8 });
        // the door: 9 faces reported = 6 + 3 new, 24 edges, 16 vertices
        assert_eq!(counts(WALL, &[[2.5, 3.5, -1.0, 1.0, 0.0, 2.0]]), Counts { faces: 3, edges: 24, vertices: 16 });
    }

    #[test]
    fn grooves_notches_and_overlaps_add_the_observed_counts() {
        let c = |r: &[Aabb]| cut_delta(WALL, &[], r);
        // a groove along the whole length: 3 faces, 12 edges, 8 vertices
        let g = [0.0, 6.0, -0.0524, 0.1, 1.0, 1.1524];
        assert_eq!(c(&[g]), Counts { faces: 3, edges: 12, vertices: 8 });
        // a notch at the top: 2, 6, 4
        assert_eq!(c(&[[0.0, 6.0, -0.0524, 0.1, 2.9, 3.0]]), Counts { faces: 2, edges: 6, vertices: 4 });
        // two overlapping grooves make one; two separate ones make two
        assert_eq!(c(&[g, [0.0, 6.0, -0.0524, 0.1, 1.1, 1.2524]]), Counts { faces: 3, edges: 12, vertices: 8 });
        assert_eq!(c(&[g, [0.0, 6.0, -0.0524, 0.1, 2.0, 2.1524]]), Counts { faces: 6, edges: 24, vertices: 16 });
    }

    #[test]
    fn a_groove_that_stops_short_of_the_end_adds_a_face() {
        let outer = [0.0, 6.1, -0.1, 0.1, 0.0, 3.0];
        // the end wall of the groove is one more face; the polyhedron has the same edges and vertices as a through one
        assert_eq!(cut_delta(outer, &[], &[[0.0, 6.0524, -0.1, 0.0524, 0.5, 0.6524]]), Counts { faces: 4, edges: 12, vertices: 8 });
        assert_eq!(cut_delta([0.0, 5.0, -0.1, 0.1, 0.0, 3.0], &[], &[[0.0, 5.0, -0.1, 0.0524, 0.5, 0.6524]]), Counts { faces: 3, edges: 12, vertices: 8 });
    }

    #[test]
    fn a_horizontal_and_a_vertical_reveal_across_a_door() {
        let door = [2.5, 3.5, -1.0, 1.0, 0.0, 2.0];
        let h = [0.0, 6.0, -0.0524, 0.1, 1.0, 1.1524];
        let v = [1.8476, 2.0, -0.0524, 0.1, 0.0, 3.0];
        // OBSERVED: the wall with the door and both reveals reports 17 faces, 72 edges, 48 vertices
        let d = cut_delta(WALL, &[door], &[h, v]);
        assert_eq!(counts(WALL, &[door]).faces + d.faces, 3 + 8);
        assert_eq!((d.edges + 24, d.vertices + 16), (72, 48));
    }
}
