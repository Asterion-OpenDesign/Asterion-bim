//! A number the engine cannot calculate with (NaN, infinity, beyond +-1e9 m) is refused AT THE CALL with
//! `input.invalid_number`, and nothing is stored. Before this guard a NaN coordinate was accepted (`NaN < floor` is false),
//! the model held a value that the document format writes as null, and the saved file could no longer be opened.

use asterion_bim::spec::codes;
use asterion_bim::{new_document, ops, CompoundStructure, Layer, LayerFunction};
use asterion_core::Failure;
use asterion_geom::Vec2;

fn v(x: f64, y: f64) -> Vec2 {
    Vec2::new(x, y)
}

fn refused(f: Result<impl std::fmt::Debug, Failure>) -> String {
    match f {
        Err(e) => e.code,
        Ok(x) => panic!("accepted: {x:?}"),
    }
}

#[test]
fn non_finite_and_absurd_numbers_are_refused_and_change_nothing() {
    let mut doc = new_document();
    let out = doc.transact("setup", false, |m, cx| {
        let l = ops::create_level(m, cx, "L0", 0.0)?;
        let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
        let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(5.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
        Ok((l, wt, w))
    });
    let (l, wt, w) = out.value.expect("setup");
    let before = format!("{:?}", doc.model());
    let depth = doc.undo_names().len();

    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300, -1e300, 2e9];
    for x in bad {
        let wall = |start: Vec2, end: Vec2, base_offset: f64| ops::NewWall { wall_type: wt, level: l, start, end, height: 3.0, base_offset, flip: false, structural: false };
        let mut codes_seen = Vec::new();
        for (name, r) in [
            ("wall start", doc.transact("t", false, |m, cx| ops::create_wall(m, cx, wall(v(x, 0.0), v(5.0, 0.0), 0.0))).failures.first().map(|f| f.code.clone())),
            ("wall end", doc.transact("t", false, |m, cx| ops::create_wall(m, cx, wall(v(0.0, 0.0), v(5.0, x), 0.0))).failures.first().map(|f| f.code.clone())),
            ("wall base offset", doc.transact("t", false, |m, cx| ops::create_wall(m, cx, wall(v(0.0, 1.0), v(5.0, 1.0), x))).failures.first().map(|f| f.code.clone())),
        ] {
            codes_seen.push((name, r));
        }
        for (name, c) in &codes_seen {
            assert!(c.is_some(), "{name} with {x} was accepted");
        }
        // the operations themselves (the transaction wrapper hides the Err in `failures`)
        let mut scratch = new_document();
        let _ = scratch.transact("s", false, |m, cx| {
            let l2 = ops::create_level(m, cx, "L", 0.0)?;
            assert_eq!(refused(ops::create_level(m, cx, "bad", x)), codes::INVALID_NUMBER, "level elevation {x}");
            assert_eq!(refused(ops::set_level_elevation(m, cx, l2, x)), codes::INVALID_NUMBER, "set level elevation {x}");
            assert_eq!(refused(ops::create_grid(m, cx, None, v(x, 0.0), v(5.0, 5.0))), codes::INVALID_NUMBER, "grid {x}");
            assert_eq!(refused(ops::create_room(m, cx, l2, v(x, x))), codes::INVALID_NUMBER, "room point {x}");
            let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
            let sketch = vec![v(0.0, 0.0), v(4.0, 0.0), v(4.0, x), v(0.0, 4.0)];
            assert_eq!(refused(ops::create_floor(m, cx, ft, l2, sketch, 0.0, false)), codes::INVALID_NUMBER, "floor corner {x}");
            Ok(())
        });
    }
    // three points that coincide are not a floor
    let mut scratch = new_document();
    let _ = scratch.transact("s", false, |m, cx| {
        let l2 = ops::create_level(m, cx, "L", 0.0)?;
        let ft = ops::create_floor_type(m, cx, "F", 0.2)?;
        let same = vec![v(1.0, 1.0), v(1.0, 1.0), v(1.0, 1.0)];
        assert_eq!(refused(ops::create_floor(m, cx, ft, l2, same, 0.0, false)), codes::INVALID_BOUNDARY);
        Ok(())
    });
    // moving the wall by NaN / by a huge vector is refused too, and the wall is where it was
    let moved = doc.transact("move", false, |m, cx| ops::set_wall_curve(m, cx, w, v(f64::NAN, 0.0), v(5.0, 0.0)));
    assert!(!moved.committed() && moved.failures.iter().any(|f| f.code == codes::INVALID_NUMBER), "{:?}", moved.failures);
    assert_eq!(format!("{:?}", doc.model()), before, "a refused call changes nothing");
    assert_eq!(doc.undo_names().len(), depth, "and leaves no undo step");
    // ordinary numbers still work
    let ok = doc.transact("ok", false, |m, cx| ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 2.0), end: v(5.0, 2.0), height: 3.0, base_offset: -0.5, flip: false, structural: false }));
    assert!(ok.committed());
}

/// A value so small that adding it to an elevation changes nothing must not reach the solid with a prism of no height (a panic that the
/// transaction now catches and reports as `engine.internal_error`): the operations refuse it or handle it like 0.
#[test]
fn tiny_heights_and_offsets_never_panic_an_operation() {
    let mut doc = new_document();
    let out = doc.transact("setup", false, |m, cx| {
        let l = ops::create_level(m, cx, "L0", 20.0)?;
        let l2 = ops::create_level(m, cx, "L1", 23.0)?;
        let wt = ops::create_wall_type(m, cx, "W", CompoundStructure::new(vec![Layer { width: 0.2, function: LayerFunction::Structure }], 0, 0)?)?;
        let w = ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 0.0), end: v(5.0, 0.0), height: 3.0, base_offset: 0.0, flip: false, structural: false })?;
        Ok((l, l2, w))
    });
    let (l, l2, w) = out.value.expect("setup");
    for x in [1e-300, -1e-300, 2.2250738585072014e-308, 5e-324, 1e-20, 1e-12] {
        let cases: Vec<(&str, asterion_core::txn::Outcome<()>)> = vec![
            ("height", doc.transact("t", false, |m, cx| ops::set_unconnected_height(m, cx, w, x))),
            ("base offset", doc.transact("t", false, |m, cx| ops::set_wall_base_offset(m, cx, w, x))),
            ("top constraint offset", doc.transact("t", false, |m, cx| ops::set_top_constraint(m, cx, w, Some(l2), Some(x)))),
            ("level elevation", doc.transact("t", false, |m, cx| ops::set_level_elevation(m, cx, l, x))),
        ];
        for (name, o) in cases {
            assert!(
                !o.failures.iter().any(|f| f.code == asterion_core::txn::INTERNAL_ERROR),
                "{name} = {x:e} panicked inside the operation: {:?}",
                o.failures
            );
        }
        let created = doc.transact("t", false, |m, cx| {
            let wt = m.wall_types.keys().next().copied().unwrap();
            ops::create_wall(m, cx, ops::NewWall { wall_type: wt, level: l, start: v(0.0, 3.0), end: v(5.0, 3.0), height: x.abs(), base_offset: 0.0, flip: false, structural: false })
        });
        assert!(
            !created.failures.iter().any(|f| f.code == asterion_core::txn::INTERNAL_ERROR),
            "a wall of height {x:e} panicked inside create_wall: {:?}",
            created.failures
        );
    }
}
