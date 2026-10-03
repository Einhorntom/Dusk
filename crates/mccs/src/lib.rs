#![forbid(unsafe_code)]

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
    InvalidStructure,
    InvalidHexCode(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStructure => f.write_str("invalid MCCS capability string structure"),
            Self::InvalidHexCode(value) => write!(f, "invalid hexadecimal MCCS code: {value}"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn parse_capabilities(input: &str) -> Result<Capabilities, ParseError> {
    let tokens = tokenize(input)?;
    let mut vcp = BTreeMap::new();
    let mut model = None;
    let mut mccs_version = None;
    let mut index = 0;

    while index < tokens.len() {
        match tokens[index].as_str() {
            "vcp" => {
                index += 1;
                if tokens.get(index).map(String::as_str) != Some("(") {
                    return Err(ParseError::InvalidStructure);
                }
                index += 1;
                while index < tokens.len() && tokens[index] != ")" {
                    let code = parse_hex(&tokens[index])?;
                    index += 1;
                    let mut values = Vec::new();
                    if tokens.get(index).map(String::as_str) == Some("(") {
                        index += 1;
                        let mut depth = 1usize;
                        while index < tokens.len() && depth > 0 {
                            match tokens[index].as_str() {
                                "(" => depth += 1,
                                ")" => depth -= 1,
                                value if depth == 1 => values.push(parse_hex(value)?),
                                _ => {}
                            }
                            index += 1;
                        }
                        if depth != 0 {
                            return Err(ParseError::InvalidStructure);
                        }
                    }
                    vcp.insert(code, values);
                }
                if tokens.get(index).map(String::as_str) != Some(")") {
                    return Err(ParseError::InvalidStructure);
                }
                index += 1;
            }
            "model" | "mccs_ver" => {
                let key = tokens[index].as_str();
                index += 1;
                if tokens.get(index).map(String::as_str) != Some("(") {
                    return Err(ParseError::InvalidStructure);
                }
                index += 1;
                let start = index;
                while index < tokens.len() && tokens[index] != ")" {
                    index += 1;
                }
                if tokens.get(index).map(String::as_str) != Some(")") {
                    return Err(ParseError::InvalidStructure);
                }
                let value = tokens[start..index].join(" ");
                if key == "model" {
                    model = Some(value);
                } else {
                    mccs_version = Some(value);
                }
                index += 1;
            }
            "(" | ")" => index += 1,
            _ => index += 1,
        }
    }

    Ok(Capabilities {
        vcp,
        model,
        mccs_version,
    })
}

fn tokenize(input: &str) -> Result<Vec<String>, ParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for character in input.chars() {
        match character {
            '(' => {
                if !current.trim().is_empty() {
                    tokens.push(current.trim().to_owned());
                    current.clear();
                }
                tokens.push("(".to_owned());
                depth += 1;
            }
            ')' => {
                if !current.trim().is_empty() {
                    tokens.push(current.trim().to_owned());
                    current.clear();
                }
                depth = depth.checked_sub(1).ok_or(ParseError::InvalidStructure)?;
                tokens.push(")".to_owned());
            }
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        tokens.push(current.trim().to_owned());
    }
    if depth != 0 {
        return Err(ParseError::InvalidStructure);
    }
    Ok(tokens)
}

fn parse_hex(value: &str) -> Result<u8, ParseError> {
    u8::from_str_radix(value, 16).map_err(|_| ParseError::InvalidHexCode(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn rejects_unbalanced_capability_strings() {
        assert_eq!(
            parse_capabilities("(vcp(10 60(11))"),
            Err(ParseError::InvalidStructure)
        );
    }

    #[test]
    fn parses_captured_l32p30_capabilities() {
        let captured = include_str!("../tests/data/l32p-30.cap");
        let parsed = parse_capabilities(captured).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("Lenovo L32p-30"));
        assert_eq!(parsed.mccs_version.as_deref(), Some("2.2"));
        assert!(parsed.vcp.contains_key(&0x10));
        assert_eq!(parsed.vcp.get(&0x60), Some(&vec![0x11, 0x0f, 0x31]));
        assert_eq!(
            parsed.vcp.get(&0x14),
            Some(&vec![0x01, 0x05, 0x06, 0x08, 0x0b, 0x0e, 0x0f])
        );
    }
}
