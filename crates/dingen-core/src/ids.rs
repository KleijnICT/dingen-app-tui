//! ULIDs and client ids (protocol.md, section 2).

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn random_chars(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    getrandom::getrandom(&mut bytes).expect("the system has no source of randomness");
    bytes
        .iter()
        .map(|b| ALPHABET[(b % 32) as usize] as char)
        .collect()
}

/// A new ULID for the given time in milliseconds.
pub fn ulid_at(mut time_ms: u64) -> String {
    let mut time = [b'0'; 10];
    for slot in time.iter_mut().rev() {
        *slot = ALPHABET[(time_ms % 32) as usize];
        time_ms /= 32;
    }
    let mut id = String::from_utf8(time.to_vec()).expect("the alphabet is ASCII");
    id.push_str(&random_chars(16));
    id
}

pub fn ulid() -> String {
    ulid_at(crate::hlc::wall_now())
}

/// A new client id: 8 random characters.
pub fn client_id() -> String {
    random_chars(8)
}

pub fn is_ulid(s: &str) -> bool {
    s.len() == 26 && s.bytes().all(|b| ALPHABET.contains(&b))
}
