//! Join Geometry: which of two joined elements cuts the other.
//!
//! Every rule carries its rule; nothing here comes from a measurement.
//!
//! # The relation (Spec F-05a)
//!
//! `JoinGeometry(first, second)` decides the cut order ([`resolve_cut_order`]) and then adds ONE
//! cut-pair step, held by the CUT element and naming the CUTTER. Nothing is added when the holder
//! already holds a step naming the other element. The cleaning bookkeeping is symmetric (a
//! clean-pair step on both elements) but the cut step is not. So:
//!
//! * the element that holds the step is the cut element, the id inside the step is the cutter;
//! * When `a` is the cutter of `b` ([`crate::BimModel::is_cutter_of_join`]).
//! * the engine stores the relation as an unordered pair plus the cutter
//!   ([`crate::model::GeometryJoin`]), which is the same information.
//!
//! # The order
//!
//! Writes two flags. Flag "A is the cut element" is TRUE when B cuts A: the step is then stored on A, otherwise the pair is
//! swapped and the step is stored on B, so with no rule matching the FIRST element cuts the SECOND. Flag "no cut step"
//! suppresses the cut step (only the clean-pair steps are added). The 11 rules are tested in the order
//! [`resolve_cut_order`] states them. Consequences read from the table: a wall is cut by a floor, ceiling, roof or
//! toposolid whichever order they are given in; a column is cut by a wall in either order; wall against wall and any pair
//! not listed make the first element the cutter.
//!
//! # What is NOT specified (open, so NOT guessed)
//!
//! UNKNOWN: the three virtual predicates other DLLs answer (the "concrete" test at the reference product, the coping rank
//! test, the stairs test at the reference product), the wall-sweep internals and the exact membership of "Floor-class" and
//! "Toposolid-class". They are explicit INPUTS of [`CutOrderElement`]; the engine models none of the element kinds that
//! need them and leaves them `false`.

use asterion_core::ElementId;

/// ASTERION's own names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JoinCategory {
    /// -2000011.
    Wall,
    /// -2003500.
    StackedWall,
    /// -2000032.
    Floor,
    /// -2000038.
    Ceiling,
    /// -2000035.
    Roof,
    /// -2001079.
    Toposolid,
    /// -2000100.
    Column,
    /// -2000120.
    Stair,
    /// -2000181: the default category of a wall-sweep element.
    WallSweep,
    /// -2001300.
    FoundationStructural,
    /// -2001320.
    FramingStructural,
    /// -2001330.
    ColumnStructural,
    /// -2001390.
    Fascia,
    /// -2001391.
    Gutter,
    /// -2001392.
    SlabEdge,
    /// -2009030.
    ConnectionStructural,
    /// Any category the table does not name.
    #[default]
    Other,
}

impl JoinCategory {
    /// The category of a signed category id as read at the table.
    pub fn from_category_id(id: i64) -> JoinCategory {
        match id {
            -2000011 => JoinCategory::Wall,
            -2003500 => JoinCategory::StackedWall,
            -2000032 => JoinCategory::Floor,
            -2000038 => JoinCategory::Ceiling,
            -2000035 => JoinCategory::Roof,
            -2001079 => JoinCategory::Toposolid,
            -2000100 => JoinCategory::Column,
            -2000120 => JoinCategory::Stair,
            -2000181 => JoinCategory::WallSweep,
            -2001300 => JoinCategory::FoundationStructural,
            -2001320 => JoinCategory::FramingStructural,
            -2001330 => JoinCategory::ColumnStructural,
            -2001390 => JoinCategory::Fascia,
            -2001391 => JoinCategory::Gutter,
            -2001392 => JoinCategory::SlabEdge,
            -2009030 => JoinCategory::ConnectionStructural,
            _ => JoinCategory::Other,
        }
    }

    /// The categories named as "host category" in rules (2) and (5): walls, stacked walls, floors,
    /// ceilings, roofs and toposolids.
    fn is_host_category(self) -> bool {
        matches!(
            self,
            JoinCategory::Wall
                | JoinCategory::StackedWall
                | JoinCategory::Floor
                | JoinCategory::Ceiling
                | JoinCategory::Roof
                | JoinCategory::Toposolid
        )
    }

    /// Structural framing, structural columns and structural foundations: the set rules (2) and
    /// (8) name.
    fn is_structural_member(self) -> bool {
        matches!(
            self,
            JoinCategory::FramingStructural | JoinCategory::ColumnStructural | JoinCategory::FoundationStructural
        )
    }

    fn is_wall_like(self) -> bool {
        matches!(self, JoinCategory::Wall | JoinCategory::StackedWall)
    }

    fn is_edge_like(self) -> bool {
        matches!(self, JoinCategory::Fascia | JoinCategory::Gutter | JoinCategory::SlabEdge)
    }
}

/// One element of a pair, as the table sees it: its category and the predicates the table asks of
/// it. The predicates the table takes from other DLLs are inputs, never derived here (see the
/// module notes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CutOrderElement {
    pub category: JoinCategory,
    /// "Floor-class element". UNKNOWN: the membership beyond floors.
    pub floor_class: bool,
    /// "Toposolid-class element". UNKNOWN: the membership.
    pub toposolid_class: bool,
    pub structural_wall: bool,
    pub concrete: bool,
    /// Also the coping test.
    pub coping_rank: bool,
    pub wall_foundation: bool,
    pub stairs_flag: bool,
    /// Either element being one sets "no cut step".
    pub thin_ceiling: bool,
    /// "a wall sweep that cuts the wall (or carries its own flag) is hosted by the OTHER element":
    /// this element is such a sweep. Sets "no cut step".
    pub sweep_hosted_by_other: bool,
}

impl CutOrderElement {
    pub fn of(category: JoinCategory) -> CutOrderElement {
        CutOrderElement { category, ..CutOrderElement::default() }
    }

    /// The element as a floor: certainly "Floor-class".
    pub fn floor() -> CutOrderElement {
        CutOrderElement { floor_class: true, ..CutOrderElement::of(JoinCategory::Floor) }
    }

    fn rank(&self) -> u8 {
        if self.floor_class {
            3
        } else if self.coping_rank {
            2
        } else if self.wall_foundation {
            1
        } else {
            0
        }
    }
}

/// The two flags writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutOrder {
    /// "A is the cut element": B cuts A.
    pub a_is_cut: bool,
    /// "no cut step": no cut-pair step is added, only the clean-pair steps.
    pub no_cut_step: bool,
}

impl CutOrder {
    /// `(cutter, cut)` for the pair `(a, b)`: the step is stored on the CUT element and names the CUTTER. `None` when no
    /// cut step is added.
    pub fn cutter_and_cut(self, a: ElementId, b: ElementId) -> Option<(ElementId, ElementId)> {
        if self.no_cut_step {
            None
        } else if self.a_is_cut {
            Some((b, a))
        } else {
            Some((a, b))
        }
    }
}

/// The 15 stored pairs of category ids (the lower id first, the table is sorted. Walls with walls, an architectural column
/// (-2000100) with a wall, structural framing / columns / foundation among themselves and with floors and walls,
/// connections with columns and walls, slab edges with floors and toposolids, and the pair (-2003400, -2003400). A wall
/// with a floor, ceiling or roof is NOT in the table).
pub const AUTO_JOINABLE_CATEGORY_PAIRS: [(i64, i64); 15] = [
    (-2000011, -2000011),
    (-2000100, -2000011),
    (-2001320, -2001320),
    (-2001330, -2001320),
    (-2001320, -2001300),
    (-2001300, -2001300),
    (-2001320, -2000032),
    (-2001330, -2000032),
    (-2001320, -2000011),
    (-2001330, -2000011),
    (-2009030, -2001330),
    (-2009030, -2000011),
    (-2001392, -2000032),
    (-2001392, -2001079),
    (-2003400, -2003400),
];

/// Whether two category ids are auto-joinable, in either argument order.
pub fn auto_joinable(category_a: i64, category_b: i64) -> bool {
    AUTO_JOINABLE_CATEGORY_PAIRS.iter().any(|&(x, y)| (x == category_a && y == category_b) || (x == category_b && y == category_a))
}

/// `user_join` is true on the public API path. Evaluation:
///
/// * "no cut step" (and an immediate return) when either element is a thin ceiling, or a wall sweep
///   that cuts the wall is hosted by the other element;
/// * otherwise "A is the cut element" (B cuts A) is TRUE for the first of these rules that matches,
///   tested in this order, and FALSE when none does:
///   1. A is a column and B is a wall or stacked wall;
///   2. A is a wall, stacked wall, floor, ceiling, roof or toposolid and B is structural framing,
///      structural columns or structural foundation: TRUE, except that when A is Floor-class or a
///      structural wall AND B's "concrete" predicate holds, evaluation continues with the rules
///      below;
///   3. A is a wall or stacked wall and B is a floor, ceiling, roof or toposolid;
///   4. not `user_join`, A is a slab edge and B is Floor-class or Toposolid-class;
///   5. A is a host category of (2) and B is a fascia, gutter or slab edge: TRUE, except that when
///      `user_join`, B is a slab edge and A is Floor-class or Toposolid-class evaluation continues;
///   6. A is a wall sweep and B is not a wall sweep, not fascia/gutter/slab edge, not a wall or
///      stacked wall;
///   7. A is structural framing and B is structural columns or structural foundation;
///   8. A is structural framing, columns or foundation, B is Floor-class or a structural wall, and
///      A's "concrete" predicate holds;
///   9. A is structural connections and B is not;
///  10. A is Floor-class, B is stairs and B's stairs predicate holds;
pub fn resolve_cut_order(user_join: bool, a: &CutOrderElement, b: &CutOrderElement) -> CutOrder {
    let (ca, cb) = (a.category, b.category);
    if a.thin_ceiling || b.thin_ceiling || a.sweep_hosted_by_other || b.sweep_hosted_by_other {
        return CutOrder { a_is_cut: false, no_cut_step: true };
    }
    let cut = |a_is_cut: bool| CutOrder { a_is_cut, no_cut_step: false };

    // (1)
    if ca == JoinCategory::Column && cb.is_wall_like() {
        return cut(true);
    }
    // (2)
    if ca.is_host_category() && cb.is_structural_member() {
        let continues = (a.floor_class || a.structural_wall) && b.concrete;
        if !continues {
            return cut(true);
        }
    }
    // (3)
    if ca.is_wall_like()
        && matches!(
            cb,
            JoinCategory::Floor | JoinCategory::Ceiling | JoinCategory::Roof | JoinCategory::Toposolid
        )
    {
        return cut(true);
    }
    // (4)
    if !user_join && ca == JoinCategory::SlabEdge && (b.floor_class || b.toposolid_class) {
        return cut(true);
    }
    // (5)
    if ca.is_host_category() && cb.is_edge_like() {
        let continues = user_join && cb == JoinCategory::SlabEdge && (a.floor_class || a.toposolid_class);
        if !continues {
            return cut(true);
        }
    }
    // (6)
    if ca == JoinCategory::WallSweep && cb != JoinCategory::WallSweep && !cb.is_edge_like() && !cb.is_wall_like() {
        return cut(true);
    }
    // (7)
    if ca == JoinCategory::FramingStructural
        && matches!(cb, JoinCategory::ColumnStructural | JoinCategory::FoundationStructural)
    {
        return cut(true);
    }
    // (8)
    if ca.is_structural_member() && (b.floor_class || b.structural_wall) && a.concrete {
        return cut(true);
    }
    // (9)
    if ca == JoinCategory::ConnectionStructural && cb != JoinCategory::ConnectionStructural {
        return cut(true);
    }
    // (10)
    if a.floor_class && cb == JoinCategory::Stair && b.stairs_flag {
        return cut(true);
    }
    // (11)
    if ca == JoinCategory::FoundationStructural && cb == JoinCategory::FoundationStructural && a.rank() < b.rank() {
        return cut(true);
    }
    cut(false)
}

/// The order of the WALLS of a wall join: which wall is first, which two walls are the geometric neighbours of a wall, and
/// the join-order predicate.
///
/// Everything here is a pure function over an EXPLICIT array of join entries. UNKNOWN: the order of the list and where the
/// predicate is applied. [`crate::joins::join_list_precedes`] is the engine's own order.
///
/// * The FIRST wall of a join is entry 0 of the array of its end ([`reduce_span_entries`] and [`join_walls_on_one_z_span`]
///   are the only reorderings/removals the reference product applies).
/// * The two geometric neighbours of a wall at array position `k` are chosen among the walls that
///   PRECEDE it ([`geom_ordered_wall_neighbours`]); the element id breaks only exact geometric
///   ties between candidates.
/// * A stacked wall never receives a join step ([`stacked_wall_gets_a_join_step`]).
/// * The join-order predicate reads the wall TYPE's rank in the document's wall join default
///   setting, then the wall function; it reads no element id
///   ([`new_order_ranks_above_first`]).
pub mod wall_join_list {
    use asterion_core::ElementId;
    use asterion_geom::Vec2;
    use std::cmp::Ordering;
    use std::ops::Range;

    /// 0.1 degree in radians, the angle tie tolerance and the "within 0.1 degree of zero" limit of the neighbour comparator
    /// (the constant is the one the rule quotes).
    pub const ANGLE_TIE_LIMIT: f64 = 0.0017453292519943279;

    /// An angle above this value (2 pi minus 0.1 degree) is replaced by 0.
    pub const ANGLE_WRAP_TO_ZERO: f64 = 6.28144;

    /// The `|sine|` below which the parallel-wall replacement of entry 0 is skipped (0.1 degree), and the margin by which a
    /// candidate must beat the running best.
    pub const SPAN_SINE_LIMIT: f64 = 0.0017453292519943279;
    pub const SPAN_VALUE_MARGIN: f64 = 1e-9;

    // ------------------------------------------------------------------ : the span array

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct EndInfoEntry {
        pub wall: ElementId,
        /// 0 and 1 are the two ends of the wall's curve; 2 and 3 are its two sides (the join point
        /// lies on the wall, a "side attachment").
        pub end_index: u8,
        /// . UNKNOWN: what the boolean means; only its equality with the processed wall's own value is used.
        pub side_is_join_on: bool,
        /// Unit tangent of the entry's centre curve.
        pub tangent: Vec2,
        /// Vector from the join control point to the point at parameter 0.5 of the entry's centre
        /// curve.
        pub mid_point_vector: Vec2,
        /// The entry's wall width.
        pub width: f64,
        /// The centre curve of this entry is parallel to that of entry 0 of the array. UNKNOWN: the tolerance of the
        /// reference product's parallel test.
        pub parallel_to_first: bool,
    }

    impl EndInfoEntry {
        /// Side attachment: end index 2 or 3.
        pub fn is_side_attached(&self) -> bool {
            self.end_index == 2 || self.end_index == 3
        }
    }

    /// The only reordering read in the reference product.
    ///
    /// `entries` is the span array in array order, `wall_tangent` the tangent of the wall being regenerated at the join and
    /// `wall_side` its own.
    ///
    /// 1. When the span has MORE than two entries and entry 0 is attached at a side index, every
    ///    side-attached entry from index 1 on is removed. The removed entries whose centre curves
    ///    are parallel to that of entry 0 are compared by
    ///    `value = dot(mid_point_vector, wall_tangent) - width / |sin|`, `|sin|` being the sine of
    ///    the angle between the wall tangent and the tangent of entry 0; entry 0 is the initial
    ///    best and a candidate smaller than the running best by more than `1e-9` is written over
    ///    entry 0. No replacement is made when that `|sin|` is below [`SPAN_SINE_LIMIT`].
    /// 2. Afterwards, when entry 0 is still side-attached and more than two entries remain,
    ///    end-attached entries whose `side_is_join_on` differs from the wall's side are removed.
    ///
    /// Arrays of two or fewer entries, and arrays whose entry 0 is end-attached, are returned
    /// unchanged by this step (the later removal is [`join_walls_on_one_z_span`]).
    pub fn reduce_span_entries(entries: &[EndInfoEntry], wall_tangent: Vec2, wall_side: bool) -> Vec<EndInfoEntry> {
        let mut out: Vec<EndInfoEntry> = entries.to_vec();
        if out.is_empty() {
            return out;
        }
        if out.len() > 2 && out[0].is_side_attached() {
            let first = out[0];
            let (removed, kept): (Vec<_>, Vec<_>) = out[1..].iter().copied().partition(|e| e.is_side_attached());
            let sine = wall_tangent.cross(first.tangent).abs();
            let value = |e: &EndInfoEntry| e.mid_point_vector.dot(wall_tangent) - e.width / sine;
            let mut head = first;
            if sine >= SPAN_SINE_LIMIT {
                let mut best = value(&first);
                for e in removed.iter().filter(|e| e.parallel_to_first) {
                    let v = value(e);
                    if v < best - SPAN_VALUE_MARGIN {
                        best = v;
                        head = *e;
                    }
                }
            }
            out = std::iter::once(head).chain(kept).collect();
        }
        if out[0].is_side_attached() && out.len() > 2 {
            let head = out[0];
            out = std::iter::once(head)
                .chain(
                    out[1..]
                        .iter()
                        .copied()
                        .filter(|e| e.is_side_attached() || e.side_is_join_on == wall_side),
                )
                .collect();
        }
        out
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TargetBuilder {
        FirstOrSecond { wall_index: usize },
        Later { wall_index: usize },
    }

    /// The builder for a wall at `index` of the array.
    pub fn builder_for_index(index: usize) -> TargetBuilder {
        if index < 2 {
            TargetBuilder::FirstOrSecond { wall_index: index }
        } else {
            TargetBuilder::Later { wall_index: index }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct SpanJoin {
        /// The array after the removal.
        pub remaining: Vec<EndInfoEntry>,
        /// The wall itself was a removed side-attached entry: nothing is built.
        pub wall_removed: bool,
        /// Position of the wall in the array BEFORE the removal; `None` when it is not there.
        pub index_before_removal: Option<usize>,
        /// Position of the wall in the array AFTER the removal.
        ///
        /// UNKNOWN: whether the builder is selected with the index taken before or after the removal. The rule states the
        /// removal and that the wall is located in the array; it does not state their order, so both indices are returned.
        pub index_after_removal: Option<usize>,
    }

    /// For arrays longer than two, every side-attached entry at index 1 or later is removed, and nothing is built when the
    /// wall itself is such an entry. An array of two entries is not touched. The wall is located by element id (a wall
    /// appears once per end index in the array).
    pub fn join_walls_on_one_z_span(entries: &[EndInfoEntry], wall: ElementId) -> SpanJoin {
        let index_before_removal = entries.iter().position(|e| e.wall == wall);
        let mut remaining: Vec<EndInfoEntry> = Vec::with_capacity(entries.len());
        let mut wall_removed = false;
        for (i, e) in entries.iter().enumerate() {
            let removable = entries.len() > 2 && i >= 1 && e.is_side_attached();
            if removable {
                if e.wall == wall {
                    wall_removed = true;
                }
            } else {
                remaining.push(*e);
            }
        }
        let index_after_removal = if wall_removed { None } else { remaining.iter().position(|e| e.wall == wall) };
        SpanJoin { remaining, wall_removed, index_before_removal, index_after_removal }
    }

    /// The engine models no stacked wall, so for it this is always `true`; the argument is the predicate's answer for the
    /// wall.
    pub fn stacked_wall_gets_a_join_step(is_stacked_wall: bool) -> bool {
        !is_stacked_wall
    }

    // ------------------------------------------------------------ : the geometric neighbours

    /// One processed wall of the neighbour selection.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct NeighbourEntry {
        pub wall: ElementId,
        /// Attached at a side index (2 or 3): the entry counts as two directions.
        pub side_attached: bool,
        /// Angle from the processing wall's own tangent at the join to this wall's tangent at its control point, about the
        /// driver normal, in `[0, 2 pi]`. UNKNOWN: whether runs counter-clockwise or clockwise.
        pub angle: f64,
        pub width: f64,
        /// Curvature of the centre curve at the control point (sign already flipped when the
        /// tangent was negated).
        pub curvature: f64,
        /// Overlap of the entry's Z span with the processing wall's Z span.
        pub z_overlap: f64,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct NeighbourRecord {
        pub wall: ElementId,
        pub angle: f64,
        pub width: f64,
        pub curvature: f64,
        pub z_overlap: f64,
    }

    /// Why the selection returns no pair.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum NeighbourError {
        /// An array with fewer than two entries: Err 0, both outputs invalid.
        FewerThanTwoEntries,
        /// "Unset element id for left or right neighbor": Err 1.
        UnsetNeighbour,
    }

    /// The range of the array the operation processes for the wall at `position` (`None`: the wall is not in the array and
    /// every entry is seen). For the wall at position `k >= 1` it is `0..k` (the walls that precede it); for the wall at
    /// position 0 it is entries 0 and 1 (itself and the next one).
    pub fn processed_range(position: Option<usize>, len: usize) -> Range<usize> {
        match position {
            None => 0..len,
            Some(0) => 0..len.min(2),
            Some(k) => 0..k.min(len),
        }
    }

    /// The records built for the processed entries. A value of `angle` above [`ANGLE_WRAP_TO_ZERO`] becomes 0; a
    /// side-attached entry contributes a second record with the angle plus pi (wrapped at 2 pi) and the curvature sign
    /// flipped.
    pub fn neighbour_records(processed: &[NeighbourEntry]) -> Vec<NeighbourRecord> {
        let two_pi = std::f64::consts::TAU;
        let mut out = Vec::with_capacity(processed.len() * 2);
        for e in processed {
            let angle = if e.angle > ANGLE_WRAP_TO_ZERO { 0.0 } else { e.angle };
            out.push(NeighbourRecord {
                wall: e.wall,
                angle,
                width: e.width,
                curvature: e.curvature,
                z_overlap: e.z_overlap,
            });
            if e.side_attached {
                let mut a = angle + std::f64::consts::PI;
                if a >= two_pi {
                    a -= two_pi;
                }
                out.push(NeighbourRecord {
                    wall: e.wall,
                    angle: a,
                    width: e.width,
                    curvature: -e.curvature,
                    z_overlap: e.z_overlap,
                });
            }
        }
        out
    }

    /// The two passes of the selection: pass 0 fills output A and keeps the LARGEST angle, pass 1
    /// fills output B and keeps the SMALLEST angle.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Pass {
        Largest,
        Smallest,
    }

    /// Comparator: is `a` kept over `b` in `pass`? `vertex_eps` is the vertex tolerance (UNKNOWN: its numeric value, so it
    /// is an input).
    ///
    /// A record whose angle is within 0.1 degree of zero loses against any record with a non-zero angle. Angles closer than
    /// 0.1 degree are tied and decided in this order: the wider wall wins in both passes (tolerance `vertex_eps`); then
    /// curvature (larger kept by pass 0, smaller by pass 1; tolerance twice `vertex_eps`); then Z-span overlap (smaller
    /// kept by pass 0, larger by pass 1; UNKNOWN: a tolerance, none is applied); and finally the LOWER element id wins in
    /// both passes.
    pub fn is_kept_over(pass: Pass, a: &NeighbourRecord, b: &NeighbourRecord, vertex_eps: f64) -> bool {
        let near_zero = |x: f64| x.abs() <= ANGLE_TIE_LIMIT;
        if near_zero(a.angle) != near_zero(b.angle) {
            return !near_zero(a.angle);
        }
        if (a.angle - b.angle).abs() >= ANGLE_TIE_LIMIT {
            return match pass {
                Pass::Largest => a.angle > b.angle,
                Pass::Smallest => a.angle < b.angle,
            };
        }
        if (a.width - b.width).abs() > vertex_eps {
            return a.width > b.width;
        }
        if (a.curvature - b.curvature).abs() > 2.0 * vertex_eps {
            return match pass {
                Pass::Largest => a.curvature > b.curvature,
                Pass::Smallest => a.curvature < b.curvature,
            };
        }
        match a.z_overlap.partial_cmp(&b.z_overlap).unwrap_or(Ordering::Equal) {
            Ordering::Less => return pass == Pass::Largest,
            Ordering::Greater => return pass == Pass::Smallest,
            Ordering::Equal => {}
        }
        a.wall.0 < b.wall.0
    }

    /// The two geometric neighbours `(out_a, out_b)` of the wall at `position` of the ordered array `entries`.
    ///
    /// Both outputs start invalid and an array with fewer than two entries is Err 0. Only the walls that precede the wall
    /// are looked at ([`processed_range`]). Which of the two outputs the callers treat as the left or the right neighbour
    /// is UNKNOWN.
    pub fn geom_ordered_wall_neighbours(
        entries: &[NeighbourEntry],
        position: Option<usize>,
        vertex_eps: f64,
    ) -> Result<(ElementId, ElementId), NeighbourError> {
        if entries.len() < 2 {
            return Err(NeighbourError::FewerThanTwoEntries);
        }
        let range = processed_range(position, entries.len());
        let records = neighbour_records(&entries[range]);
        let pick = |pass: Pass| -> Option<ElementId> {
            let mut best: Option<NeighbourRecord> = None;
            for r in &records {
                best = Some(match best {
                    None => *r,
                    Some(b) => {
                        if is_kept_over(pass, r, &b, vertex_eps) {
                            *r
                        } else {
                            b
                        }
                    }
                });
            }
            best.map(|b| b.wall)
        };
        match (pick(Pass::Largest), pick(Pass::Smallest)) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => Err(NeighbourError::UnsetNeighbour),
        }
    }

    // ------------------------------------------------------------ : the join-order predicate

    /// * `rank_a`, `rank_b`: the index of the setting record of the document's wall join default
    ///   setting whose set of wall-type ids contains the TYPE id of the wall; `None` when no record
    ///   matches (-1);
    /// * `function_a`, `function_b`: the wall function enum stored on the wall type, as the raw integer (UNKNOWN: the names
    ///   of the values; only 0..3 are read).
    ///
    /// Different ranks: `true` when `rank_a` is unset, `false` when `rank_b` is unset, otherwise
    /// `true` exactly when `rank_b < rank_a`. Equal ranks (or no setting): equal functions give
    /// `false`; the result is `true` only for (A=3, B=2), (A=0, B=1) and (A=0, B=2); every other
    /// pair gives `false`. No element id is read.
    pub fn new_order_ranks_above_first(
        global_switch_set: bool,
        both_walls: bool,
        rank_a: Option<usize>,
        rank_b: Option<usize>,
        function_a: i32,
        function_b: i32,
    ) -> bool {
        if global_switch_set || !both_walls {
            return false;
        }
        if rank_a != rank_b {
            return match (rank_a, rank_b) {
                (None, _) => true,
                (_, None) => false,
                (Some(a), Some(b)) => b < a,
            };
        }
        matches!((function_a, function_b), (3, 2) | (0, 1) | (0, 2))
    }

    /// When the wall's type has a record in the setting, `(record_count / 100 + 10) * 100 - rank` (integer division; a
    /// larger value for a smaller rank); otherwise the value stored in the driver (`stored`, 0 for a null wall).
    pub fn join_order_strength(record_count: usize, rank: Option<usize>, stored: i32) -> i32 {
        match rank {
            Some(r) => ((record_count / 100 + 10) * 100) as i32 - r as i32,
            None => stored,
        }
    }

    /// True when any wall of the join has a STRICTLY smaller join-order strength than the wall (the scan runs from the last
    /// entry, which does not change the answer).
    pub fn has_join_strength_violation(wall_strength: i32, others: &[i32]) -> bool {
        others.iter().rev().any(|&s| s < wall_strength)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(c: JoinCategory) -> CutOrderElement {
        CutOrderElement::of(c)
    }

    fn order(a: CutOrderElement, b: CutOrderElement) -> CutOrder {
        resolve_cut_order(true, &a, &b)
    }

    /// The architectural column and the wall are auto-joinable in either order, a wall and a floor, ceiling or roof are
    /// not, and the COLUMN is the cut element in either argument order.
    #[test]
    fn spec_col03_a_column_and_a_wall_auto_join_and_the_column_is_cut() {
        assert!(auto_joinable(-2000100, -2000011) && auto_joinable(-2000011, -2000100));
        assert!(auto_joinable(-2000011, -2000011) && auto_joinable(-2003400, -2003400));
        assert!(!auto_joinable(-2000011, -2000032), "a wall and a floor are not in the table");
        assert!(!auto_joinable(-2000011, -2000038) && !auto_joinable(-2000011, -2000035));
        assert_eq!(AUTO_JOINABLE_CATEGORY_PAIRS.len(), 15);
        let (col, wall) = (el(JoinCategory::Column), el(JoinCategory::Wall));
        let (c, w) = (ElementId(1), ElementId(2));
        assert_eq!(order(col, wall).cutter_and_cut(c, w), Some((w, c)), "column first: the wall cuts the column");
        assert_eq!(order(wall, col).cutter_and_cut(w, c), Some((w, c)), "wall first: the same");
    }

    /// "otherwise the flag stays FALSE": with no rule matching the FIRST element cuts the SECOND (wall against wall, and
    /// any pair not listed).
    #[test]
    fn spec_f05b1_with_no_rule_the_first_element_cuts_the_second() {
        let w = el(JoinCategory::Wall);
        let o = order(w, w);
        assert_eq!(o, CutOrder { a_is_cut: false, no_cut_step: false });
        let (a, b) = (ElementId(1), ElementId(2));
        assert_eq!(o.cutter_and_cut(a, b), Some((a, b)), "first cuts second");
        // a pair the table does not name
        assert!(!order(el(JoinCategory::Stair), el(JoinCategory::Roof)).a_is_cut);
    }

    /// Rule (3): a wall is cut by a floor, ceiling, roof or toposolid, and the flag is FALSE in the other order, so the
    /// floor is the cutter whichever way the pair is given.
    #[test]
    fn spec_f05b1_a_wall_is_cut_by_a_floor_in_either_order() {
        let (wall, floor) = (ElementId(10), ElementId(20));
        for slab in [JoinCategory::Ceiling, JoinCategory::Roof, JoinCategory::Toposolid] {
            let o = order(el(JoinCategory::Wall), el(slab));
            assert!(o.a_is_cut, "{slab:?}: B cuts A");
            assert_eq!(o.cutter_and_cut(wall, floor), Some((floor, wall)));
            // reversed: the first element (the slab) cuts the second (the wall)
            let r = order(el(slab), el(JoinCategory::Wall));
            assert!(!r.a_is_cut, "{slab:?} first: A cuts B");
            assert_eq!(r.cutter_and_cut(floor, wall), Some((floor, wall)));
        }
        let o = order(el(JoinCategory::Wall), CutOrderElement::floor());
        assert_eq!(o.cutter_and_cut(wall, floor), Some((floor, wall)));
        let r = order(CutOrderElement::floor(), el(JoinCategory::Wall));
        assert_eq!(r.cutter_and_cut(floor, wall), Some((floor, wall)));
        // a stacked wall is a wall for this rule
        assert!(order(el(JoinCategory::StackedWall), el(JoinCategory::Roof)).a_is_cut);
    }

    /// Rule (1): a column is cut by a wall in either order.
    #[test]
    fn spec_f05b1_a_column_is_cut_by_a_wall_in_either_order() {
        assert!(order(el(JoinCategory::Column), el(JoinCategory::Wall)).a_is_cut, "(1): B cuts A");
        assert!(order(el(JoinCategory::Column), el(JoinCategory::StackedWall)).a_is_cut);
        // wall first: no rule matches (the wall is the first element and cuts the column)
        assert!(!order(el(JoinCategory::Wall), el(JoinCategory::Column)).a_is_cut);
    }

    /// Rule (2) and its exception: a host is cut by structural members, unless A is Floor-class or a structural wall AND B
    /// is "concrete", when evaluation continues.
    #[test]
    fn spec_f05b1_rule_2_and_its_concrete_exception() {
        let beam = el(JoinCategory::FramingStructural);
        assert!(order(el(JoinCategory::Wall), beam).a_is_cut);
        assert!(order(el(JoinCategory::Roof), el(JoinCategory::ColumnStructural)).a_is_cut);
        assert!(order(el(JoinCategory::Toposolid), el(JoinCategory::FoundationStructural)).a_is_cut);
        // exception: floor-class A and concrete B continue; no later rule matches -> FALSE
        let concrete_beam = CutOrderElement { concrete: true, ..beam };
        assert!(!order(CutOrderElement::floor(), concrete_beam).a_is_cut);
        // a structural wall behaves as the floor does
        let structural_wall = CutOrderElement { structural_wall: true, ..el(JoinCategory::Wall) };
        assert!(!order(structural_wall, concrete_beam).a_is_cut);
        // not concrete: the exception does not apply
        assert!(order(CutOrderElement::floor(), beam).a_is_cut);
        // a plain wall keeps rule (2) even against a concrete member (not floor-class, not structural)
        assert!(order(el(JoinCategory::Wall), concrete_beam).a_is_cut);
    }

    /// Rules (4) and (5), the slab-edge / fascia / gutter family and the user-join split.
    #[test]
    fn spec_f05b1_rules_4_and_5_depend_on_user_join() {
        let edge = el(JoinCategory::SlabEdge);
        let floor = CutOrderElement::floor();
        // (4): only when NOT a user join
        assert!(resolve_cut_order(false, &edge, &floor).a_is_cut);
        assert!(!resolve_cut_order(true, &edge, &floor).a_is_cut);
        // (5): a host cut by a fascia/gutter/slab edge ...
        assert!(resolve_cut_order(true, &el(JoinCategory::Wall), &el(JoinCategory::Fascia)).a_is_cut);
        assert!(resolve_cut_order(true, &el(JoinCategory::Roof), &el(JoinCategory::Gutter)).a_is_cut);
        // ... except a user join of a slab edge against a floor-class host: evaluation continues
        assert!(resolve_cut_order(false, &floor, &edge).a_is_cut);
        assert!(!resolve_cut_order(true, &floor, &edge).a_is_cut);
        let topo = CutOrderElement { toposolid_class: true, ..el(JoinCategory::Toposolid) };
        assert!(!resolve_cut_order(true, &topo, &edge).a_is_cut);
    }

    /// Rules (6) to (11).
    #[test]
    fn spec_f05b1_rules_6_to_11() {
        // (6) a wall sweep is cut by anything that is not a sweep, an edge, or a wall
        let sweep = el(JoinCategory::WallSweep);
        assert!(order(sweep, el(JoinCategory::Column)).a_is_cut);
        assert!(!order(sweep, el(JoinCategory::WallSweep)).a_is_cut);
        assert!(!order(sweep, el(JoinCategory::Wall)).a_is_cut);
        assert!(!order(sweep, el(JoinCategory::SlabEdge)).a_is_cut);
        // (7) framing is cut by structural columns and foundations
        let beam = el(JoinCategory::FramingStructural);
        assert!(order(beam, el(JoinCategory::ColumnStructural)).a_is_cut);
        assert!(order(beam, el(JoinCategory::FoundationStructural)).a_is_cut);
        assert!(!order(el(JoinCategory::ColumnStructural), beam).a_is_cut);
        // (8) a concrete structural member is cut by a floor-class element or a structural wall
        let conc = CutOrderElement { concrete: true, ..el(JoinCategory::ColumnStructural) };
        assert!(order(conc, CutOrderElement::floor()).a_is_cut);
        assert!(!order(el(JoinCategory::ColumnStructural), CutOrderElement::floor()).a_is_cut);
        // (9) structural connections are cut by anything that is not one
        let conn = el(JoinCategory::ConnectionStructural);
        assert!(order(conn, el(JoinCategory::Wall)).a_is_cut);
        assert!(!order(conn, conn).a_is_cut);
        // (10) a floor-class element is cut by stairs whose predicate holds
        let stair = CutOrderElement { stairs_flag: true, ..el(JoinCategory::Stair) };
        assert!(order(CutOrderElement::floor(), stair).a_is_cut);
        assert!(!order(CutOrderElement::floor(), el(JoinCategory::Stair)).a_is_cut);
        // (11) foundations by rank: floor-class 3, coping 2, wall foundation 1, otherwise 0
        let f0 = el(JoinCategory::FoundationStructural);
        let f1 = CutOrderElement { wall_foundation: true, ..f0 };
        let f2 = CutOrderElement { coping_rank: true, ..f0 };
        let f3 = CutOrderElement { floor_class: true, ..f0 };
        assert!(order(f0, f1).a_is_cut && order(f1, f2).a_is_cut && order(f2, f3).a_is_cut);
        assert!(!order(f1, f0).a_is_cut && !order(f3, f2).a_is_cut);
        assert!(!order(f1, f1).a_is_cut, "equal rank: the first cuts the second");
    }

    /// "no cut step" and: nothing is stored, so there is no cutter.
    #[test]
    fn spec_f05b1_a_thin_ceiling_or_a_hosted_sweep_adds_no_cut_step() {
        let thin = CutOrderElement { thin_ceiling: true, ..el(JoinCategory::Ceiling) };
        let o = order(el(JoinCategory::Wall), thin);
        assert!(o.no_cut_step);
        assert_eq!(o.cutter_and_cut(ElementId(1), ElementId(2)), None);
        let hosted = CutOrderElement { sweep_hosted_by_other: true, ..el(JoinCategory::WallSweep) };
        assert!(order(hosted, el(JoinCategory::Wall)).no_cut_step);
        assert!(order(el(JoinCategory::Wall), hosted).no_cut_step);
    }

    /// The signed category ids read at the table.
    #[test]
    fn spec_f05b1_category_ids_map_to_the_table_categories() {
        for (id, c) in [
            (-2000011, JoinCategory::Wall),
            (-2000032, JoinCategory::Floor),
            (-2000035, JoinCategory::Roof),
            (-2000038, JoinCategory::Ceiling),
            (-2000100, JoinCategory::Column),
            (-2000120, JoinCategory::Stair),
            (-2000181, JoinCategory::WallSweep),
            (-2001300, JoinCategory::FoundationStructural),
            (-2001320, JoinCategory::FramingStructural),
            (-2001330, JoinCategory::ColumnStructural),
            (-2001390, JoinCategory::Fascia),
            (-2001391, JoinCategory::Gutter),
            (-2001392, JoinCategory::SlabEdge),
            (-2003500, JoinCategory::StackedWall),
            (-2009030, JoinCategory::ConnectionStructural),
            (-2001079, JoinCategory::Toposolid),
        ] {
            assert_eq!(JoinCategory::from_category_id(id), c, "{id}");
        }
        assert_eq!(JoinCategory::from_category_id(-1), JoinCategory::Other);
    }
}

#[cfg(test)]
mod wall_join_list_tests {
    use super::wall_join_list::*;
    use asterion_core::ElementId;
    use asterion_geom::Vec2;

    fn id(n: u64) -> ElementId {
        ElementId(n)
    }

    fn entry(wall: u64, end_index: u8, side_is_join_on: bool, tangent: Vec2, mid: Vec2, width: f64) -> EndInfoEntry {
        EndInfoEntry {
            wall: id(wall),
            end_index,
            side_is_join_on,
            tangent,
            mid_point_vector: mid,
            width,
            parallel_to_first: false,
        }
    }

    fn ne(wall: u64, angle: f64) -> NeighbourEntry {
        NeighbourEntry { wall: id(wall), side_attached: false, angle, width: 0.2, curvature: 0.0, z_overlap: 3.0 }
    }

    const EPS: f64 = 1e-4;

    /// (regen): "when a span has more than two entries and entry 0 is attached at a side index (2 or 3), every
    /// side-attached entry from index 1 on is removed from the span array; the removed entries whose centre curves are
    /// parallel to that of entry 0 are compared by the value (vector ... projected on the wall tangent) minus (entry width
    /// divided by |sine| ...), and the one that is smaller than the running best by more than 1e-9 is written over entry
    /// 0".
    #[test]
    fn spec_w05c1_a_smaller_parallel_side_entry_replaces_entry_0() {
        let wall_t = Vec2::new(1.0, 0.0);
        let host_t = Vec2::new(0.0, 1.0); // sine 1
        let e0 = entry(10, 2, false, host_t, Vec2::new(0.5, 0.0), 0.2); // value 0.5 - 0.2 = 0.3
        let end = entry(11, 0, false, wall_t, Vec2::new(0.0, 0.0), 0.2);
        let mut p1 = entry(12, 3, false, host_t, Vec2::new(0.1, 0.0), 0.2); // value -0.1
        p1.parallel_to_first = true;
        let mut p2 = entry(13, 2, false, host_t, Vec2::new(0.4, 0.0), 0.2); // value 0.2
        p2.parallel_to_first = true;
        let mut unparallel = entry(14, 2, false, host_t, Vec2::new(-5.0, 0.0), 0.2); // would win, not parallel
        unparallel.parallel_to_first = false;
        let out = reduce_span_entries(&[e0, end, p1, p2, unparallel], wall_t, false);
        // every side-attached entry from index 1 on is gone; entry 0 is now the smallest parallel one
        assert_eq!(out.iter().map(|e| e.wall.0).collect::<Vec<_>>(), vec![12, 11]);
    }

    /// "no replacement is made when that |sine| is below 0.0017453292519943279", the side-attached entries are still
    /// removed; and a candidate must beat the best by more than 1e-9.
    #[test]
    fn spec_w05c1_no_replacement_below_the_sine_limit_or_within_the_margin() {
        let wall_t = Vec2::new(1.0, 0.0);
        let e0 = entry(10, 2, false, Vec2::new(1.0, 0.001), Vec2::new(0.5, 0.0), 0.2);
        let end = entry(11, 0, false, wall_t, Vec2::new(0.0, 0.0), 0.2);
        let mut p = entry(12, 3, false, Vec2::new(1.0, 0.001), Vec2::new(-9.0, 0.0), 0.2);
        p.parallel_to_first = true;
        let out = reduce_span_entries(&[e0, end, p], wall_t, false);
        assert_eq!(out.iter().map(|e| e.wall.0).collect::<Vec<_>>(), vec![10, 11]);
        // |sine| = 1 but the candidate ties entry 0 within the margin: no replacement
        let host_t = Vec2::new(0.0, 1.0);
        let e0 = entry(10, 2, false, host_t, Vec2::new(0.5, 0.0), 0.2);
        let mut p = entry(12, 3, false, host_t, Vec2::new(0.5 - 5e-10, 0.0), 0.2);
        p.parallel_to_first = true;
        let out = reduce_span_entries(&[e0, end, p], wall_t, false);
        assert_eq!(out[0].wall.0, 10);
    }

    /// An array whose entry 0 is end-attached, or with two entries, is not reordered.
    #[test]
    fn spec_w05c1_end_attached_entries_on_the_other_side_are_removed() {
        let t = Vec2::new(1.0, 0.0);
        let host = entry(10, 2, false, Vec2::new(0.0, 1.0), Vec2::new(0.0, 0.0), 0.2);
        let a = entry(11, 0, true, t, Vec2::new(0.0, 0.0), 0.2);
        let b = entry(12, 1, false, t, Vec2::new(0.0, 0.0), 0.2);
        let c = entry(13, 0, false, t, Vec2::new(0.0, 0.0), 0.2);
        let out = reduce_span_entries(&[host, a, b, c], t, false);
        assert_eq!(out.iter().map(|e| e.wall.0).collect::<Vec<_>>(), vec![10, 12, 13]);
        // entry 0 end-attached: untouched, side entries included
        let first = entry(20, 0, false, t, Vec2::new(0.0, 0.0), 0.2);
        let side = entry(21, 2, false, t, Vec2::new(0.0, 0.0), 0.2);
        let input = [first, side, a];
        assert_eq!(reduce_span_entries(&input, t, false), input.to_vec());
        // two entries: untouched
        let two = [host, a];
        assert_eq!(reduce_span_entries(&two, t, false), two.to_vec());
        assert!(reduce_span_entries(&[], t, false).is_empty());
    }

    /// "for arrays longer than two, removes every side-attached entry at index 1 or later and builds nothing when the wall
    /// itself is such an entry".
    #[test]
    fn spec_w05c1_side_entries_after_the_first_are_removed_and_the_builder_follows_the_index() {
        let t = Vec2::new(1.0, 0.0);
        let a = entry(1, 0, false, t, Vec2::new(0.0, 0.0), 0.2);
        let b = entry(2, 1, false, t, Vec2::new(0.0, 0.0), 0.2);
        let h = entry(3, 2, false, t, Vec2::new(0.0, 0.0), 0.2);
        let c = entry(4, 0, false, t, Vec2::new(0.0, 0.0), 0.2);
        let j = join_walls_on_one_z_span(&[a, h, b, c], id(4));
        assert_eq!(j.remaining.iter().map(|e| e.wall.0).collect::<Vec<_>>(), vec![1, 2, 4]);
        assert!(!j.wall_removed);
        assert_eq!((j.index_before_removal, j.index_after_removal), (Some(3), Some(2)));
        assert_eq!(builder_for_index(0), TargetBuilder::FirstOrSecond { wall_index: 0 });
        assert_eq!(builder_for_index(1), TargetBuilder::FirstOrSecond { wall_index: 1 });
        assert_eq!(builder_for_index(2), TargetBuilder::Later { wall_index: 2 });
        // the wall itself is a removed side-attached entry: nothing is built
        let j = join_walls_on_one_z_span(&[a, h, b], id(3));
        assert!(j.wall_removed && j.index_after_removal.is_none());
        // a two-entry array is not touched, even with a side entry at index 1
        let j = join_walls_on_one_z_span(&[a, h], id(3));
        assert!(!j.wall_removed && j.remaining.len() == 2);
        // the wall is not in the array: nothing to build
        let j = join_walls_on_one_z_span(&[a, b], id(9));
        assert_eq!((j.index_before_removal, j.index_after_removal), (None, None));
    }

    #[test]
    fn spec_w05c3_a_stacked_wall_never_gets_a_join_step() {
        assert!(!stacked_wall_gets_a_join_step(true));
        assert!(stacked_wall_gets_a_join_step(false));
    }

    /// "for the wall at array position k >= 1 it processes entries 0..k-1; for the wall at position 0 it processes entries
    /// 0 and 1 (itself and the next one) and ignores the rest; a wall that is not in the array sees every entry"; "an array
    /// with fewer than two entries returns Err 0".
    #[test]
    fn spec_w05c2_only_the_walls_that_precede_the_wall_are_processed() {
        assert_eq!(processed_range(Some(3), 5), 0..3);
        assert_eq!(processed_range(Some(1), 5), 0..1);
        assert_eq!(processed_range(Some(0), 5), 0..2);
        assert_eq!(processed_range(None, 5), 0..5);
        assert_eq!(processed_range(Some(0), 1), 0..1);
        assert_eq!(geom_ordered_wall_neighbours(&[ne(1, 1.0)], Some(0), EPS), Err(NeighbourError::FewerThanTwoEntries));
        assert_eq!(geom_ordered_wall_neighbours(&[], None, EPS), Err(NeighbourError::FewerThanTwoEntries));
        // entry 3 (angle 5.0) comes AFTER position 2 and is ignored
        let entries = [ne(1, 1.0), ne(2, 4.0), ne(3, 2.0), ne(4, 5.0)];
        let (a, b) = geom_ordered_wall_neighbours(&entries, Some(2), EPS).unwrap();
        assert_eq!((a, b), (id(2), id(1)));
        // a wall that is not in the array sees every entry
        let (a, b) = geom_ordered_wall_neighbours(&entries, None, EPS).unwrap();
        assert_eq!((a, b), (id(4), id(1)));
    }

    /// A record whose angle is within 0.1 degree of zero loses against any record with a non-zero angle".
    #[test]
    fn spec_w05c2_pass_0_keeps_the_largest_angle_pass_1_the_smallest() {
        let entries = [ne(1, 1.0), ne(2, 4.0), ne(3, 2.5)];
        assert_eq!(geom_ordered_wall_neighbours(&entries, None, EPS).unwrap(), (id(2), id(1)));
        // a record within 0.1 degree of zero loses in BOTH passes against a non-zero one
        let entries = [ne(1, 0.0005), ne(2, 3.0)];
        assert_eq!(geom_ordered_wall_neighbours(&entries, None, EPS).unwrap(), (id(2), id(2)));
        // "values above 6.28144 become 0"
        let entries = [ne(1, 6.2831), ne(2, 3.0)];
        assert_eq!(geom_ordered_wall_neighbours(&entries, None, EPS).unwrap(), (id(2), id(2)));
        // 6.28 is below the limit and is the largest angle
        let entries = [ne(1, 6.28), ne(2, 3.0)];
        assert_eq!(geom_ordered_wall_neighbours(&entries, None, EPS).unwrap(), (id(1), id(2)));
    }

    /// Tie-breaks: "angles closer than 0.1 degree are tied and decided in this order: the wider wall wins in both passes,
    /// then curvature (larger kept by pass 0, smaller by pass 1), then Z-span overlap (smaller kept by pass 0, larger by
    /// pass 1), and finally the LOWER element id wins in both passes."
    #[test]
    fn spec_w05c2_tie_breaks_in_the_order_the_row_gives() {
        let rec = |wall: u64, width: f64, curvature: f64, z: f64| NeighbourEntry {
            wall: id(wall),
            side_attached: false,
            angle: 2.0, // all tied
            width,
            curvature,
            z_overlap: z,
        };
        let pick = |v: &[NeighbourEntry]| geom_ordered_wall_neighbours(v, None, EPS).unwrap();
        // 1. the wider wall wins in both passes (width beats curvature, overlap and id)
        assert_eq!(pick(&[rec(1, 0.2, 9.0, 0.0), rec(2, 0.3, 0.0, 9.0)]), (id(2), id(2)));
        // 2. larger curvature kept by pass 0, smaller by pass 1
        assert_eq!(pick(&[rec(1, 0.2, 0.5, 3.0), rec(2, 0.2, 0.1, 3.0)]), (id(1), id(2)));
        // widths within the vertex tolerance are equal: curvature decides
        assert_eq!(pick(&[rec(1, 0.2 + 5e-5, 0.5, 3.0), rec(2, 0.2, 0.1, 3.0)]), (id(1), id(2)));
        // curvatures within twice the tolerance are equal: Z overlap decides
        // 3. smaller overlap kept by pass 0, larger by pass 1
        assert_eq!(pick(&[rec(1, 0.2, 0.0, 1.0), rec(2, 0.2, 0.0, 2.0)]), (id(1), id(2)));
        assert_eq!(pick(&[rec(1, 0.2, 0.0, 1.0), rec(2, 0.2, 1.5e-4, 2.0)]), (id(1), id(2)));
        // 4. everything equal: the LOWER element id in both passes, whatever the array order
        assert_eq!(pick(&[rec(7, 0.2, 0.0, 3.0), rec(5, 0.2, 0.0, 3.0)]), (id(5), id(5)));
        assert_eq!(pick(&[rec(5, 0.2, 0.0, 3.0), rec(7, 0.2, 0.0, 3.0)]), (id(5), id(5)));
        // two angles 0.05 degree apart are tied; 0.2 degree apart are not
        let near = |wall: u64, angle: f64| NeighbourEntry { angle, ..ne(wall, 2.0) };
        let tied = geom_ordered_wall_neighbours(&[near(9, 2.0), near(4, 2.0 + 0.05_f64.to_radians())], None, EPS);
        assert_eq!(tied.unwrap(), (id(4), id(4)), "tied: the lower id wins both passes");
        let apart = geom_ordered_wall_neighbours(&[near(9, 2.0), near(4, 2.0 + 0.2_f64.to_radians())], None, EPS);
        assert_eq!(apart.unwrap(), (id(4), id(9)), "not tied: largest and smallest angle");
    }

    /// "a wall attached at a side index (2 or 3) contributes a second record with the angle plus pi (wrapped at 2 pi) and
    /// the curvature sign flipped, so a side attachment counts as two directions."
    #[test]
    fn spec_w05c2_a_side_attachment_counts_as_two_directions() {
        let side = NeighbourEntry { side_attached: true, curvature: 0.4, ..ne(8, 1.0) };
        let recs = neighbour_records(&[side]);
        assert_eq!(recs.len(), 2);
        assert!((recs[1].angle - (1.0 + std::f64::consts::PI)).abs() < 1e-12);
        assert_eq!((recs[0].curvature, recs[1].curvature), (0.4, -0.4));
        // wrapped at 2 pi
        let side = NeighbourEntry { side_attached: true, ..ne(8, 5.0) };
        let recs = neighbour_records(&[side]);
        assert!((recs[1].angle - (5.0 + std::f64::consts::PI - std::f64::consts::TAU)).abs() < 1e-12);
        // a single side-attached neighbour next to another wall: both directions compete
        let entries = [ne(1, 2.0), NeighbourEntry { side_attached: true, ..ne(2, 0.5) }];
        // records: 1@2.0, 2@0.5, 2@(0.5+pi=3.64): largest is wall 2 (3.64), smallest wall 2 (0.5)
        assert_eq!(geom_ordered_wall_neighbours(&entries, None, EPS).unwrap(), (id(2), id(2)));
    }

    /// The predicate reads no element id; different ranks decide, then the wall function pairs (A=3,B=2), (A=0,B=1),
    /// (A=0,B=2).
    #[test]
    fn spec_w05c4_rank_then_function_and_no_element_id() {
        let p = |ra, rb, fa, fb| new_order_ranks_above_first(false, true, ra, rb, fa, fb);
        // the global switch and the "both are walls" test
        assert!(!new_order_ranks_above_first(true, true, Some(5), Some(1), 3, 2));
        assert!(!new_order_ranks_above_first(false, false, Some(5), Some(1), 3, 2));
        // ranks differ: true when rank(B) < rank(A); rank -1 handled first
        assert!(p(Some(5), Some(1), 1, 1));
        assert!(!p(Some(1), Some(5), 1, 1));
        assert!(p(None, Some(1), 1, 1), "A has no record: true");
        assert!(!p(Some(1), None, 1, 1), "B has no record: false");
        // the rank decides before the function
        assert!(!p(Some(1), Some(5), 3, 2));
        // equal ranks (also both -1): the function pairs
        for (ra, rb) in [(Some(2), Some(2)), (None, None)] {
            assert!(p(ra, rb, 3, 2) && p(ra, rb, 0, 1) && p(ra, rb, 0, 2));
            assert!(!p(ra, rb, 2, 3) && !p(ra, rb, 1, 0) && !p(ra, rb, 2, 0));
            assert!(!p(ra, rb, 1, 1) && !p(ra, rb, 3, 3), "equal functions give false");
            assert!(!p(ra, rb, 3, 1) && !p(ra, rb, 1, 2) && !p(ra, rb, 3, 0));
        }
    }

    /// With integer division, a larger value for a smaller rank; the stored value without a record; a violation is a
    /// STRICTLY smaller strength among the walls of the join.
    #[test]
    fn spec_w05c4_join_order_strength_and_violation() {
        assert_eq!(join_order_strength(4, Some(0), 7), 1000);
        assert_eq!(join_order_strength(4, Some(3), 7), 997);
        assert_eq!(join_order_strength(250, Some(3), 7), 1200 - 3, "250 / 100 = 2");
        assert_eq!(join_order_strength(4, None, 7), 7);
        assert!(join_order_strength(4, Some(0), 0) > join_order_strength(4, Some(1), 0));
        assert!(has_join_strength_violation(1000, &[1000, 999]));
        assert!(!has_join_strength_violation(1000, &[1000, 1001]), "equal is not a violation");
        assert!(!has_join_strength_violation(1000, &[]));
    }
}
