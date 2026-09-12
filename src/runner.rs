//! Boundary to external processes (git, gh), swappable in tests.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

#[derive(Default)]
pub struct RunOptions<'a> {
    pub cwd: Option<&'a Path>,
    pub input: Option<&'a str>,
    pub env: Vec<(&'a str, &'a str)>,
    /// Stream the child's output straight to the terminal instead of capturing.
    pub inherit: bool,
}

pub trait Runner {
    fn run(&self, cmd: &str, args: &[&str], opts: &RunOptions<'_>) -> Result<String>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InheritTarget {
    Stdout,
    /// The remote helper owns stdout for the git protocol, so inherited child
    /// output must go to stderr instead.
    Stderr,
}

pub struct ExecRunner {
    pub inherit_target: InheritTarget,
}

impl Runner for ExecRunner {
    fn run(&self, cmd: &str, args: &[&str], opts: &RunOptions<'_>) -> Result<String> {
        let mut command = Command::new(cmd);
        command.args(args);
        if let Some(cwd) = opts.cwd {
            command.current_dir(cwd);
        }
        for (k, v) in &opts.env {
            command.env(k, v);
        }
        let fail = |stderr: String, status: Option<i32>| Error::Command {
            cmd: cmd.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
            stderr,
            status,
        };

        if opts.inherit {
            let stdout = match self.inherit_target {
                InheritTarget::Stdout => Stdio::inherit(),
                InheritTarget::Stderr => Stdio::from(std::io::stderr()),
            };
            let status = command
                .stdin(Stdio::null())
                .stdout(stdout)
                .stderr(Stdio::inherit())
                .status()?;
            return if status.success() {
                Ok(String::new())
            } else {
                Err(fail(String::new(), status.code()))
            };
        }

        command.stdin(if opts.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| Error::msg(format!("failed to run {cmd}: {e}")))?;
        if let Some(input) = opts.input {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(input.as_bytes())?;
            }
        }
        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(fail(
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code(),
            ))
        }
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::cell::RefCell;

    pub struct Call {
        pub cmd: String,
        pub args: Vec<String>,
        pub inherit: bool,
    }

    type Responder = Box<dyn Fn(&str, &[String]) -> Option<String>>;
    type Predicate = Box<dyn Fn(&str, &[String]) -> bool>;

    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: RefCell<Vec<Call>>,
        responders: Vec<Responder>,
        failures: Vec<Predicate>,
    }

    impl FakeRunner {
        pub fn on(mut self, f: impl Fn(&str, &[String]) -> Option<String> + 'static) -> Self {
            self.responders.push(Box::new(f));
            self
        }

        /// Calls matching `f` fail as if the command exited non-zero.
        pub fn fail_when(mut self, f: impl Fn(&str, &[String]) -> bool + 'static) -> Self {
            self.failures.push(Box::new(f));
            self
        }

        pub fn args_of(&self, cmd: &str, sub: &str) -> Vec<Vec<String>> {
            self.calls
                .borrow()
                .iter()
                .filter(|c| c.cmd == cmd && c.args.first().map(String::as_str) == Some(sub))
                .map(|c| c.args.clone())
                .collect()
        }

        pub fn joined(&self, cmd: &str) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .filter(|c| c.cmd == cmd)
                .map(|c| c.args.join(" "))
                .collect()
        }
    }

    impl Runner for FakeRunner {
        fn run(&self, cmd: &str, args: &[&str], opts: &RunOptions<'_>) -> Result<String> {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            self.calls.borrow_mut().push(Call {
                cmd: cmd.to_string(),
                args: args.clone(),
                inherit: opts.inherit,
            });
            if self.failures.iter().any(|f| f(cmd, &args)) {
                return Err(Error::Command {
                    cmd: cmd.into(),
                    args,
                    stderr: "simulated failure".into(),
                    status: Some(1),
                });
            }
            for r in &self.responders {
                if let Some(out) = r(cmd, &args) {
                    return Ok(out);
                }
            }
            panic!("FakeRunner: unexpected call {cmd} {}", args.join(" "));
        }
    }
}
