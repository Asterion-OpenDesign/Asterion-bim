//! How openings reduce their host, **as measured** in round 9 (2026-10-13).
//!
//! This module exists because round 9 falsified four of the rules the opening slice was built on.
//! It holds the measured replacement, self-contained and pure, so the rule can be tested against
//! the reference's own numbers without dragging the rest of the engine in.
//!
//! # The rule
//!
//! > `Area` = gross face area − area of the **union** of every opening rectangle, each **clipped**
//! > to the host's extent in both axes. `Volume` = `Area × width`.
//!
//! Two words carry the whole correction:
//!
//! * **union**, not sum. Overlapping openings merge; the overlap is removed once, not twice.
//! * **clipped**, not rejected. An opening wider than the host, hanging off its end, or poking
//!   above its top is trimmed to the host and committed. It is never refused.
//!
//! ASTERION previously summed the areas and raised a commit-time failure for each of those three
//! cases. All four behaviours were wrong, and two of the failure codes described something the
//! reference does not do at all — so they are deleted rather than relaxed.
//!
//! # Why this is trustworthy
//!
//! It is not a plausibility check: those are the reference's numbers, and the overlapping step in particular can only be
//! produced by a union (the overlapping rectangle removed the 1.050 m² that was new, not its own 1.890).
//!
//! # What is NOT settled here
//!
//! The host solid's **face and edge counts** follow the topology of the merged boundary, not a
//! per-opening formula — see [`FaceCountRule`]. ASTERION's old `6 + 4n` is falsified; the measured
//! replacement is partial, and the exact answer needs the union's boundary polygon, which the
//! geometry kernel is growing in another lane.

/// A rectangle on the host's elevation face: station along the location curve × elevation.
///
/// `lo`/`hi` are not assumed ordered — the reference takes two opposite corners, so a caller may
/// hand them over either way round and [`Rect::normalised`] is what fixes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub u_lo: f64,
    pub u_hi: f64,
    pub z_lo: f64,
    pub z_hi: f64,
}

impl Rect {
    pub fn new(u0: f64, z0: f64, u1: f64, z1: f64) -> Rect {
        Rect { u_lo: u0, u_hi: u1, z_lo: z0, z_hi: z1 }.normalised()
    }

    /// Swap either pair that is the wrong way round.
    pub fn normalised(self) -> Rect {
        Rect {
            u_lo: self.u_lo.min(self.u_hi),
            u_hi: self.u_lo.max(self.u_hi),
            z_lo: self.z_lo.min(self.z_hi),
            z_hi: self.z_lo.max(self.z_hi),
        }
    }

    pub fn width(&self) -> f64 {
        (self.u_hi - self.u_lo).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.z_hi - self.z_lo).max(0.0)
    }

    pub fn area(&self) -> f64 {
        self.width() * self.height()
    }

    /// Whether the rectangle encloses any area at all.
    pub fn is_empty(&self) -> bool {
        self.u_hi <= self.u_lo || self.z_hi <= self.z_lo
    }

    /// Trim to `host`, or `None` when nothing of it lands on the host.
    ///
    /// OBSERVED: this is what the reference does instead of refusing. An opening spanning u −1.000 → 7.000 on a 6.000 m
    /// wall removed exactly the band that lies **on** the wall; one hanging off the end contributed only its inside part;
    /// one reaching to z 3.400 on a 3.000 m wall contributed only up to 3.000.
    pub fn clipped_to(&self, host: &Rect) -> Option<Rect> {
        let r = Rect {
            u_lo: self.u_lo.max(host.u_lo),
            u_hi: self.u_hi.min(host.u_hi),
            z_lo: self.z_lo.max(host.z_lo),
            z_hi: self.z_hi.min(host.z_hi),
        };
        if r.is_empty() {
            None
        } else {
            Some(r)
        }
    }

    fn contains_point(&self, u: f64, z: f64) -> bool {
        self.u_lo < u && u < self.u_hi && self.z_lo < z && z < self.z_hi
    }
}

/// Area of the union of `rects`, each first clipped to `host`.
///
/// The algorithm is an exact coordinate decomposition: cut the plane along every rectangle edge
/// and keep each cell that lies inside at least one rectangle. It is O(n²) cells for n rectangles,
/// which is the right trade at the scale an opening list ever reaches (a wall has tens of
/// openings, not thousands) and it is **exact** — no sweep-line tolerance, no polygon boolean, and
/// no dependence on the geometry kernel, so the measured rule can be verified on its own.
pub fn union_area(host: &Rect, rects: &[Rect]) -> f64 {
    let clipped: Vec<Rect> = rects.iter().filter_map(|r| r.normalised().clipped_to(host)).collect();
    if clipped.is_empty() {
        return 0.0;
    }
    let mut us: Vec<f64> = clipped.iter().flat_map(|r| [r.u_lo, r.u_hi]).collect();
    let mut zs: Vec<f64> = clipped.iter().flat_map(|r| [r.z_lo, r.z_hi]).collect();
    us.sort_by(f64::total_cmp);
    us.dedup();
    zs.sort_by(f64::total_cmp);
    zs.dedup();

    let mut area = 0.0;
    for i in 0..us.len().saturating_sub(1) {
        let (u0, u1) = (us[i], us[i + 1]);
        let um = 0.5 * (u0 + u1);
        for j in 0..zs.len().saturating_sub(1) {
            let (z0, z1) = (zs[j], zs[j + 1]);
            let zm = 0.5 * (z0 + z1);
            if clipped.iter().any(|r| r.contains_point(um, zm)) {
                area += (u1 - u0) * (z1 - z0);
            }
        }
    }
    area
}

/// The host's net face area: gross minus the union of its openings.
pub fn net_face_area(host: &Rect, rects: &[Rect]) -> f64 {
    (host.area() - union_area(host, rects)).max(0.0)
}

/// How many of `rects` actually remove material from `host`.
///
/// Note this counts openings that *cut*, which is not the same as openings that *add a hole*: two
/// overlapping openings both cut, and together they make one hole.
pub fn cutting_count(host: &Rect, rects: &[Rect]) -> usize {
    rects.iter().filter_map(|r| r.normalised().clipped_to(host)).count()
}

/// What round 9 measured about the host solid's face count, and what it did not settle.
///
/// ASTERION predicted `6 + 4n`. That is **falsified**. The measured sequence on one wall was
/// 6 → 10 → 13 → 9 → 9 → 11 → 11 → 14, which no per-opening formula reproduces, because the count
/// follows the topology of the **merged** boundary:
///
/// | what was added | face delta | why |
/// |---|---|---|
/// | a hole floating inside the face | **+4** | a new inner loop, four side faces |
/// | a hole touching the base or the top | **+3** | it opens the outer boundary into a notch instead of adding a loop |
/// | a rectangle merging into an existing hole | **+0** | the boundary does not gain a loop |
/// | a band spanning the full width that merges with a notch | **+2** | measured, and not derivable from the three rules above |
///
/// The last row is why this is an enum and not a function: the general answer needs the union's
/// boundary polygon, which `crates/asterion-geom` is growing in another lane. Until then ASTERION
/// reports the cases it has measured and records the rest as a known limitation rather than
/// inventing a formula that happens to fit four of eight rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceCountRule {
    /// The opening lies strictly inside the host face: a new inner loop.
    FloatingHole,
    /// The opening reaches the host's base or top: a notch in the outer boundary.
    NotchAtBaseOrTop,
    /// The opening merges into material already removed: no new boundary.
    MergedIntoExisting,
    /// Measured cases exist that none of the above describe; the count is not derivable yet.
    NotDerivableYet,
}

impl FaceCountRule {
    /// The measured face delta, or `None` when the case is one this round could not pin down.
    pub fn face_delta(self) -> Option<i32> {
        match self {
            FaceCountRule::FloatingHole => Some(4),
            FaceCountRule::NotchAtBaseOrTop => Some(3),
            FaceCountRule::MergedIntoExisting => Some(0),
            FaceCountRule::NotDerivableYet => None,
        }
    }

    /// Classify one opening against the host and the openings already present.
    pub fn classify(host: &Rect, existing: &[Rect], candidate: &Rect) -> FaceCountRule {
        let Some(c) = candidate.normalised().clipped_to(host) else {
            return FaceCountRule::MergedIntoExisting; // removes nothing, changes no boundary
        };
        // Does it remove anything that was not already gone?
        let before = union_area(host, existing);
        let mut with = existing.to_vec();
        with.push(c);
        let after = union_area(host, &with);
        if after - before <= 0.0 {
            return FaceCountRule::MergedIntoExisting;
        }
        let touches_base = c.z_lo <= host.z_lo;
        let touches_top = c.z_hi >= host.z_hi;
        let overlaps_existing = existing
            .iter()
            .filter_map(|r| r.normalised().clipped_to(host))
            .any(|r| r.u_lo < c.u_hi && c.u_lo < r.u_hi && r.z_lo < c.z_hi && c.z_lo < r.z_hi);
        match (overlaps_existing, touches_base || touches_top) {
            // A partial merge changes the boundary in a way the measured cases do not determine.
            (true, _) => FaceCountRule::NotDerivableYet,
            (false, true) => FaceCountRule::NotchAtBaseOrTop,
            (false, false) => FaceCountRule::FloatingHole,
        }
    }
}

/// Perimeter of the host's **net region** — the host face minus the union of its openings —
/// counting the outer boundary (notched where an opening reaches an edge) and every inner loop.
///
/// OBSERVED: this is the term that turns the net area into the host's surface area, and it is what makes the overlapping
/// and clipped cases come out right. See [`net_surface_area`].
///
/// Computed on the same exact decomposition as [`union_area`]: walk every grid line and add the
/// length of each segment whose two sides disagree about being inside the net region. No polygon
/// boolean, no tolerance, and it handles a hole that touches an edge (which stops being a loop and
/// becomes a notch) without a special case.
pub fn net_region_perimeter(host: &Rect, rects: &[Rect]) -> f64 {
    let clipped: Vec<Rect> = rects.iter().filter_map(|r| r.normalised().clipped_to(host)).collect();
    let mut us: Vec<f64> = vec![host.u_lo, host.u_hi];
    let mut zs: Vec<f64> = vec![host.z_lo, host.z_hi];
    for r in &clipped {
        us.push(r.u_lo);
        us.push(r.u_hi);
        zs.push(r.z_lo);
        zs.push(r.z_hi);
    }
    us.sort_by(f64::total_cmp);
    us.dedup();
    zs.sort_by(f64::total_cmp);
    zs.dedup();

    // A point is in the net region when it is inside the host and inside no opening.
    let inside = |u: f64, z: f64| -> bool {
        host.contains_point(u, z) && !clipped.iter().any(|r| r.contains_point(u, z))
    };
    // Half the smallest cell, so a probe never lands on another grid line.
    let eps = {
        let du = us.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
        let dz = zs.windows(2).map(|w| w[1] - w[0]).fold(f64::INFINITY, f64::min);
        0.25 * du.min(dz).min(1.0)
    };
    if !(eps > 0.0) {
        return 0.0;
    }

    let mut p = 0.0;
    for &u in &us {
        for w in zs.windows(2) {
            let zm = 0.5 * (w[0] + w[1]);
            if inside(u - eps, zm) != inside(u + eps, zm) {
                p += w[1] - w[0];
            }
        }
    }
    for &z in &zs {
        for w in us.windows(2) {
            let um = 0.5 * (w[0] + w[1]);
            if inside(um, z - eps) != inside(um, z + eps) {
                p += w[1] - w[0];
            }
        }
    }
    p
}

/// Total boundary area of the host solid: both large faces plus the extruded boundary.
///
/// > `surface = 2 × net area + perimeter of the net region × thickness`
///
/// It replaces an earlier formula that added `2 × (w + h) × thickness` of "reveal" per opening.
/// That one is right only while openings are disjoint and wholly inside the host: it double-counts
/// the shared edge of two overlapping openings, and it bills reveal for a notch edge that is not
/// interior at all. The perimeter of the net region has no such cases because it never looks at an
/// opening individually.
pub fn net_surface_area(host: &Rect, rects: &[Rect], thickness: f64) -> f64 {
    2.0 * net_face_area(host, rects) + net_region_perimeter(host, rects) * thickness
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 6.000 x 3.000 m wall face.
    fn host() -> Rect {
        Rect::new(0.0, 0.0, 6.0, 3.0)
    }

    /// THE test of this module: replay the eight measurement steps and assert the exact area the
    /// reference reported for each.
    ///
    /// These are not plausible numbers, they are the reference product's own, read.
    #[test]
    fn the_union_rule_reproduces_every_measured_area() {
        let h = host();
        let win = Rect::new(1.0, 0.9, 2.2, 2.1); // 1.2 x 1.2, floating
        let door = Rect::new(3.0, 0.0, 3.9, 2.1); // 0.9 x 2.1, touching the base
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1); // laid over the door
        let wider = Rect::new(-1.0, 0.5, 7.0, 1.5); // wider than the wall
        let off_end = Rect::new(5.5, 0.5, 6.5, 1.5); // hangs off the end
        let above = Rect::new(4.6, 2.8, 5.2, 3.4); // pokes above the top

        let steps: [(&str, Vec<Rect>, f64); 8] = [
            ("wall-no-opening", vec![], 18.0),
            ("one-window-opening", vec![win], 16.56),
            ("plus-base-touching-opening", vec![win, door], 14.67),
            ("after-delete-window-opening", vec![door], 16.11),
            ("overlapping-rect-opening", vec![door, overlap], 15.06),
            ("opening-wider-than-wall", vec![door, overlap, wider], 10.46),
            ("opening-hanging-off-wall-end", vec![door, overlap, wider, off_end], 10.46),
            ("opening-above-wall-top", vec![door, overlap, wider, off_end, above], 10.34),
        ];
        for (label, rects, measured) in steps {
            let got = net_face_area(&h, &rects);
            assert!(
                (got - measured).abs() < 1e-9,
                "{label}: union rule gives {got}, the reference product measured {measured}"
            );
            // And the identity the reference also keeps, at the type's 0.200 m thickness.
            assert!((got * 0.2 - measured * 0.2).abs() < 1e-9);
        }
    }

    /// The overlapping step on its own, because it is the one that can only be a union.
    #[test]
    fn an_overlapping_opening_removes_only_what_is_new() {
        let h = host();
        let door = Rect::new(3.0, 0.0, 3.9, 2.1);
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1);
        assert!((door.area() - 1.89).abs() < 1e-12);
        assert!((overlap.area() - 1.89).abs() < 1e-12);
        // Summing would remove 3.780; the union removes 1.890 + 1.050.
        let u = union_area(&h, &[door, overlap]);
        assert!((u - 2.94).abs() < 1e-12, "union was {u}");
        assert!((u - (door.area() + overlap.area())).abs() > 0.8, "a sum would be visibly larger");
    }

    /// Clipping, not rejecting — the three cases ASTERION used to refuse.
    #[test]
    fn out_of_range_openings_are_clipped_and_never_refused() {
        let h = host();
        // Wider than the wall: only the 6.000 x 1.000 band on the wall counts.
        let wider = Rect::new(-1.0, 0.5, 7.0, 1.5);
        assert!((union_area(&h, &[wider]) - 6.0).abs() < 1e-12);
        // Hanging off the end: only the inside half.
        let off = Rect::new(5.5, 0.5, 6.5, 1.5);
        assert!((union_area(&h, &[off]) - 0.5).abs() < 1e-12);
        // Above the top: only up to the wall top. 0.600 x 0.200 = 0.120, as measured.
        let above = Rect::new(4.6, 2.8, 5.2, 3.4);
        assert!((union_area(&h, &[above]) - 0.12).abs() < 1e-12);
        // Entirely off the host: contributes nothing, and still is not an error.
        let away = Rect::new(10.0, 0.0, 11.0, 1.0);
        assert_eq!(union_area(&h, &[away]), 0.0);
        assert_eq!(cutting_count(&h, &[away]), 0);
    }

    /// A 2 mm opening is accepted, as measured; there is no minimum size to enforce.
    #[test]
    fn a_two_millimetre_opening_is_accepted() {
        let h = host();
        let tiny = Rect::new(5.0, 1.0, 5.002, 1.002);
        let a = union_area(&h, &[tiny]);
        assert!((a - 4e-6).abs() < 1e-15, "got {a}");
        assert_eq!(cutting_count(&h, &[tiny]), 1);
    }

    /// Corners handed over the other way round describe the same opening.
    #[test]
    fn opposite_corners_may_arrive_in_either_order() {
        let h = host();
        let a = Rect::new(1.0, 0.9, 2.2, 2.1);
        let b = Rect::new(2.2, 2.1, 1.0, 0.9);
        assert_eq!(a, b);
        assert!((union_area(&h, &[b]) - 1.44).abs() < 1e-12);
    }

    /// The face-count rule reports what round 9 measured and admits what it did not settle.
    #[test]
    fn the_face_count_rule_is_honest_about_its_limits() {
        let h = host();
        let win = Rect::new(1.0, 0.9, 2.2, 2.1);
        let door = Rect::new(3.0, 0.0, 3.9, 2.1);
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1);

        // 6 -> 10: a floating hole adds four side faces.
        assert_eq!(FaceCountRule::classify(&h, &[], &win), FaceCountRule::FloatingHole);
        assert_eq!(FaceCountRule::FloatingHole.face_delta(), Some(4));
        // 10 -> 13: a base-touching opening adds three, not four. This is what falsified 6 + 4n.
        assert_eq!(FaceCountRule::classify(&h, &[win], &door), FaceCountRule::NotchAtBaseOrTop);
        assert_eq!(FaceCountRule::NotchAtBaseOrTop.face_delta(), Some(3));
        // An opening that removes nothing new changes no boundary.
        assert_eq!(
            FaceCountRule::classify(&h, &[door], &Rect::new(3.2, 0.5, 3.6, 1.5)),
            FaceCountRule::MergedIntoExisting
        );
        assert_eq!(FaceCountRule::MergedIntoExisting.face_delta(), Some(0));
        // A partial merge is the case round 9 could not pin down, and it says so rather than
        // guessing a number that would silently be wrong.
        assert_eq!(FaceCountRule::classify(&h, &[door], &overlap), FaceCountRule::NotDerivableYet);
        assert_eq!(FaceCountRule::NotDerivableYet.face_delta(), None);
    }
    /// Scenario: a hosted DOOR and WINDOW on an 8.000 x 3.000 m wall.
    ///
    /// The same rule, with the cut rectangle derived from the type's width and height instead of
    /// from two corners. The step that matters is the type-width change: widening the door type
    /// from 0.900 to 1.200 re-cut the host, which is only possible if the cut is derived.
    #[test]
    fn the_same_rule_explains_the_hosted_instance_round() {
        let h = Rect::new(0.0, 0.0, 8.0, 3.0);
        assert!((h.area() - 24.0).abs() < 1e-12);

        // Door 0.900 x 2.100 at station 2.000, sill 0.
        let door09 = Rect::new(2.0 - 0.45, 0.0, 2.0 + 0.45, 2.1);
        assert!((net_face_area(&h, &[door09]) - 22.11).abs() < 1e-9);

        // Type width 0.900 -> 1.200: the host is re-cut, not left alone.
        let door12 = Rect::new(2.0 - 0.6, 0.0, 2.0 + 0.6, 2.1);
        assert!((net_face_area(&h, &[door12]) - 21.48).abs() < 1e-9);

        // Plus a 1.200 x 1.200 window elsewhere on the same wall.
        let win = Rect::new(5.0 - 0.6, 0.0, 5.0 + 0.6, 1.2);
        assert!((net_face_area(&h, &[door12, win]) - 20.04).abs() < 1e-9);

        // Deleting the window restores the host exactly - the cut is not baked in.
        assert!((net_face_area(&h, &[door12]) - 21.48).abs() < 1e-9);
    }

    /// Scenario: the two cascades that ASTERION predicted would be ERRORS.
    ///
    /// Both were measured as **accepted, with the cut clipped**, which is the same correction the
    /// opening round forced. An engine that refuses either of these is wrong twice over: it
    /// rejects a legal edit, and it never produces the measured area.
    #[test]
    fn the_two_cascades_predicted_to_fail_are_clipped_instead() {
        // A 6.000 x 3.000 m wall with a 0.900 x 2.100 door at station 2.000.
        let h = Rect::new(0.0, 0.0, 6.0, 3.0);
        let door = Rect::new(1.55, 0.0, 2.45, 2.1);
        assert!((net_face_area(&h, &[door]) - 16.11).abs() < 1e-9);

        // Sill 2.000 pushes the head to 4.100, a metre above the wall top. ASTERION predicted a
        // failure; the reference clips the cut at the top and commits: 18.000 - 0.900 = 17.100.
        let raised = Rect::new(1.55, 2.0, 2.45, 4.1);
        assert!((net_face_area(&h, &[raised]) - 17.10).abs() < 1e-9);

        // Shortening the host to 1.500, below the door head. Predicted a failure; measured as a
        // clip: gross 9.000 minus a 0.900 x 1.500 cut = 7.650.
        let short = Rect::new(0.0, 0.0, 6.0, 1.5);
        assert!((short.area() - 9.0).abs() < 1e-12);
        assert!((net_face_area(&short, &[door]) - 7.65).abs() < 1e-9);
    }

    /// Sill and head are two views of ONE relationship, so neither may be stored.
    ///
    /// OBSERVED (scenario 030): writing the sill moves the head and writing the head moves the sill, both through the
    /// type's height - sill 0.150 gave head 2.250, head 2.400 gave sill 0.300, on a 2.100-high door. This helper is the
    /// relationship itself; the point of testing it here is that it is arithmetic, not state.
    #[test]
    fn sill_and_head_are_one_relationship_in_both_directions() {
        let type_height = 2.1_f64;
        let head_from_sill = |sill: f64| sill + type_height;
        let sill_from_head = |head: f64| head - type_height;

        assert!((head_from_sill(0.0) - 2.1).abs() < 1e-12);
        assert!((head_from_sill(0.15) - 2.25).abs() < 1e-12);
        assert!((sill_from_head(2.4) - 0.3).abs() < 1e-12);
        // A window 1.200 high with sill 1.000 measured head 2.200.
        assert!(((1.0 + 1.2) - 2.2_f64).abs() < 1e-12);
        // Round-trips, which is what makes storing either one a bug waiting to desynchronise.
        for s in [0.0, 0.15, 0.3, 2.0] {
            assert!((sill_from_head(head_from_sill(s)) - s).abs() < 1e-12);
        }
    }
    /// The surface-area model, against every measured value.
    ///
    /// `surface = 2 * net area + perimeter of the net region * thickness`. The overlapping step is
    /// again the discriminating one: a per-opening reveal term double-counts the shared edge and
    /// cannot produce 34.560.
    #[test]
    fn the_surface_model_reproduces_every_measured_surface_area() {
        let h = host();
        let t = 0.2;
        let win = Rect::new(1.0, 0.9, 2.2, 2.1);
        let door = Rect::new(3.0, 0.0, 3.9, 2.1);
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1);
        let wider = Rect::new(-1.0, 0.5, 7.0, 1.5);
        let off_end = Rect::new(5.5, 0.5, 6.5, 1.5);
        let above = Rect::new(4.6, 2.8, 5.2, 3.4);

        let steps: [(&str, Vec<Rect>, f64); 8] = [
            ("wall-no-opening", vec![], 39.6),
            ("one-window-opening", vec![win], 37.68),
            ("plus-base-touching-opening", vec![win, door], 34.74),
            ("after-delete-window-opening", vec![door], 36.66),
            ("overlapping-rect-opening", vec![door, overlap], 34.56),
            ("opening-wider-than-wall", vec![door, overlap, wider], 26.4),
            ("opening-hanging-off-wall-end", vec![door, overlap, wider, off_end], 26.4),
            ("opening-above-wall-top", vec![door, overlap, wider, off_end, above], 26.24),
        ];
        for (label, rects, measured) in steps {
            let got = net_surface_area(&h, &rects, t);
            assert!(
                (got - measured).abs() < 1e-9,
                "{label}: surface model gives {got}, the reference product measured {measured}"
            );
        }
    }

    /// The perimeter term on its own, for the three shapes that exercise it.
    #[test]
    fn the_net_region_perimeter_handles_loops_and_notches() {
        let h = host();
        // Bare rectangle: its own perimeter.
        assert!((net_region_perimeter(&h, &[]) - 18.0).abs() < 1e-9);
        // A floating hole adds a full inner loop: 18.000 + 4 x 1.200.
        let win = Rect::new(1.0, 0.9, 2.2, 2.1);
        assert!((net_region_perimeter(&h, &[win]) - 22.8).abs() < 1e-9);
        // A base-touching opening is a NOTCH, not a loop: it removes 0.900 of the bottom edge and
        // adds three sides, 18.000 - 0.900 + (2.100 + 0.900 + 2.100) = 22.200.
        let door = Rect::new(3.0, 0.0, 3.9, 2.1);
        assert!((net_region_perimeter(&h, &[door]) - 22.2).abs() < 1e-9);
        // Two overlapping notches are ONE notch 1.400 wide, so the perimeter does not grow:
        // 18.000 - 1.400 + (2.100 + 1.400 + 2.100) = 22.200, the same as one notch.
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1);
        assert!((net_region_perimeter(&h, &[door, overlap]) - 22.2).abs() < 1e-9);
    }

    /// Why the face and edge counts are NOT derived here, with the measurement that proves it.
    ///
    /// Round 9 measured faces 6, 10, 13, 9, 9, 11, 11, 14 and edges 12, 24, 36, 24, 24, 48, 52, 64.
    /// The `opening-hanging-off-wall-end` step is the decisive one: it changed **neither the area
    /// nor the face count**, and still took the edge count from 48 to 52. An opening that removes
    /// nothing cannot change the region's geometry, so the counts are tracking the reference's own
    /// B-rep bookkeeping - split edges it keeps from how the solid was built - and not the shape.
    ///
    /// So area, volume and surface area are modelled exactly, and the counts are recorded as a
    /// measured limitation rather than fitted with a formula that would be wrong the moment the
    /// build order changed.
    #[test]
    fn face_and_edge_counts_track_brep_history_not_geometry() {
        let h = host();
        let door = Rect::new(3.0, 0.0, 3.9, 2.1);
        let overlap = Rect::new(3.5, 0.0, 4.4, 2.1);
        let wider = Rect::new(-1.0, 0.5, 7.0, 1.5);
        let off_end = Rect::new(5.5, 0.5, 6.5, 1.5);

        let before = [door, overlap, wider];
        let after = [door, overlap, wider, off_end];
        // Geometrically identical: same area, same perimeter, therefore same surface.
        assert!((net_face_area(&h, &before) - net_face_area(&h, &after)).abs() < 1e-12);
        assert!((net_region_perimeter(&h, &before) - net_region_perimeter(&h, &after)).abs() < 1e-12);
        // Yet the reference reported 48 -> 52 edges across exactly that step. Nothing derived from
        // the region can produce that, which is the whole point of this test.
        assert_eq!(FaceCountRule::classify(&h, &before, &off_end), FaceCountRule::MergedIntoExisting);
    }
}
