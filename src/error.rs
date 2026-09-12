use std::fmt;

#[derive(Debug)]
pub enum Error {
    Message(String),
    Command {
        cmd: String,
        args: Vec<String>,
        stderr: String,
        status: Option<i32>,
    },
}

impl Error {
    pub fn msg(text: impl Into<String>) -> Self {
        Error::Message(text.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Message(m) => f.write_str(m),
            Error::Command {
                cmd,
                args,
                stderr,
                status,
            } => {
                let status = status.map_or("signal".to_string(), |s| s.to_string());
                write!(f, "{cmd} {} failed (exit {status})", args.join(" "))?;
                if !stderr.trim().is_empty() {
                    write!(f, "\n{}", stderr.trim())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Message(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Message(format!("invalid JSON from gh: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
