//! In the order the reference product makes them: an empty name; a name with a prohibited character; (the type class's own
//! validator, not specified: none here); the name trimmed of excess whitespace must not be empty; the trimmed name must be
//! unused among the types of the class. The copy is named exactly the trimmed name (no "copy" name is made). UNKNOWN: the
//! type-specific validator and which properties a copy carries per class; "trimmed" is read as both ends (the trimming
//! operation is not transcribed in the rule).

use asterion_core::Failure;

pub mod codes {
    pub const EMPTY: &str = "type.name_empty";
    pub const PROHIBITED: &str = "type.name_prohibited_characters";
    pub const IN_USE: &str = "type.name_in_use";
}

/// UNKNOWN: the message says "such as".
pub const PROHIBITED_CHARS: [char; 11] = ['{', '}', '[', ']', '|', ';', '<', '>', '?', '`', '~'];

/// Whether a name holds one of the prohibited characters.
pub fn has_prohibited_char(name: &str) -> bool {
    name.chars().any(|c| PROHIBITED_CHARS.contains(&c))
}

/// The name a duplicate takes, or the refusal (raised at the call: an exception in the reference product).
pub fn check_duplicate_name(name: &str, taken: impl Fn(&str) -> bool) -> Result<String, Failure> {
    if name.is_empty() {
        return Err(Failure::error(codes::EMPTY, "name is an empty string."));
    }
    if has_prohibited_char(name) {
        return Err(Failure::error(
            codes::PROHIBITED,
            "name cannot include prohibited characters, such as \"{, }, [, ], |, ;, less-than sign, greater-than sign, ?, `, ~\".",
        ));
    }
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Failure::error(codes::EMPTY, "Once trimmed of excess whitespace, name is an empty string."));
    }
    if taken(trimmed) {
        return Err(Failure::error(codes::IN_USE, "The name is already in use for this element type."));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checks_run_in_the_read_order() {
        let taken = |n: &str| n == "A";
        assert_eq!(check_duplicate_name("", taken).unwrap_err().code, codes::EMPTY);
        assert_eq!(check_duplicate_name(" {A} ", taken).unwrap_err().code, codes::PROHIBITED);
        assert_eq!(check_duplicate_name("   ", taken).unwrap_err().message, "Once trimmed of excess whitespace, name is an empty string.");
        assert_eq!(check_duplicate_name(" A ", taken).unwrap_err().code, codes::IN_USE);
        assert_eq!(check_duplicate_name("  B ", taken).unwrap(), "B");
    }
}
