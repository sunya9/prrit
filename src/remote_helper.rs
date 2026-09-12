//! The git remote-helper protocol (gitremote-helpers(7)), push capability only.

use std::io::{BufRead, Write};

use crate::error::Result;

pub trait HelperIo {
    fn read_line(&mut self) -> Option<String>;
    fn write(&mut self, text: &str);
}

pub fn serve(io: &mut dyn HelperIo, mut push: impl FnMut(&str, &str) -> Result<()>) {
    loop {
        let Some(line) = io.read_line() else { return };
        if line.is_empty() {
            return;
        }
        if line == "capabilities" {
            io.write("push\n\n");
        } else if line.starts_with("list") {
            io.write("\n");
        } else if let Some(first) = line.strip_prefix("push ") {
            let mut batch = vec![first.to_string()];
            while let Some(next) = io.read_line() {
                if next.is_empty() {
                    break;
                }
                if let Some(spec) = next.strip_prefix("push ") {
                    batch.push(spec.to_string());
                }
            }
            for spec in &batch {
                let reply = handle_push(spec, &mut push);
                io.write(&reply);
            }
            io.write("\n");
        }
    }
}

fn handle_push(spec: &str, push: &mut impl FnMut(&str, &str) -> Result<()>) -> String {
    let (src, dst) = spec.split_once(':').unwrap_or((spec, ""));
    let src = src.strip_prefix('+').unwrap_or(src);
    match push(src, dst) {
        Ok(()) => format!("ok {dst}\n"),
        Err(e) => {
            let reason = e.to_string();
            let first = reason.lines().next().unwrap_or("failed");
            format!("error {dst} {first}\n")
        }
    }
}

/// Talks to git over the process' stdin/stdout. git waits for each reply
/// before sending the next command, so lines are read one at a time.
pub struct StdIo {
    stdin: std::io::StdinLock<'static>,
    stdout: std::io::Stdout,
}

impl Default for StdIo {
    fn default() -> Self {
        StdIo {
            stdin: std::io::stdin().lock(),
            stdout: std::io::stdout(),
        }
    }
}

impl StdIo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HelperIo for StdIo {
    fn read_line(&mut self) -> Option<String> {
        let mut buf = String::new();
        match self.stdin.read_line(&mut buf) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(buf.trim_end_matches(['\n', '\r']).to_string()),
        }
    }

    fn write(&mut self, text: &str) {
        let _ = self.stdout.write_all(text.as_bytes());
        let _ = self.stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    struct Fake {
        input: std::collections::VecDeque<String>,
        out: String,
    }

    impl HelperIo for Fake {
        fn read_line(&mut self) -> Option<String> {
            self.input.pop_front()
        }
        fn write(&mut self, text: &str) {
            self.out.push_str(text);
        }
    }

    fn session(lines: &[&str], push: impl FnMut(&str, &str) -> Result<()>) -> String {
        let mut io = Fake {
            input: lines.iter().map(|s| s.to_string()).collect(),
            out: String::new(),
        };
        serve(&mut io, push);
        io.out
    }

    #[test]
    fn advertises_push_only() {
        assert_eq!(session(&["capabilities"], |_, _| Ok(())), "push\n\n");
    }

    #[test]
    fn empty_list_for_push() {
        assert_eq!(session(&["list for-push"], |_, _| Ok(())), "\n");
    }

    #[test]
    fn dispatches_each_push_and_acks() {
        let mut seen = Vec::new();
        let out = session(
            &[
                "push refs/heads/feature:refs/for/main",
                "push +refs/heads/x:refs/for/dev%wip",
                "",
            ],
            |src, dst| {
                seen.push((src.to_string(), dst.to_string()));
                Ok(())
            },
        );
        assert_eq!(
            seen,
            vec![
                (
                    "refs/heads/feature".to_string(),
                    "refs/for/main".to_string()
                ),
                ("refs/heads/x".to_string(), "refs/for/dev%wip".to_string()),
            ]
        );
        assert_eq!(out, "ok refs/for/main\nok refs/for/dev%wip\n\n");
    }

    #[test]
    fn reports_errors_per_ref() {
        let out = session(&["push refs/heads/feature:refs/heads/main", ""], |_, _| {
            Err(Error::msg("push to refs/for/<base>\nmore detail"))
        });
        assert_eq!(out, "error refs/heads/main push to refs/for/<base>\n\n");
    }

    #[test]
    fn stops_at_blank_line_and_ignores_unknown() {
        assert_eq!(session(&["", "capabilities"], |_, _| Ok(())), "");
        assert_eq!(session(&["option verbosity 1"], |_, _| Ok(())), "");
    }
}
