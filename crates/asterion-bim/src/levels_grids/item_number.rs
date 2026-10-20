//! The item-number incrementer: how a name like "A101" becomes "A102" (new grids, levels, sheets, sections).
//!
//! (the reference product, the string incrementer `IncrementStringManager` with its three strategy objects). The document
//! keeps, per key, the LAST NUMBER STRING; a new number is that string incremented in place, using the length of the base
//! string as the minimum width. UNKNOWN: the key class `NameEquivalenceClass` and the base string each caller passes: the
//! engine keeps ONE cursor per kind of item (for grids [`crate::model::BimModel::last_grid_name`]) and falls back to "1"
//! where the reference product stores the base string of its caller.

fn strategy(c: char) -> Option<u8> {
    if c.is_ascii_alphabetic() {
        Some(0)
    } else if c.is_ascii_digit() {
        Some(1)
    } else if c.is_alphabetic() {
        Some(2)
    } else {
        None
    }
}

/// . `None` when the string cannot be changed (its last character belongs to no strategy, the run is not a single repeated
/// letter, a decrement runs out).
pub fn increment_string(s: &str, width: usize, decrement: bool) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    if n == 0 {
        return None;
    }
    let width = width.min(n);
    let st = strategy(chars[n - 1])?;
    // the trailing run of characters of one strategy
    let mut start = n;
    while start > 0 && strategy(chars[start - 1]) == Some(st) {
        start -= 1;
    }
    // inside the run the leading "0" characters are set aside while more than one character remains
    let mut z = start;
    while st == 1 && z + 1 < n && chars[z] == '0' {
        z += 1;
    }
    let right: Vec<char> = chars[z..].to_vec();
    let mut left: Vec<char> = chars[..z].to_vec();
    let max_width = width.saturating_sub(left.len());
    let new_right = match st {
        1 => change_digits(&right, max_width, decrement)?,
        0 => change_letters(&right, decrement)?,
        _ => change_other(&right, decrement)?,
    };
    // it grew by a character while zeros had been set aside: one of those zeros is dropped from the left part
    if new_right.len() > right.len() && z > start {
        left.pop();
    }
    Some(left.into_iter().chain(new_right).collect())
}

fn change_digits(right: &[char], max_width: usize, decrement: bool) -> Option<Vec<char>> {
    let mut d: Vec<u8> = right.iter().map(|c| c.to_digit(10).map(|v| v as u8)).collect::<Option<_>>()?;
    if decrement {
        let mut i = d.len();
        loop {
            if i == 0 {
                return None; // the borrow leaves the first digit
            }
            i -= 1;
            if d[i] == 0 {
                d[i] = 9;
            } else {
                d[i] -= 1;
                break;
            }
        }
    } else {
        let mut i = d.len();
        loop {
            if i == 0 {
                d.insert(0, 1); // the carry leaves the first digit
                break;
            }
            i -= 1;
            if d[i] == 9 {
                d[i] = 0;
            } else {
                d[i] += 1;
                break;
            }
        }
    }
    // trimmed or left-padded with "0" to max(significant digit count, width)
    let sig = d.iter().position(|v| *v != 0).map_or(1, |p| d.len() - p);
    let target = sig.max(max_width);
    let mut out: Vec<char> = d[d.len() - sig..].iter().map(|v| char::from(b'0' + v)).collect();
    while out.len() < target {
        out.insert(0, '0');
    }
    Some(out)
}

fn change_letters(right: &[char], decrement: bool) -> Option<Vec<char>> {
    let c = *right.first()?;
    if right.iter().any(|x| *x != c) {
        return None; // "AZ" cannot be changed
    }
    let (first, last) = if c.is_ascii_uppercase() { ('A', 'Z') } else { ('a', 'z') };
    if decrement {
        return if c == first {
            // wraps to the last letter and drops one character; a run of one fails
            (right.len() > 1).then(|| vec![last; right.len() - 1])
        } else {
            let p = char::from(c as u8 - 1);
            Some(vec![p; right.len()])
        };
    }
    if c == last {
        // every character wraps and ONE MORE character of the wrapped letter is appended
        Some(vec![first; right.len() + 1])
    } else {
        Some(vec![char::from(c as u8 + 1); right.len()])
    }
}

fn change_other(right: &[char], decrement: bool) -> Option<Vec<char>> {
    let c = *right.first()?;
    if right.iter().any(|x| *x != c) {
        return None;
    }
    let code = c as u32;
    let next = char::from_u32(if decrement { code.checked_sub(1)? } else { code + 1 })?;
    Some(vec![next; right.len()])
}

/// The next number after `previous`: the first name is "1"; otherwise the stored string incremented with its own length as
/// the width. A string that cannot be incremented gives "1" (the reference product stores the base string of the caller,
/// which is not specified).
pub fn next_item_number(previous: Option<&str>) -> String {
    match previous {
        None => "1".to_owned(),
        Some(p) => increment_string(p, p.chars().count(), false).unwrap_or_else(|| "1".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_examples_of_the_row() {
        // "A101" -> "A102", "A9" -> "A10", "A099" -> "A100" (width kept), "Z" -> "AA", "AA" -> "BB", "AZ" fails
        for (from, to) in [("A101", "A102"), ("A9", "A10"), ("A099", "A100"), ("Z", "AA"), ("AA", "BB"), ("ZZ", "AAA"), ("a", "b"), ("09", "10"), ("A001", "A002"), ("999", "1000"), ("A", "B")] {
            assert_eq!(increment_string(from, from.chars().count(), false).as_deref(), Some(to), "{from}");
        }
        assert_eq!(increment_string("AZ", 2, false), None);
        assert_eq!(increment_string("A-", 2, false), None, "a last character no strategy accepts");
        assert_eq!(next_item_number(None), "1");
        assert_eq!(next_item_number(Some("AZ")), "1");
        assert_eq!(next_item_number(Some("G1-B")), "G1-C");
    }

    #[test]
    fn the_decrement_of_the_row() {
        // digits borrow and fail when the borrow leaves the first digit; letters wrap a -> z and drop the last character
        assert_eq!(increment_string("A102", 4, true).as_deref(), Some("A101"));
        assert_eq!(increment_string("A100", 4, true).as_deref(), Some("A099"));
        assert_eq!(increment_string("0", 1, true), None);
        assert_eq!(increment_string("AA", 2, true).as_deref(), Some("Z"));
        assert_eq!(increment_string("A", 1, true), None);
        assert_eq!(increment_string("C", 1, true).as_deref(), Some("B"));
    }
}
