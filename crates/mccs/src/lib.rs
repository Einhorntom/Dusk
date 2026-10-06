#![forbid(unsafe_code)]

//! MCCS capability strings, e.g.
//! `(prot(monitor)type(LCD)model(L32p-30)vcp(10 12 60(11 0F))mccs_ver(2.2))`.
//!
//! Monitors often send slightly malformed strings, so parsing is lenient:
//! a missing or extra parenthesis (including a truncated string), codes run
//! together (`vcp(101214)`), upper-case keys and unknown tokens are
//! tolerated, and whatever is readable is used. Only a string without any
//! `vcp` section is rejected, since it offers nothing to control.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub vcp: BTreeMap<u8, Vec<u8>>,
    pub model: Option<String>,
    pub mccs_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// The string has no `vcp(...)` section.
    NoVcpSection,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVcpSection => f.write_str("the MCCS capability string lists no VCP codes"),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Open,
    Close,
    Word(String),
}

pub fn parse_capabilities(input: &str) -> Result<Capabilities, ParseError> {
    let tokens = tokenize(input);
    let mut capabilities = Capabilities {
        vcp: BTreeMap::new(),
        model: None,
        mccs_version: None,
    };
    let mut found_vcp = false;
    let mut index = 0;
    while index < tokens.len() {
        let key = match (&tokens[index], tokens.get(index + 1)) {
            (Token::Word(word), Some(Token::Open)) => word.to_ascii_lowercase(),
            _ => {
                // Outer parentheses and the contents of other sections.
                index += 1;
                continue;
            }
        };
        let open = index + 1;
        let close = group_end(&tokens, open);
        let content = &tokens[open + 1..close];
        match key.as_str() {
            "vcp" => {
                found_vcp = true;
                parse_vcp(content, &mut capabilities.vcp);
            }
            "model" => capabilities.model = text(content),
            "mccs_ver" => capabilities.mccs_version = text(content),
            _ => {}
        }
        index = close + 1;
    }
    if found_vcp {
        Ok(capabilities)
    } else {
        Err(ParseError::NoVcpSection)
    }
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let flush = |word: &mut String, tokens: &mut Vec<Token>| {
        if !word.is_empty() {
            tokens.push(Token::Word(std::mem::take(word)));
        }
    };
    for character in input.chars() {
        match character {
            '(' | ')' => {
                flush(&mut word, &mut tokens);
                tokens.push(if character == '(' {
                    Token::Open
                } else {
                    Token::Close
                });
            }
            character if character.is_whitespace() || character == '\0' => {
                flush(&mut word, &mut tokens);
            }
            character => word.push(character),
        }
    }
    flush(&mut word, &mut tokens);
    tokens
}

/// The index of the `Close` matching the `Open` at `open`, or the end of
/// the tokens if the string was cut off.
fn group_end(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Open => depth += 1,
            Token::Close => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            Token::Word(_) => {}
        }
    }
    tokens.len()
}

/// `10 14(01 05) 60(11 0F)`: codes, each optionally followed by its values.
fn parse_vcp(content: &[Token], vcp: &mut BTreeMap<u8, Vec<u8>>) {
    let mut index = 0;
    let mut last_code = None;
    while index < content.len() {
        match &content[index] {
            Token::Word(word) => {
                last_code = None;
                for code in hex_bytes(word) {
                    vcp.entry(code).or_default();
                    last_code = Some(code);
                }
                index += 1;
            }
            Token::Open => {
                let close = group_end(content, index);
                // Values belong to the code just before them; a group after
                // anything else (or nested deeper) is skipped.
                if let Some(code) = last_code.take() {
                    let values = group_values(&content[index + 1..close]);
                    let known = vcp.entry(code).or_default();
                    if known.is_empty() {
                        *known = values;
                    } else {
                        // A repeated section adds the values it did not list yet.
                        for value in values {
                            if !known.contains(&value) {
                                known.push(value);
                            }
                        }
                    }
                }
                index = close + 1;
            }
            Token::Close => index += 1,
        }
    }
}

/// The values of one code's group, in order; deeper groups are skipped.
fn group_values(content: &[Token]) -> Vec<u8> {
    let mut values = Vec::new();
    let mut depth = 0usize;
    for token in content {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 => values.extend(hex_bytes(word)),
            Token::Word(_) => {}
        }
    }
    values
}

/// `60` -> [0x60]; `101214` -> [0x10, 0x12, 0x14]; anything else -> [].
fn hex_bytes(word: &str) -> Vec<u8> {
    if word.is_empty() || !word.chars().all(|character| character.is_ascii_hexdigit()) {
        return Vec::new();
    }
    if word.len() <= 2 {
        return u8::from_str_radix(word, 16).into_iter().collect();
    }
    if !word.len().is_multiple_of(2) {
        return Vec::new();
    }
    (0..word.len())
        .step_by(2)
        .filter_map(|start| u8::from_str_radix(&word[start..start + 2], 16).ok())
        .collect()
}

fn text(content: &[Token]) -> Option<String> {
    let words: Vec<&str> = content
        .iter()
        .filter_map(|token| match token {
            Token::Word(word) => Some(word.as_str()),
            _ => None,
        })
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const L32P30: &str = include_str!("../tests/data/l32p-30.cap");

    fn codes(parsed: &Capabilities) -> Vec<u8> {
        parsed.vcp.keys().copied().collect()
    }

    #[test]
    fn parses_reference_monitor_control_and_enum_codes() {
        let parsed = parse_capabilities(
            "(prot(monitor)type(LCD)model(Lenovo L32p-30) vcp(10 14(01 05) 60(11 0F 31)) mccs_ver(2.2))",
        )
        .unwrap();
        assert_eq!(parsed.model.as_deref(), Some("Lenovo L32p-30"));
        assert_eq!(parsed.mccs_version.as_deref(), Some("2.2"));
        assert_eq!(parsed.vcp.get(&0x10), Some(&vec![]));
        assert_eq!(parsed.vcp.get(&0x14), Some(&vec![0x01, 0x05]));
        assert_eq!(parsed.vcp.get(&0x60), Some(&vec![0x11, 0x0f, 0x31]));
    }

    #[test]
    fn parses_captured_l32p30_capabilities() {
        let parsed = parse_capabilities(L32P30).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("Lenovo L32p-30"));
        assert_eq!(parsed.mccs_version.as_deref(), Some("2.2"));
        assert!(parsed.vcp.contains_key(&0x10));
        assert_eq!(parsed.vcp.get(&0x60), Some(&vec![0x11, 0x0f, 0x31]));
        assert_eq!(
            parsed.vcp.get(&0x14),
            Some(&vec![0x01, 0x05, 0x06, 0x08, 0x0b, 0x0e, 0x0f])
        );
    }

    #[test]
    fn unbalanced_and_truncated_strings_keep_what_is_readable() {
        // One closing parenthesis short.
        let short = parse_capabilities("(vcp(10 60(11))").unwrap();
        assert_eq!(short.vcp.get(&0x60), Some(&vec![0x11]));
        // Cut off inside a value list, with no closing parentheses.
        let cut = parse_capabilities("(prot(monitor)vcp(10 12 14(05 08").unwrap();
        assert_eq!(codes(&cut), [0x10, 0x12, 0x14]);
        assert_eq!(cut.vcp.get(&0x14), Some(&vec![0x05, 0x08]));
        // Extra closing parentheses and no outer ones.
        let extra = parse_capabilities("vcp(10 12)) model(X))) mccs_ver(2.1)").unwrap();
        assert_eq!(codes(&extra), [0x10, 0x12]);
        assert_eq!(extra.model.as_deref(), Some("X"));
    }

    #[test]
    fn codes_run_together_are_split_into_bytes() {
        let parsed = parse_capabilities("(vcp(021012 14(050608)60(0F11)))").unwrap();
        assert_eq!(codes(&parsed), [0x02, 0x10, 0x12, 0x14, 0x60]);
        assert_eq!(parsed.vcp.get(&0x14), Some(&vec![0x05, 0x06, 0x08]));
        assert_eq!(parsed.vcp.get(&0x60), Some(&vec![0x0f, 0x11]));
    }

    #[test]
    fn upper_case_keys_junk_tokens_and_deeper_groups_are_tolerated() {
        let parsed = parse_capabilities(
            "(VCP(10 zz 12 DC(00 (01 02) 03) (77) 1)MCCS_VER(2.0)vcpname(10(Brightness)))",
        )
        .unwrap();
        assert_eq!(codes(&parsed), [0x01, 0x10, 0x12, 0xdc]);
        assert_eq!(parsed.vcp.get(&0xdc), Some(&vec![0x00, 0x03]));
        assert_eq!(parsed.mccs_version.as_deref(), Some("2.0"));
    }

    #[test]
    fn repeated_vcp_sections_are_merged() {
        let parsed = parse_capabilities("vcp(10 60(11)) vcp(12 60(0F 11))").unwrap();
        assert_eq!(codes(&parsed), [0x10, 0x12, 0x60]);
        assert_eq!(parsed.vcp.get(&0x60), Some(&vec![0x11, 0x0f]));
    }

    #[test]
    fn a_string_without_vcp_codes_is_rejected() {
        assert_eq!(
            parse_capabilities("(prot(monitor)type(LCD)model(X))"),
            Err(ParseError::NoVcpSection)
        );
        assert_eq!(parse_capabilities(""), Err(ParseError::NoVcpSection));
    }

    #[test]
    fn every_truncation_of_a_real_string_parses_without_panicking() {
        let vcp_start = L32P30.find("vcp(").expect("the capture has a vcp section");
        for end in 0..=L32P30.len() {
            let result = parse_capabilities(&L32P30[..end]);
            // Once the vcp section has started, the string is usable.
            assert_eq!(
                result.is_ok(),
                end >= vcp_start + 4,
                "prefix of {end} bytes"
            );
        }
        let cut = L32P30.find("16 18").unwrap();
        let parsed = parse_capabilities(&L32P30[..cut]).unwrap();
        assert_eq!(codes(&parsed), [0x02, 0x04, 0x05, 0x08, 0x10, 0x12, 0x14]);
        assert_eq!(parsed.model.as_deref(), Some("Lenovo L32p-30"));
    }
}
