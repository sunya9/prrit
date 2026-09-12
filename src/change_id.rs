//! Change-Id trailer handling, following git's own notion of a trailer block.

use std::fmt::Write as _;

const KEY: &str = "Change-Id";

fn paragraphs(message: &str) -> Vec<&str> {
    let trimmed = message.trim_end();
    if trimmed.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_blank = false;
    let mut blank_start = 0;
    for (i, line) in trimmed.split_inclusive('\n').enumerate() {
        let _ = i;
        let offset = line.as_ptr() as usize - trimmed.as_ptr() as usize;
        if line.trim().is_empty() {
            if !in_blank {
                in_blank = true;
                blank_start = offset;
            }
        } else if in_blank {
            out.push(&trimmed[start..blank_start]);
            start = offset;
            in_blank = false;
        }
    }
    out.push(&trimmed[start..]);
    out.into_iter().map(|p| p.trim_end()).collect()
}

fn is_trailer_line(line: &str) -> bool {
    match line.split_once(": ") {
        Some((key, value)) => {
            !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !value.trim().is_empty()
        }
        None => false,
    }
}

// Only the last paragraph can be a trailer block, and a subject-only message
// never has one.
fn trailer_block(message: &str) -> Option<&str> {
    let paragraphs = paragraphs(message);
    if paragraphs.len() < 2 {
        return None;
    }
    let last = paragraphs[paragraphs.len() - 1];
    last.lines().all(is_trailer_line).then_some(last)
}

pub fn extract_change_id(message: &str) -> Option<String> {
    trailer_block(message)?.lines().find_map(|line| {
        let (key, value) = line.split_once(": ")?;
        (key == KEY).then(|| value.trim().to_string())
    })
}

pub fn append_change_id(message: &str, change_id: &str) -> String {
    let separator = if trailer_block(message).is_some() {
        "\n"
    } else {
        "\n\n"
    };
    format!(
        "{}{}{}: {}\n",
        message.trim_end(),
        separator,
        KEY,
        change_id
    )
}

pub fn strip_change_id(message: &str) -> String {
    let Some(block) = trailer_block(message) else {
        return message.to_string();
    };
    let kept: Vec<&str> = block
        .lines()
        .filter(|line| !line.starts_with(&format!("{KEY}: ")))
        .collect();
    let mut paragraphs = paragraphs(message);
    paragraphs.pop();
    let mut out = paragraphs.join("\n\n");
    if !kept.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&kept.join("\n"));
    }
    out.push('\n');
    out
}

pub fn generate_change_id() -> String {
    // Uniqueness is all that matters; mix time, pid and a pointer so two
    // commits made within the same nanosecond still differ.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let salt = Box::new(0u8);
    let addr = &*salt as *const u8 as usize;
    let mut state = [
        nanos as u64,
        (nanos >> 64) as u64,
        std::process::id() as u64,
        addr as u64,
    ];
    let mut id = String::with_capacity(41);
    id.push('I');
    for round in 0..5u64 {
        // xorshift-style mixing; not cryptographic, just well spread.
        let mut x = state[0]
            ^ state[1].rotate_left(17)
            ^ state[2].wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ state[3]
            ^ round;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        x ^= x >> 33;
        state.rotate_left(1);
        state[3] = x;
        let _ = write!(id, "{x:016x}");
    }
    id.truncate(41);
    id
}

pub fn branch_name(login: &str, change_id: &str) -> String {
    let short: String = change_id.chars().skip(1).take(8).collect();
    format!("prrit/{login}/{short}")
}

pub fn subject_of(message: &str) -> &str {
    message.lines().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSG_WITH_ID: &str = "feat: add login\n\nSome body text.\n\nChange-Id: I0123456789abcdef0123456789abcdef01234567\n";

    #[test]
    fn extracts_the_trailer_value() {
        assert_eq!(
            extract_change_id(MSG_WITH_ID).as_deref(),
            Some("I0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[test]
    fn none_without_trailer() {
        assert_eq!(extract_change_id("feat: add login\n\nbody\n"), None);
        assert_eq!(extract_change_id("feat: only subject"), None);
    }

    #[test]
    fn ignores_change_id_in_the_body() {
        let msg = "feat: x\n\nChange-Id: Iaaaa is mentioned here\n\nSigned-off-by: me\n";
        assert_eq!(extract_change_id(msg), None);
    }

    #[test]
    fn appends_a_new_trailer_block() {
        assert_eq!(
            append_change_id("feat: x\n\nbody\n", "Iabc"),
            "feat: x\n\nbody\n\nChange-Id: Iabc\n"
        );
        assert_eq!(
            append_change_id("feat: x", "Iabc"),
            "feat: x\n\nChange-Id: Iabc\n"
        );
    }

    #[test]
    fn joins_an_existing_trailer_block() {
        assert_eq!(
            append_change_id("feat: x\n\nSigned-off-by: me\n", "Iabc"),
            "feat: x\n\nSigned-off-by: me\nChange-Id: Iabc\n"
        );
    }

    #[test]
    fn strips_only_the_change_id_line() {
        let msg = "feat: x\n\nbody\n\nSigned-off-by: me\nChange-Id: Iabc\n";
        assert_eq!(
            strip_change_id(msg),
            "feat: x\n\nbody\n\nSigned-off-by: me\n"
        );
        assert_eq!(
            strip_change_id(MSG_WITH_ID),
            "feat: add login\n\nSome body text.\n"
        );
        assert_eq!(strip_change_id("feat: x\n"), "feat: x\n");
    }

    #[test]
    fn generates_i_plus_40_hex_and_unique() {
        let a = generate_change_id();
        let b = generate_change_id();
        assert_eq!(a.len(), 41);
        assert!(a.starts_with('I'));
        assert!(a[1..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn derives_branch_name() {
        assert_eq!(
            branch_name("octocat", "I0123456789abcdef0123456789abcdef01234567"),
            "prrit/octocat/01234567"
        );
    }

    #[test]
    fn subject_is_first_line() {
        assert_eq!(subject_of("feat: x\n\nbody\n"), "feat: x");
    }
}
