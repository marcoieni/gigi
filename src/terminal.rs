pub fn strip_control_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut last = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        let Some(seq_len) = control_sequence_len(&bytes[index..]) else {
            index += 1;
            continue;
        };

        if last < index {
            output.push_str(&input[last..index]);
        }
        index += seq_len;
        last = index;
    }

    if last == 0 {
        return input.to_string();
    }

    if last < input.len() {
        output.push_str(&input[last..]);
    }

    output
}

fn control_sequence_len(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.first()?;
    match first {
        0x1b => escape_sequence_len(bytes),
        0x9b => csi_sequence_len(bytes),
        _ => None,
    }
}

fn escape_sequence_len(bytes: &[u8]) -> Option<usize> {
    let second = *bytes.get(1)?;
    match second {
        b'[' => csi_sequence_len(bytes),
        b']' | b'P' | b'_' | b'^' => string_escape_sequence_len(bytes),
        b'@'..=b'_' => Some(2),
        _ => None,
    }
}

fn csi_sequence_len(bytes: &[u8]) -> Option<usize> {
    let start = usize::from(bytes.first() == Some(&0x1b));
    let prefix_len = if start == 1 { 2 } else { 1 };
    let mut index = prefix_len;

    while let Some(&byte) = bytes.get(index) {
        if (0x40..=0x7e).contains(&byte) {
            return Some(index + 1);
        }
        index += 1;
    }

    None
}

fn string_escape_sequence_len(bytes: &[u8]) -> Option<usize> {
    let mut index = 2;
    while let Some(&byte) = bytes.get(index) {
        if byte == 0x07 {
            return Some(index + 1);
        }
        if byte == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
            return Some(index + 2);
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::strip_control_sequences;

    #[test]
    fn strips_ansi_color_codes() {
        let input = "\u{1b}[38;5;141m> \u{1b}[0mREQUIRES_CODE_CHANGES: NO\u{1b}[0m\n\u{1b}[1m## Summary\u{1b}[0m";
        let output = strip_control_sequences(input);
        assert_eq!(output, "> REQUIRES_CODE_CHANGES: NO\n## Summary");
    }

    #[test]
    fn strips_osc_sequences() {
        let input = "before\u{1b}]8;;https://example.com\u{7}link\u{1b}]8;;\u{7}after";
        let output = strip_control_sequences(input);
        assert_eq!(output, "beforelinkafter");
    }
}
