use std::path::Path;

use crate::error::{Error, Result};
use crate::gh::Gh;
use crate::git::Git;
use crate::runner::Runner;

pub trait Sink {
    fn out(&self, line: &str);
    fn err(&self, line: &str);
}

/// Writes to the terminal; `out_to_stderr` keeps stdout free for the git
/// remote-helper protocol.
pub struct StdSink {
    pub out_to_stderr: bool,
}

impl Sink for StdSink {
    fn out(&self, line: &str) {
        if self.out_to_stderr {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    fn err(&self, line: &str) {
        eprintln!("{line}");
    }
}

pub struct Context<'a> {
    pub git: Git<'a>,
    pub gh: Gh<'a>,
    pub sink: &'a dyn Sink,
}

impl<'a> Context<'a> {
    pub fn new(runner: &'a dyn Runner, cwd: &Path, sink: &'a dyn Sink) -> Self {
        Context {
            git: Git::new(runner, cwd),
            gh: Gh::new(runner, cwd),
            sink,
        }
    }

    pub fn out(&self, line: impl AsRef<str>) {
        self.sink.out(line.as_ref());
    }

    pub fn err(&self, line: impl AsRef<str>) {
        self.sink.err(line.as_ref());
    }
}

const LOGIN_KEY: &str = "prrit.user";

pub fn resolve_login(ctx: &Context<'_>) -> Result<String> {
    if let Some(cached) = ctx.git.get_config(LOGIN_KEY) {
        return Ok(cached);
    }
    let login = ctx.gh.login()?;
    if login.is_empty() {
        return Err(Error::msg(
            "could not determine GitHub login (is gh authenticated?)",
        ));
    }
    ctx.git.set_config(LOGIN_KEY, &login)?;
    Ok(login)
}

pub fn resolve_base(ctx: &Context<'_>, remote: &str, base: Option<&str>) -> Result<String> {
    if let Some(b) = base {
        return Ok(b.to_string());
    }
    if let Some(b) = ctx.git.default_branch(remote) {
        return Ok(b);
    }
    let from_gh = ctx.gh.default_branch()?;
    if from_gh.is_empty() {
        return Err(Error::msg(format!(
            "could not determine default branch; pass it explicitly: prrit status {remote} <base>"
        )));
    }
    Ok(from_gh)
}

#[cfg(test)]
pub mod recorder {
    use super::Sink;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct Recorder {
        pub out: RefCell<Vec<String>>,
        pub err: RefCell<Vec<String>>,
    }

    impl Recorder {
        pub fn out_text(&self) -> String {
            self.out.borrow().join("\n")
        }
        pub fn err_text(&self) -> String {
            self.err.borrow().join("\n")
        }
    }

    impl Sink for Recorder {
        fn out(&self, line: &str) {
            self.out.borrow_mut().push(line.to_string());
        }
        fn err(&self, line: &str) {
            self.err.borrow_mut().push(line.to_string());
        }
    }
}
