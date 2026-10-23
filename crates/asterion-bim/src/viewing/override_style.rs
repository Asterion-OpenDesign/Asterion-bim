//! How the graphic overrides of a view reach the drawing: the weight class a line segment carries when its element's category (or a
//! filter, or the element) overrides the projection/cut line weight, colour, pattern or the halftone.
//!
//! # Status of every rule
//!
//! * The engine draws the line in the override colour (a pen of the override weight, the pattern named), and a halftone
//!   line half way to white. The pen number to pixels table (`PEN_STEP_PX`), the dash lengths of the named line patterns
//!   and the 4-bit-per-channel colour are ASSUMED.
//!
//! # The class
//!
//! The renderer's line buffer has five floats per segment and the last is a weight class (`plan_graphics::Pen::class`). A
//! segment with an override carries a class at or above [`STYLED`] (2^23, exactly representable): the low 23 bits are the
//! code of [`decode`] (pen number, pattern id, colour 12 bits, halftone, the class it would have had). The renderer decodes
//! it, so no second buffer, no new FFI call and no change of stride.

use crate::docviews::{self, Ogs};
use crate::model::BimModel;
use asterion_core::ElementId;

/// A class at or above this carries an override.
pub const STYLED: f32 = 8_388_608.0;

/// Pixels a pen number adds over pen 1 (1 px). ASSUMED: the engine's own pens are 1 px (projection) and 2 px (cut), which
/// pens 1 and 5 reproduce.
pub const PEN_STEP_PX: f32 = 0.25;

/// The pattern id of a named line pattern (0 solid). The renderer draws seven dash patterns in paper millimetres. ASSUMED
/// definitions; an unknown name is a plain dash.
pub fn pattern_id(name: &str) -> u32 {
    let n = name.to_ascii_lowercase();
    if n.is_empty() || n == "solid" {
        0
    } else if n.contains("dash dot dot") {
        4
    } else if n.contains("dash dot") {
        3
    } else if n.contains("long dash") {
        7
    } else if n.contains("center") || n.contains("centre") || n.contains("aligning") || n.contains("grid") {
        5
    } else if n.contains("hidden") || n.contains("demolished") || n.contains("loose") || n.contains("overhead") || n.contains("double") || n.contains("triple") || n.contains("rebar") {
        6
    } else if n.contains("dot") {
        2
    } else {
        1
    }
}

/// What a styled class carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Pen number 1..=16, 0 when the weight is not overridden (the class it would have had decides the width).
    pub pen: u32,
    pub pattern: u32,
    /// 4 bits per channel, `0` when the colour is not overridden.
    pub rgb4: u32,
    pub halftone: bool,
    /// 0 beyond, 1 projection, 2 cut: the class the segment would have had.
    pub base: u32,
}

/// The class a segment of `base` class takes under `ogs`, or `base` itself when the override changes nothing of a line. `cut` says
/// which side of the override (projection or cut) applies. A class that is not 0.5 / 1.0 / 2.0 (a grid's pattern, the shell's
/// overlay classes) is left alone.
pub fn encode(base: f32, ogs: &Ogs, cut: bool) -> f32 {
    let basec = if (base - 2.0).abs() < 1e-3 {
        2u32
    } else if (base - 1.0).abs() < 1e-3 {
        1
    } else if (base - 0.5).abs() < 1e-3 {
        0
    } else {
        return base;
    };
    let (weight, color, pattern) = if cut {
        (ogs.cut_line_weight, ogs.cut_line_color, ogs.cut_line_pattern.as_deref())
    } else {
        (ogs.projection_line_weight, ogs.projection_line_color, ogs.projection_line_pattern.as_deref())
    };
    let pen = if weight >= 1 { weight.min(16) as u32 } else { 0 };
    let pat = pattern.map_or(0, pattern_id);
    let rgb4 = color.map_or(0, |[r, g, b]| {
        let q = |c: u8| ((c as u32 * 15 + 127) / 255) & 15;
        let v = (q(r) << 8) | (q(g) << 4) | q(b);
        // black cannot be told from `no colour`: it becomes the darkest colour the 4 bits hold
        if v == 0 { 1 } else { v }
    });
    if pen == 0 && pat == 0 && rgb4 == 0 && !ogs.halftone {
        return base;
    }
    let code = pen | (pat << 5) | (rgb4 << 8) | ((ogs.halftone as u32) << 20) | (basec << 21);
    STYLED + code as f32
}

/// The inverse of [`encode`]; `None` for a class below [`STYLED`].
pub fn decode(class: f32) -> Option<Decoded> {
    if class < STYLED {
        return None;
    }
    let code = (class - STYLED) as u32;
    Some(Decoded { pen: code & 31, pattern: (code >> 5) & 7, rgb4: (code >> 8) & 0xFFF, halftone: (code >> 20) & 1 == 1, base: (code >> 21) & 3 })
}

/// The overrides of one view, ready to style the segments of its drawing; `None` when the view overrides nothing a line shows.
pub struct ViewStyler<'a> {
    m: &'a BimModel,
    view: ElementId,
}

impl<'a> ViewStyler<'a> {
    pub fn new(m: &'a BimModel, view: ElementId) -> Option<ViewStyler<'a>> {
        let g = docviews::effective_graphics(m, view);
        let any = !g.category_overrides.is_empty() || !g.element_overrides.is_empty() || g.filters.iter().any(|f| f.enabled && f.ogs != Ogs::default());
        any.then_some(ViewStyler { m, view })
    }

    /// The class of a segment of `element` whose class would be `base` (cut when `base` is the cut class).
    pub fn class_for(&self, element: ElementId, base: f32) -> f32 {
        let o = docviews::effective_overrides(self.m, self.view, element);
        encode(base, &o, (base - 2.0).abs() < 1e-3)
    }

    /// The colour a body of `element` is drawn with in a shaded 3D view: the override's surface colour when it has one,
    /// then the halftone's half way to white. `base` is the colour the class gives it. (The surface transparency and the
    /// fill pattern are not drawn: the renderer's mesh has no alpha and no hatch.)
    pub fn mesh_colour(&self, element: ElementId, base: [f32; 3]) -> [f32; 3] {
        let o = docviews::effective_overrides(self.m, self.view, element);
        let mut c = match o.surface_foreground_color.or(o.cut_foreground_color) {
            Some([r, g, b]) => [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
            None => base,
        };
        if o.halftone {
            c = [(c[0] + 1.0) * 0.5, (c[1] + 1.0) * 0.5, (c[2] + 1.0) * 0.5];
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_round_trips_and_an_empty_override_changes_nothing() {
        assert_eq!(encode(2.0, &Ogs::default(), true), 2.0);
        let o = Ogs { cut_line_weight: 9, cut_line_color: Some([255, 0, 0]), cut_line_pattern: Some("Dash".into()), halftone: true, ..Ogs::default() };
        let c = encode(2.0, &o, true);
        assert!(c >= STYLED);
        let d = decode(c).unwrap();
        assert_eq!((d.pen, d.pattern, d.rgb4, d.halftone, d.base), (9, 1, 0xF00, true, 2));
        // the projection side of the same override is untouched except for the halftone, which is one flag for both
        let p = decode(encode(1.0, &o, false)).unwrap();
        assert_eq!((p.pen, p.pattern, p.rgb4, p.halftone, p.base), (0, 0, 0, true, 1));
        // a grid's class and the overlay classes are left alone
        assert_eq!(encode(12.0, &o, false), 12.0);
        assert_eq!(encode(3.0, &o, false), 3.0);
        // black is not `no colour`
        let k = decode(encode(1.0, &Ogs { projection_line_color: Some([0, 0, 0]), ..Ogs::default() }, false)).unwrap();
        assert_ne!(k.rgb4, 0);
        // the largest class is exactly representable
        let big = encode(2.0, &Ogs { cut_line_weight: 16, cut_line_color: Some([255, 255, 255]), cut_line_pattern: Some("Long dash".into()), halftone: true, ..Ogs::default() }, true);
        let d = decode(big).unwrap();
        assert_eq!((d.pen, d.pattern, d.rgb4, d.halftone, d.base), (16, 7, 0xFFF, true, 2));
    }

    #[test]
    fn line_patterns_map_to_the_seven_renderer_patterns() {
        for (name, id) in [("solid", 0), ("Dash", 1), ("Dot", 2), ("Dot 2mm", 2), ("Dash dot", 3), ("Dash dot dot", 4), ("Center", 5), ("Hidden", 6), ("Long dash", 7), ("Triple dash", 6), ("Unknown", 1)] {
            assert_eq!(pattern_id(name), id, "{name}");
        }
    }
}
