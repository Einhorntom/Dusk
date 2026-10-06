//! Monitor identity (SPEC-MON-1), as pure functions: EDID parsing and ID
//! assignment. An ID is the model name plus the EDID serial number, so it
//! survives reconnecting and reordering; without a usable serial it falls
//! back to the model name plus the connection position, flagged unstable.

/// The identifying fields of a 128-byte EDID base block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Edid {
    /// Three-letter PNP ID, e.g. `LEN`.
    pub manufacturer: String,
    pub product: u16,
    /// The numeric serial; 0 when unused.
    pub serial_number: u32,
    /// The serial-number text descriptor (`0xFF`), if any.
    pub serial_text: Option<String>,
    /// The monitor-name descriptor (`0xFC`), if any.
    pub name: Option<String>,
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

pub(crate) fn parse_edid(bytes: &[u8]) -> Option<Edid> {
    if bytes.len() < 128 || bytes[..8] != HEADER {
        return None;
    }
    let packed = u16::from_be_bytes([bytes[8], bytes[9]]);
    let letter = |shift: u16| char::from(b'@' + ((packed >> shift) & 0x1F) as u8);
    let manufacturer: String = [letter(10), letter(5), letter(0)].into_iter().collect();
    let mut edid = Edid {
        manufacturer,
        product: u16::from_le_bytes([bytes[10], bytes[11]]),
        serial_number: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        serial_text: None,
        name: None,
    };
    // Four 18-byte descriptors; text descriptors start 00 00 00 <tag> 00.
    for descriptor in bytes[54..126].as_chunks::<18>().0 {
        if descriptor[..3] != [0, 0, 0] || descriptor[4] != 0 {
            continue;
        }
        let text = descriptor_text(&descriptor[5..]);
        match descriptor[3] {
            0xFF => edid.serial_text = text,
            0xFC => edid.name = text,
            _ => {}
        }
    }
    Some(edid)
}

/// Descriptor text ends at a line feed and is padded with spaces.
fn descriptor_text(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    let text: String = bytes[..end]
        .iter()
        .filter(|byte| byte.is_ascii_graphic() || **byte == b' ')
        .map(|byte| char::from(*byte))
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// `\\?\DISPLAY#LEN66C9#4&e219404&0&UID8261#{guid}` becomes the device
/// instance `DISPLAY\LEN66C9\4&e219404&0&UID8261`, whose registry key holds
/// the EDID.
pub(crate) fn device_instance(device_path: &str) -> Option<String> {
    let path = device_path.strip_prefix(r"\\?\")?;
    let path = match path.rfind("#{") {
        Some(guid) => &path[..guid],
        None => path,
    };
    let parts: Vec<&str> = path.split('#').collect();
    (parts.len() == 3 && parts.iter().all(|part| !part.is_empty())).then(|| parts.join(r"\"))
}

/// What is known about one physical monitor before IDs are assigned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Candidate {
    pub name: String,
    pub serial: Option<String>,
}

impl Candidate {
    /// Prefers the EDID's model name and serial; falls back to Windows'
    /// description of the monitor.
    pub(crate) fn new(edid: Option<&Edid>, description: &str) -> Self {
        let description = description.trim();
        let name = edid
            .and_then(|edid| edid.name.clone())
            .or_else(|| (!description.is_empty()).then(|| description.to_owned()))
            .or_else(|| edid.map(|edid| format!("{}{:04X}", edid.manufacturer, edid.product)))
            .unwrap_or_else(|| "Monitor".to_owned());
        let serial = edid.and_then(|edid| {
            edid.serial_text
                .clone()
                .or_else(|| (edid.serial_number != 0).then(|| edid.serial_number.to_string()))
        });
        Self { name, serial }
    }
}

/// `(id, unstable)` for each candidate, in order. A serial shared by two
/// monitors (some models repeat one) cannot tell them apart, so those fall
/// back to their positions, as do monitors without a serial.
pub(crate) fn assign_ids(candidates: &[Candidate]) -> Vec<(String, bool)> {
    let positional = |index: usize| (format!("{}#{index}", candidates[index].name), true);
    let mut ids: Vec<(String, bool)> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| match &candidate.serial {
            Some(serial) => (format!("{}#{serial}", candidate.name), false),
            None => positional(index),
        })
        .collect();
    // Positional IDs are unique among themselves, so one pass that demotes
    // every stable ID involved in a clash leaves all IDs unique.
    let clashing: Vec<usize> = (0..ids.len())
        .filter(|&index| {
            !ids[index].1
                && ids
                    .iter()
                    .enumerate()
                    .any(|(other, id)| other != index && id.0 == ids[index].0)
        })
        .collect();
    for index in clashing {
        ids[index] = positional(index);
    }
    ids
}

/// Pairs the physical monitors of one Windows display with its outputs.
/// Usually there is one of each; when a display is duplicated, Windows
/// gives no documented order, so outputs are paired only when the counts
/// match (all outputs, or the external ones alone).
pub(crate) fn pair_outputs<T: Clone>(
    physical_count: usize,
    outputs: &[T],
    is_external: impl Fn(&T) -> bool,
) -> Vec<Option<T>> {
    if outputs.len() == physical_count {
        return outputs.iter().cloned().map(Some).collect();
    }
    let external: Vec<T> = outputs
        .iter()
        .filter(|output| is_external(output))
        .cloned()
        .collect();
    if external.len() == physical_count {
        return external.into_iter().map(Some).collect();
    }
    vec![None; physical_count]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An EDID base block with optional name and serial descriptors.
    fn edid(name: Option<&str>, serial_text: Option<&str>, serial_number: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..8].copy_from_slice(&HEADER);
        // "LEN": L=12, E=5, N=14.
        let packed: u16 = (12 << 10) | (5 << 5) | 14;
        bytes[8..10].copy_from_slice(&packed.to_be_bytes());
        bytes[10..12].copy_from_slice(&0x66C9u16.to_le_bytes());
        bytes[12..16].copy_from_slice(&serial_number.to_le_bytes());
        let mut slot = 54;
        for (tag, text) in [(0xFC, name), (0xFF, serial_text)] {
            if let Some(text) = text {
                let descriptor = &mut bytes[slot..slot + 18];
                descriptor[3] = tag;
                let mut padded = format!("{text}\n").into_bytes();
                padded.resize(13, b' ');
                descriptor[5..].copy_from_slice(&padded);
                slot += 18;
            }
        }
        bytes
    }

    fn candidate(name: &str, serial: Option<&str>) -> Candidate {
        Candidate {
            name: name.into(),
            serial: serial.map(str::to_owned),
        }
    }

    #[test]
    fn edid_identity_fields_are_parsed() {
        let parsed = parse_edid(&edid(Some("L32p-30"), Some("U512AY02"), 0)).unwrap();
        assert_eq!(
            parsed,
            Edid {
                manufacturer: "LEN".into(),
                product: 0x66C9,
                serial_number: 0,
                serial_text: Some("U512AY02".into()),
                name: Some("L32p-30".into()),
            }
        );
        let bare = parse_edid(&edid(None, None, 16_843_009)).unwrap();
        assert_eq!((bare.name, bare.serial_text), (None, None));
        assert_eq!(bare.serial_number, 16_843_009);
    }

    #[test]
    fn invalid_edids_are_rejected() {
        assert_eq!(parse_edid(&[0u8; 128]), None);
        assert_eq!(parse_edid(&edid(Some("x"), None, 0)[..127]), None);
    }

    #[test]
    fn device_paths_map_to_their_registry_instances() {
        assert_eq!(
            device_instance(
                r"\\?\DISPLAY#LEN66C9#4&e219404&0&UID8261#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
            )
            .as_deref(),
            Some(r"DISPLAY\LEN66C9\4&e219404&0&UID8261")
        );
        assert_eq!(device_instance(r"DISPLAY#LEN66C9#x"), None);
        assert_eq!(device_instance(r"\\?\DISPLAY#LEN66C9#{guid}"), None);
    }

    #[test]
    fn candidates_prefer_the_edid_and_fall_back_to_the_description() {
        let full = parse_edid(&edid(Some("L32p-30"), Some("U512AY02"), 7)).unwrap();
        assert_eq!(
            Candidate::new(Some(&full), "Generic PnP Monitor"),
            candidate("L32p-30", Some("U512AY02"))
        );
        let numeric = parse_edid(&edid(None, None, 1234)).unwrap();
        assert_eq!(
            Candidate::new(Some(&numeric), "Generic PnP Monitor"),
            candidate("Generic PnP Monitor", Some("1234"))
        );
        assert_eq!(
            Candidate::new(Some(&numeric), " "),
            candidate("LEN66C9", Some("1234"))
        );
        assert_eq!(Candidate::new(None, "Dell"), candidate("Dell", None));
    }

    #[test]
    fn serials_give_stable_ids_and_missing_ones_fall_back_to_position() {
        assert_eq!(
            assign_ids(&[
                candidate("L32p-30", Some("U512AY02")),
                candidate("Dell", None)
            ]),
            [
                ("L32p-30#U512AY02".to_owned(), false),
                ("Dell#1".to_owned(), true)
            ]
        );
    }

    #[test]
    fn a_shared_serial_cannot_identify_either_monitor() {
        assert_eq!(
            assign_ids(&[
                candidate("Cheap", Some("123456")),
                candidate("Cheap", Some("123456")),
                candidate("Other", Some("123456")),
            ]),
            [
                ("Cheap#0".to_owned(), true),
                ("Cheap#1".to_owned(), true),
                ("Other#123456".to_owned(), false)
            ]
        );
    }

    #[test]
    fn a_numeric_serial_never_clashes_with_a_position() {
        let ids = assign_ids(&[candidate("Twin", None), candidate("Twin", Some("0"))]);
        assert_eq!(
            ids,
            [("Twin#0".to_owned(), true), ("Twin#1".to_owned(), true)]
        );
    }

    #[test]
    fn outputs_pair_with_physical_monitors_only_when_counts_match() {
        let external = |output: &&str| !output.starts_with("internal");
        assert_eq!(pair_outputs(1, &["hdmi"], external), [Some("hdmi")]);
        assert_eq!(
            pair_outputs(1, &["internal", "hdmi"], external),
            [Some("hdmi")]
        );
        assert_eq!(pair_outputs(2, &["dp"], external), [None, None]);
    }
}
