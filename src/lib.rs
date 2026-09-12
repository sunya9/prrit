pub mod change_id;
pub mod commands;
pub mod context;
pub mod error;
pub mod format;
pub mod gh;
pub mod git;
pub mod hook;
pub mod plan;
pub mod refs_for;
pub mod remote_helper;
pub mod runner;
#[cfg(test)]
pub mod testutil;

use std::path::Path;

use commands::init::{init, InitOptions, DEFAULT_REVIEW_REMOTE};
use commands::remote_helper::remote_helper;
use commands::status::{status, StatusOptions};
use context::{Context, StdSink};
use runner::{ExecRunner, InheritTarget};

const USAGE: &str = "prrit — one commit, one PR, stacked on GitHub

Usage:
  prrit init [remote] [base]      Install the Change-Id hook and the \"review\" remote (default: origin <default branch>)
  prrit status [remote] [base]    Show the PR for each commit on top of <remote>/<base>

Then push like Gerrit:
  git push review                       upload HEAD to refs/for/<base>
  git push review HEAD:refs/for/<base>  pick another base; add %wip for draft PRs

Options:
  --review-remote <name>   init only: name of the helper remote (default: review)
  --push-default           init only: also set remote.pushDefault so a bare \"git push\" uploads
  -h, --help               Show this help
";

struct Cli {
    command: Option<String>,
    remote: String,
    base: Option<String>,
    review_remote: String,
    push_default: bool,
    help: bool,
}

fn parse_cli(args: &[String]) -> Result<Cli, String> {
    let mut cli = Cli {
        command: None,
        remote: "origin".into(),
        base: None,
        review_remote: DEFAULT_REVIEW_REMOTE.into(),
        push_default: false,
        help: false,
    };
    let mut positionals = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => cli.help = true,
            "--push-default" => cli.push_default = true,
            "--review-remote" => {
                cli.review_remote = it.next().ok_or("--review-remote needs a value")?.clone();
            }
            s if s.starts_with("--review-remote=") => {
                cli.review_remote = s["--review-remote=".len()..].to_string()
            }
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            _ => positionals.push(arg.clone()),
        }
    }
    let mut pos = positionals.into_iter();
    cli.command = pos.next();
    if let Some(r) = pos.next() {
        cli.remote = r;
    }
    cli.base = pos.next();
    Ok(cli)
}

fn run(args: &[String], ctx: &Context<'_>) -> i32 {
    let cli = match parse_cli(args) {
        Ok(cli) => cli,
        Err(e) => {
            ctx.err(format!("error: {e}\n"));
            ctx.out(USAGE);
            return 1;
        }
    };
    let Some(command) = &cli.command else {
        ctx.out(USAGE);
        return if cli.help { 0 } else { 1 };
    };
    if cli.help {
        ctx.out(USAGE);
        return 0;
    }
    let result = match command.as_str() {
        "init" => init(
            ctx,
            &InitOptions {
                remote: &cli.remote,
                base: cli.base.as_deref(),
                review_remote: &cli.review_remote,
                push_default: cli.push_default,
                path_dirs: None,
            },
        ),
        "status" => status(
            ctx,
            &StatusOptions {
                remote: &cli.remote,
                base: cli.base.as_deref(),
            },
        )
        .map(|_| true),
        other => {
            ctx.err(format!("unknown command: {other}\n"));
            ctx.out(USAGE);
            return 1;
        }
    };
    match result {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            ctx.err(format!("error: {e}"));
            1
        }
    }
}

/// git invokes the helper as `git-remote-prrit <remote> <url>`; the same
/// binary also answers to `prrit remote-helper <remote> <url>` for wrappers.
fn helper_args(argv: &[String]) -> Option<&[String]> {
    let invoked_as = argv.first().map(|a| {
        Path::new(a)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
    })??;
    if invoked_as == "git-remote-prrit" {
        return Some(&argv[1..]);
    }
    if argv.get(1).map(String::as_str) == Some("remote-helper") {
        return Some(&argv[2..]);
    }
    None
}

/// Shared entry point of the `prrit` and `git-remote-prrit` binaries.
pub fn run_main() {
    let argv: Vec<String> = std::env::args().collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let helper = helper_args(&argv).map(|a| a.to_vec());
    let sink = StdSink {
        out_to_stderr: helper.is_some(),
    };
    let runner = ExecRunner {
        inherit_target: if helper.is_some() {
            InheritTarget::Stderr
        } else {
            InheritTarget::Stdout
        },
    };
    let ctx = Context::new(&runner, &cwd, &sink);

    let code = match helper {
        Some(args) => {
            let mut io = remote_helper::StdIo::new();
            let ok = remote_helper(
                &ctx,
                &mut io,
                args.first().map(String::as_str),
                args.get(1).map(String::as_str),
            );
            if ok {
                0
            } else {
                1
            }
        }
        None => run(&argv[1..], &ctx),
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_positionals_and_options() {
        let cli = parse_cli(&strs(&[
            "init",
            "upstream",
            "develop",
            "--push-default",
            "--review-remote",
            "gerrit",
        ]))
        .unwrap();
        assert_eq!(cli.command.as_deref(), Some("init"));
        assert_eq!(cli.remote, "upstream");
        assert_eq!(cli.base.as_deref(), Some("develop"));
        assert!(cli.push_default);
        assert_eq!(cli.review_remote, "gerrit");
        let cli = parse_cli(&strs(&["status"])).unwrap();
        assert_eq!(cli.remote, "origin");
        assert_eq!(cli.base, None);
        assert!(parse_cli(&strs(&["--bogus"])).is_err());
    }

    #[test]
    fn detects_helper_invocation() {
        assert_eq!(
            helper_args(&strs(&["/usr/bin/git-remote-prrit", "review", "origin"])).unwrap(),
            &strs(&["review", "origin"])[..]
        );
        assert_eq!(
            helper_args(&strs(&["prrit", "remote-helper", "review", "origin"])).unwrap(),
            &strs(&["review", "origin"])[..]
        );
        assert!(helper_args(&strs(&["prrit", "status"])).is_none());
    }
}
