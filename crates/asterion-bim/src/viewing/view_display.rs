//! * The Discipline is the view's integer parameter -1005163, 1 when absent, its values named 1 Architectural, 2
//!   Structural, 4 Mechanical, 8 Electrical Plumbing Coordination.
//!
//! * The engine keeps these two as those values' names (`normal`, `as_underlay`, `no_display`; `show_parts_only`,
//!   `show_original_only`, `show_parts_and_original`; [`display_model_code`], [`parts_visibility_code`] give the stored
//!   integers). Which UI label shows which value (Normal / Halftone / Do not display, Show Parts / Show Original / Show
//!   Both) is the shell's OBSERVED mapping, not specified.
//!
//! The engine keeps those as the reference's option names (Clean all wall joins / Clean same type wall joins, Look up /
//! Look down, Project North / True North), never as a guessed integer. Show Hidden Lines, Color Scheme Location, Sun Path,
//! Annotation Crop and Depth Clipping have no stored field read at all: ASSUMED representation.
//!
//! ASSUMED for every new view.

use asterion_core::id::ElementId;
use serde::{Deserialize, Serialize};

/// One option list: the setting's key and its tokens, in the reference's order.
pub struct Vocabulary {
    pub key: &'static str,
    pub tokens: &'static [&'static str],
}

/// The option lists of the token settings.
pub const VOCABULARIES: [Vocabulary; 8] = [
    Vocabulary { key: "display_model", tokens: &["normal", "as_underlay", "no_display"] },
    Vocabulary { key: "parts_visibility", tokens: &["show_parts_only", "show_original_only", "show_parts_and_original"] },
    Vocabulary { key: "orientation", tokens: &["project_north", "true_north"] },
    Vocabulary { key: "wall_join_display", tokens: &["clean_all_wall_joins", "clean_same_type_wall_joins"] },
    Vocabulary { key: "show_hidden_lines", tokens: &["none", "by_discipline", "all"] },
    Vocabulary { key: "color_scheme_location", tokens: &["background", "foreground"] },
    Vocabulary { key: "underlay_orientation", tokens: &["look_up", "look_down"] },
    Vocabulary { key: "depth_clipping", tokens: &["no_clip", "clip_without_line", "clip_with_line"] },
];

/// The Discipline values and their names.
pub const DISCIPLINES: [(i64, &str); 6] = [
    (1, "architectural"),
    (2, "structural"),
    (4, "mechanical"),
    (8, "electrical"),
    (0x10, "plumbing"),
    (0xfff, "coordination"),
];

/// The display settings of one view (module documentation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewDisplay {
    pub display_model: String,
    pub parts_visibility: String,
    pub orientation: String,
    pub wall_join_display: String,
    /// The integer value (1 = Architectural); `None` = the value the document stored for the view
    /// (`DocumentSettings::view_sources`), 1 when it stored none.
    pub discipline: Option<i64>,
    pub show_hidden_lines: String,
    pub color_scheme_location: String,
    pub sun_path: bool,
    /// The underlay's base level (`None`: no underlay).
    pub underlay_base: Option<ElementId>,
    /// The underlay's top level (`None`: unbounded).
    pub underlay_top: Option<ElementId>,
    pub underlay_orientation: String,
    pub annotation_crop: bool,
    pub depth_clipping: String,
}

impl Default for ViewDisplay {
    fn default() -> Self {
        ViewDisplay {
            display_model: "normal".to_owned(),
            parts_visibility: "show_original_only".to_owned(),
            orientation: "project_north".to_owned(),
            wall_join_display: "clean_all_wall_joins".to_owned(),
            discipline: None,
            show_hidden_lines: "by_discipline".to_owned(),
            color_scheme_location: "background".to_owned(),
            sun_path: false,
            underlay_base: None,
            underlay_top: None,
            underlay_orientation: "look_down".to_owned(),
            annotation_crop: false,
            depth_clipping: "no_clip".to_owned(),
        }
    }
}

/// A value written to one setting.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayValue {
    Token(String),
    Flag(bool),
    Level(Option<ElementId>),
    Discipline(i64),
}

pub fn display_model_code(token: &str) -> Option<i64> {
    match token {
        "normal" => Some(0),
        "as_underlay" => Some(1),
        "no_display" => Some(2),
        _ => None,
    }
}

pub fn display_model_token(code: i64) -> Option<&'static str> {
    ["normal", "as_underlay", "no_display"].get(usize::try_from(code).ok()?).copied()
}

pub fn parts_visibility_code(token: &str) -> Option<i64> {
    match token {
        "show_parts_only" => Some(0),
        "show_original_only" => Some(1),
        "show_parts_and_original" => Some(2),
        _ => None,
    }
}

pub fn parts_visibility_token(code: i64) -> Option<&'static str> {
    ["show_parts_only", "show_original_only", "show_parts_and_original"].get(usize::try_from(code).ok()?).copied()
}

/// The option list of a token setting.
pub fn vocabulary(key: &str) -> Option<&'static [&'static str]> {
    VOCABULARIES.iter().find(|v| v.key == key).map(|v| v.tokens)
}

/// The name of a Discipline value, `None` for a value the reference product does not name.
pub fn discipline_name(value: i64) -> Option<&'static str> {
    DISCIPLINES.iter().find(|d| d.0 == value).map(|d| d.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_tokens_of_their_vocabularies() {
        let d = ViewDisplay::default();
        for (key, value) in [
            ("display_model", &d.display_model),
            ("parts_visibility", &d.parts_visibility),
            ("orientation", &d.orientation),
            ("wall_join_display", &d.wall_join_display),
            ("show_hidden_lines", &d.show_hidden_lines),
            ("color_scheme_location", &d.color_scheme_location),
            ("underlay_orientation", &d.underlay_orientation),
            ("depth_clipping", &d.depth_clipping),
        ] {
            assert!(vocabulary(key).unwrap().contains(&value.as_str()), "{key} = {value}");
        }
        assert_eq!(d.discipline, None);
        assert_eq!(display_model_code(&d.display_model), Some(0));
        assert_eq!(parts_visibility_code(&d.parts_visibility), Some(1));
        assert_eq!(display_model_token(2), Some("no_display"));
        assert_eq!(parts_visibility_token(-1), None);
        assert_eq!(discipline_name(1), Some("architectural"));
        assert_eq!(discipline_name(3), None);
    }
}
