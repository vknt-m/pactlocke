//! The Generation III character set (English), for nicknames.

/// The string terminator.
pub const END: u8 = 0xFF;

const TABLE: &[(u8, char)] = &[
    (0x00, ' '),
    (0x1B, 'é'),
    (0xAB, '!'),
    (0xAC, '?'),
    (0xAD, '.'),
    (0xAE, '-'),
    (0xB0, '…'),
    (0xB1, '“'),
    (0xB2, '”'),
    (0xB3, '‘'),
    (0xB4, '’'),
    (0xB5, '♂'),
    (0xB6, '♀'),
    (0xB8, ','),
    (0xBA, '/'),
];

fn decode_byte(byte: u8) -> char {
    match byte {
        0xA1..=0xAA => char::from(b'0' + (byte - 0xA1)),
        0xBB..=0xD4 => char::from(b'A' + (byte - 0xBB)),
        0xD5..=0xEE => char::from(b'a' + (byte - 0xD5)),
        _ => TABLE
            .iter()
            .find(|(b, _)| *b == byte)
            .map_or(char::REPLACEMENT_CHARACTER, |(_, c)| *c),
    }
}

fn encode_char(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(0xA1 + (c as u8 - b'0')),
        'A'..='Z' => Some(0xBB + (c as u8 - b'A')),
        'a'..='z' => Some(0xD5 + (c as u8 - b'a')),
        _ => TABLE.iter().find(|(_, ch)| *ch == c).map(|(b, _)| *b),
    }
}

/// Text up to the terminator.
pub fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|b| **b != END)
        .map(|b| decode_byte(*b))
        .collect()
}

/// `text` in the game's characters, padded with the terminator to `len`.
pub fn encode(text: &str, len: usize) -> Result<Vec<u8>, String> {
    if text.is_empty() {
        return Err("a name can't be empty".into());
    }
    let mut out = text
        .chars()
        .map(|c| encode_char(c).ok_or_else(|| format!("the game has no {c:?} character")))
        .collect::<Result<Vec<u8>, _>>()?;
    if out.len() > len {
        return Err(format!("a name can be at most {len} characters"));
    }
    out.resize(len, END);
    Ok(out)
}
