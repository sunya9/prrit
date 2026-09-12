//! Gerrit's `refs/for/<base>%opt,opt` destination syntax.

use crate::error::{Error, Result};

#[derive(Debug, PartialEq, Eq)]
pub struct ForRef {
    pub base: String,
    pub draft: bool,
}

const PREFIX: &str = "refs/for/";

// Only options with a GitHub counterpart are accepted so a typo never
// silently changes behavior.
pub fn parse_for_ref(dst: &str) -> Result<ForRef> {
    let rest = dst
        .strip_prefix(PREFIX)
        .ok_or_else(|| Error::msg(format!("push to refs/for/<base> (got {dst})")))?;
    let (base, opts) = rest.split_once('%').unwrap_or((rest, ""));
    if base.is_empty() {
        return Err(Error::msg(format!("push to refs/for/<base> (got {dst})")));
    }
    let mut draft = false;
    for opt in opts.split(',').filter(|o| !o.is_empty()) {
        match opt {
            "wip" => draft = true,
            "ready" => draft = false,
            other => {
                return Err(Error::msg(format!(
                    "unsupported push option %{other} (supported: wip, ready)"
                )))
            }
        }
    }
    Ok(ForRef {
        base: base.to_string(),
        draft,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(dst: &str) -> ForRef {
        parse_for_ref(dst).unwrap()
    }

    #[test]
    fn extracts_base() {
        assert_eq!(
            ok("refs/for/main"),
            ForRef {
                base: "main".into(),
                draft: false
            }
        );
        assert_eq!(ok("refs/for/release/1.0").base, "release/1.0");
    }

    #[test]
    fn maps_wip_and_ready() {
        assert!(ok("refs/for/main%wip").draft);
        assert!(!ok("refs/for/main%ready").draft);
        assert!(!ok("refs/for/main%wip,ready").draft);
    }

    #[test]
    fn rejects_other_refs_and_options() {
        assert!(parse_for_ref("refs/heads/main")
            .unwrap_err()
            .to_string()
            .contains("refs/for/<base>"));
        assert!(parse_for_ref("refs/for/").is_err());
        assert!(parse_for_ref("refs/for/main%topic=x")
            .unwrap_err()
            .to_string()
            .contains("topic=x"));
    }
}
