use crate::{Error, Result};
use std::path::{Component, PathBuf};

/// An absolute, normalized POSIX path supplied by the scanner.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RemotePath(String);

impl RemotePath {
    /// Rejects relative paths, dot segments, repeated separators, NULs and
    /// backslashes. A single trailing slash is normalized away.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let mut value = value.into();
        if value.contains("//") {
            return Err(Error::InvalidPath(value));
        }
        if value.len() > 1 && value.ends_with('/') {
            value.pop();
        }
        if !value.starts_with('/')
            || value.contains(['\\', '\0'])
            || value
                .split('/')
                .skip(1)
                .any(|p| p.is_empty() || matches!(p, "." | ".."))
                && value != "/"
        {
            return Err(Error::InvalidPath(value));
        }
        Ok(Self(value))
    }

    /// Returns the exact normalized remote path for a device request.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns a nonempty local relative path confined to `root`.
    pub fn relative_to(&self, root: &Self) -> Result<PathBuf> {
        let prefix = if root.0 == "/" {
            "/".to_owned()
        } else {
            format!("{}/", root.0)
        };
        let relative = self
            .0
            .strip_prefix(&prefix)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                Error::InvalidPath(format!("{} is not a file below {}", self.0, root.0))
            })?;
        let path = PathBuf::from(relative);
        if path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err(Error::InvalidPath(relative.into()));
        }
        Ok(path)
    }
}

impl std::fmt::Display for RemotePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
