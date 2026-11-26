//! Order keys: fractional indexing (protocol.md, section 5).
//!
//! A port of the npm package `fractional-indexing` 4.0.0 with its default
//! alphabets. Every client must produce the same keys for the same input.

use std::cmp::Ordering;
use std::collections::BTreeMap;

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SMALLEST_INTEGER: &str = "A00000000000000000000000000";

fn digit(c: u8) -> Option<usize> {
    DIGITS.iter().position(|&d| d == c)
}

fn integer_length(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

fn integer_part(key: &str) -> Option<&str> {
    let len = integer_length(*key.as_bytes().first()?)?;
    key.get(..len)
}

fn valid(key: &str) -> bool {
    key != SMALLEST_INTEGER
        && key.bytes().all(|c| digit(c).is_some())
        && integer_part(key).is_some_and(|i| !key[i.len()..].ends_with('0'))
}

/// A digit string between `a` and `b`; `b` absent means "after everything".
fn midpoint(a: &str, b: Option<&str>) -> Option<String> {
    let (ab, bb) = (a.as_bytes(), b.map(str::as_bytes));
    if let Some(bb) = bb {
        let mut n = 0;
        while n < bb.len() && ab.get(n).copied().unwrap_or(b'0') == bb[n] {
            n += 1;
        }
        if n > 0 {
            let rest_a = a.get(n..).unwrap_or("");
            let rest = midpoint(rest_a, Some(&b?[n..]))?;
            return Some(format!("{}{}", &b?[..n], rest));
        }
    }
    let digit_a = match ab.first() {
        Some(&c) => digit(c)?,
        None => 0,
    };
    let digit_b = match bb {
        Some(bb) => digit(*bb.first()?)?,
        None => DIGITS.len(),
    };
    if digit_b > digit_a + 1 {
        // Round half up.
        return Some((DIGITS[(digit_a + digit_b).div_ceil(2)] as char).to_string());
    }
    if let Some(b) = b.filter(|b| b.len() > 1) {
        return Some(b[..1].to_string());
    }
    let rest = midpoint(a.get(1..).unwrap_or(""), None)?;
    Some(format!("{}{}", DIGITS[digit_a] as char, rest))
}

fn increment(integer: &str) -> Option<String> {
    let bytes = integer.as_bytes();
    let head = bytes[0];
    let mut digits = bytes[1..].to_vec();
    let mut carry = true;
    for slot in digits.iter_mut().rev() {
        let d = digit(*slot)? + 1;
        if d == DIGITS.len() {
            *slot = DIGITS[0];
        } else {
            *slot = DIGITS[d];
            carry = false;
            break;
        }
    }
    let head = if carry {
        match head {
            b'Z' => return Some("a0".to_string()),
            b'z' => return None,
            _ => {
                let next = head + 1;
                if next > b'a' {
                    digits.push(DIGITS[0]);
                } else {
                    digits.pop();
                }
                next
            }
        }
    } else {
        head
    };
    let mut out = vec![head];
    out.extend(digits);
    String::from_utf8(out).ok()
}

fn decrement(integer: &str) -> Option<String> {
    let bytes = integer.as_bytes();
    let head = bytes[0];
    let mut digits = bytes[1..].to_vec();
    let mut borrow = true;
    for slot in digits.iter_mut().rev() {
        let d = digit(*slot)?;
        if d == 0 {
            *slot = DIGITS[DIGITS.len() - 1];
        } else {
            *slot = DIGITS[d - 1];
            borrow = false;
            break;
        }
    }
    let head = if borrow {
        match head {
            b'a' => return Some("Zz".to_string()),
            b'A' => return None,
            _ => {
                let next = head - 1;
                if next < b'Z' {
                    digits.push(DIGITS[DIGITS.len() - 1]);
                } else {
                    digits.pop();
                }
                next
            }
        }
    } else {
        head
    };
    let mut out = vec![head];
    out.extend(digits);
    String::from_utf8(out).ok()
}

fn strict_between(a: Option<&str>, b: Option<&str>) -> Option<String> {
    if a.is_some_and(|a| !valid(a)) || b.is_some_and(|b| !valid(b)) {
        return None;
    }
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) if a > b => (Some(b), Some(a)),
        (Some(a), Some(b)) if a == b => return None,
        other => other,
    };
    match (a, b) {
        (None, None) => Some("a0".to_string()),
        (None, Some(b)) => {
            let ib = integer_part(b)?;
            let fb = &b[ib.len()..];
            if ib == SMALLEST_INTEGER {
                return Some(format!("{ib}{}", midpoint("", Some(fb))?));
            }
            if ib < b {
                return Some(ib.to_string());
            }
            decrement(ib)
        }
        (Some(a), None) => {
            let ia = integer_part(a)?;
            let fa = &a[ia.len()..];
            match increment(ia) {
                Some(i) => Some(i),
                None => Some(format!("{ia}{}", midpoint(fa, None)?)),
            }
        }
        (Some(a), Some(b)) => {
            let ia = integer_part(a)?;
            let fa = &a[ia.len()..];
            let ib = integer_part(b)?;
            let fb = &b[ib.len()..];
            if ia == ib {
                return Some(format!("{ia}{}", midpoint(fa, Some(fb))?));
            }
            let i = increment(ia)?;
            if i.as_str() < b {
                return Some(i);
            }
            Some(format!("{ia}{}", midpoint(fa, None)?))
        }
    }
}

/// A key that sorts between `before` and `after`.
///
/// Keys from older data or other clients need not be valid fractional-index
/// keys. For those no exact key exists; the answer then still sorts after
/// `before`, so the client never fails on them.
pub fn key_between(before: Option<&str>, after: Option<&str>) -> String {
    strict_between(before, after).unwrap_or_else(|| match before {
        Some(before) => format!("{before}V"),
        None => "a0".to_string(),
    })
}

/// `n` keys between `before` and `after`, in order.
pub fn n_keys_between(before: Option<&str>, after: Option<&str>, n: usize) -> Vec<String> {
    match (n, before, after) {
        (0, _, _) => vec![],
        (1, _, _) => vec![key_between(before, after)],
        (_, _, None) => {
            let mut keys = vec![key_between(before, None)];
            for _ in 1..n {
                keys.push(key_between(keys.last().map(String::as_str), None));
            }
            keys
        }
        (_, None, _) => {
            let mut keys = vec![key_between(None, after)];
            for _ in 1..n {
                keys.push(key_between(None, keys.last().map(String::as_str)));
            }
            keys.reverse();
            keys
        }
        _ => {
            let mid = n / 2;
            let c = key_between(before, after);
            let mut keys = n_keys_between(before, Some(&c), mid);
            let right = n_keys_between(Some(&c), after, n - mid - 1);
            keys.push(c);
            keys.extend(right);
            keys
        }
    }
}

/// Where moved items land, and which siblings must change with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The keys for the moved items, in order.
    pub orders: Vec<String>,
    /// Sibling id to its new key, for siblings that must change.
    pub renumber: BTreeMap<String, String>,
}

/// Puts `count` items at `index` in a sorted row of siblings (`(id, order)`,
/// without the moved items).
pub fn place(siblings: &[(String, String)], index: usize, count: usize) -> Placement {
    let index = index.min(siblings.len());
    let before = index.checked_sub(1).map(|i| siblings[i].1.as_str());
    let after = siblings.get(index).map(|s| s.1.as_str());
    let fits = match (before, after) {
        (Some(b), Some(a)) => b < a,
        _ => true,
    };
    if fits {
        return Placement {
            orders: n_keys_between(before, after, count),
            renumber: BTreeMap::new(),
        };
    }
    // No key fits between the neighbours: renumber the whole row.
    let fresh = n_keys_between(None, None, siblings.len() + count);
    let mut renumber = BTreeMap::new();
    for (i, (id, order)) in siblings.iter().enumerate() {
        let key = &fresh[if i < index { i } else { i + count }];
        if key != order {
            renumber.insert(id.clone(), key.clone());
        }
    }
    Placement {
        orders: fresh[index..index + count].to_vec(),
        renumber,
    }
}

/// The order of a list: by `order`, then by `id`, as plain strings.
pub fn compare(a: (&str, &str), b: (&str, &str)) -> Ordering {
    a.0.as_bytes()
        .cmp(b.0.as_bytes())
        .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
}
