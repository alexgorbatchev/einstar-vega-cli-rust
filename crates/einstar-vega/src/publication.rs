use crate::{CancellationToken, Client, Error, Progress, RemotePath, Result, check_cancel};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use std::{
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
};
use tokio::io::AsyncWriteExt;

/// A validated project-relative file download.
#[derive(Clone, Debug)]
pub struct DownloadSpec {
    project_name: String,
    project_root: RemotePath,
    path: RemotePath,
    relative: PathBuf,
}

impl DownloadSpec {
    /// Validates the project name and confines the file to the project root.
    pub fn new(
        project_name: impl Into<String>,
        project_root: RemotePath,
        path: RemotePath,
    ) -> Result<Self> {
        let project_name = project_name.into();
        let name = Path::new(&project_name);
        if project_name.is_empty()
            || project_name.contains(['/', '\\', '\0'])
            || name.components().count() != 1
            || !matches!(name.components().next(), Some(Component::Normal(_)))
        {
            return Err(Error::InvalidPath(project_name));
        }
        let relative = PathBuf::from(&project_name).join(path.relative_to(&project_root)?);
        Ok(Self {
            project_name,
            project_root,
            path,
            relative,
        })
    }
    /// The unmodified project name.
    pub fn project_name(&self) -> &str {
        &self.project_name
    }
    /// The project's remote root.
    pub fn project_root(&self) -> &RemotePath {
        &self.project_root
    }
    /// The exact file path sent to the device.
    pub fn path(&self) -> &RemotePath {
        &self.path
    }
    /// The native output path relative to the selected output directory.
    pub fn relative_path(&self) -> &Path {
        &self.relative
    }
}

/// An opened output-directory capability. Descendants are opened without
/// following symlinks; publication uses retained directory handles.
#[derive(Debug)]
pub struct OutputDirectory {
    directory: Dir,
}

impl OutputDirectory {
    /// Creates and opens a user-selected output root, including `.`. The root
    /// itself is trusted user configuration; device paths never choose it.
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        Ok(Self {
            directory: Dir::open_ambient_dir(path, ambient_authority())?,
        })
    }

    /// Downloads to a private sibling staging directory and atomically replaces
    /// the destination only after exact-length verification, flush and sync.
    /// Successful earlier files are independent of subsequent failures.
    /// `Error::Published` means replacement succeeded but staging cleanup failed;
    /// its paths are relative to this output root. Other errors leave the
    /// destination unchanged. Cancel the token and await completion to observe
    /// cleanup failures; dropping the future provides only best-effort cleanup.
    pub async fn download<F>(
        &self,
        client: &Client,
        spec: &DownloadSpec,
        token: &CancellationToken,
        progress: F,
    ) -> Result<PathBuf>
    where
        F: FnMut(Progress),
    {
        check_cancel(token)?;
        let size = client.file_size(&spec.path, token).await?;
        let mut stage = Stage::new(&self.directory, &spec.relative)?;
        let result = client
            .download(
                &spec.path,
                size,
                stage
                    .writer
                    .as_mut()
                    .ok_or_else(|| Error::Protocol("staging writer closed".into()))?,
                token,
                progress,
            )
            .await;
        match result {
            Ok(()) => stage.commit(token).await?,
            Err(error) => return Err(stage.discard(error).await),
        }
        Ok(spec.relative.clone())
    }
}

struct Stage {
    parent: Dir,
    work: Option<Dir>,
    destination: OsString,
    relative: PathBuf,
    work_path: PathBuf,
    writer: Option<tokio::fs::File>,
}

impl Stage {
    fn new(root: &Dir, relative: &Path) -> Result<Self> {
        let destination = relative
            .file_name()
            .ok_or_else(|| Error::InvalidPath(relative.display().to_string()))?
            .to_owned();
        let mut parent = root.try_clone()?;
        for component in relative.parent().into_iter().flat_map(Path::components) {
            let Component::Normal(name) = component else {
                return Err(Error::InvalidPath(relative.display().to_string()));
            };
            parent = match parent.open_dir_nofollow(name) {
                Ok(directory) => directory,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match parent.create_dir(name) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    parent.open_dir_nofollow(name)?
                }
                Err(error) => return Err(error.into()),
            };
        }
        let original = match parent.symlink_metadata(&destination) {
            Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
            Ok(_) => {
                return Err(Error::InvalidPath(
                    "destination is not a regular file".into(),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let temporary = format!(".vega-download-{}", uuid::Uuid::new_v4());
        let mut builder = cap_std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use cap_std::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        parent.create_dir_with(&temporary, &builder)?;
        let work_path = relative.parent().unwrap_or(Path::new("")).join(&temporary);
        let work = match parent.open_dir_nofollow(&temporary) {
            Ok(work) => work,
            Err(error) => {
                let cause = Error::Io(error);
                return Err(match parent.remove_dir(&temporary) {
                    Ok(()) => cause,
                    Err(cleanup) => Error::Cleanup {
                        cause: Box::new(cause),
                        staging: work_path,
                        cleanup,
                    },
                });
            }
        };
        let mut stage = Self {
            parent,
            work: Some(work),
            destination,
            relative: relative.to_owned(),
            work_path,
            writer: None,
        };
        stage.initialize_writer(original)?;
        Ok(stage)
    }

    fn initialize_writer(&mut self, original: Option<cap_std::fs::Permissions>) -> Result<()> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let file = match self
            .work
            .as_ref()
            .ok_or_else(|| Error::Protocol("staging directory closed".into()))?
            .open_with("content", &options)
        {
            Ok(file) => file,
            Err(error) => return Err(self.cleanup_error(error.into())),
        };
        if let Some(permissions) = original
            && let Err(error) = file.set_permissions(permissions)
        {
            let mut cause = Error::Io(error);
            if let Err(close) = close_file(file.into_std()) {
                cause = Error::Close {
                    cause: Box::new(cause),
                    close,
                };
            }
            return Err(self.cleanup_error(cause));
        }
        self.writer = Some(tokio::fs::File::from_std(file.into_std()));
        Ok(())
    }

    async fn commit(mut self, token: &CancellationToken) -> Result<()> {
        let result = self.commit_inner(token).await;
        match result {
            Ok(()) => self.cleanup().map_err(|cleanup| Error::Published {
                path: self.relative.clone(),
                staging: self.work_path.clone(),
                cleanup,
            }),
            Err(error) => Err(self.discard(error).await),
        }
    }

    async fn commit_inner(&mut self, token: &CancellationToken) -> Result<()> {
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| Error::Protocol("staging writer closed".into()))?;
        writer.flush().await?;
        writer.sync_all().await?;
        // Drain Tokio's pending blocking filesystem operations before closing;
        // cancellation never races retained writes against publication.
        if let Some(writer) = self.writer.take() {
            close_file(writer.into_std().await)?;
        }
        check_cancel(token)?;
        match self.parent.symlink_metadata(&self.destination) {
            Ok(metadata) if !metadata.is_file() => {
                return Err(Error::InvalidPath(
                    "destination changed to a non-regular file".into(),
                ));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        self.work
            .as_ref()
            .ok_or_else(|| Error::Protocol("staging directory closed".into()))?
            .rename("content", &self.parent, &self.destination)?;
        Ok(())
    }

    async fn discard(mut self, mut cause: Error) -> Error {
        if let Some(writer) = self.writer.take()
            && let Err(close) = close_file(writer.into_std().await)
        {
            cause = Error::Close {
                cause: Box::new(cause),
                close,
            };
        }
        self.cleanup_error(cause)
    }

    fn cleanup_error(&mut self, cause: Error) -> Error {
        match self.cleanup() {
            Ok(()) => cause,
            Err(cleanup) => Error::Cleanup {
                cause: Box::new(cause),
                staging: self.work_path.clone(),
                cleanup,
            },
        }
    }

    fn cleanup(&mut self) -> io::Result<()> {
        match self.work.take() {
            Some(work) => work.remove_open_dir_all(),
            None => Ok(()),
        }
    }
}

#[cfg(unix)]
fn close_file(file: std::fs::File) -> io::Result<()> {
    use std::os::fd::IntoRawFd;
    let descriptor = file.into_raw_fd();
    // SAFETY: into_raw_fd transfers sole ownership of this valid descriptor.
    // Tokio operations were drained by into_std().await. try_close consumes
    // the descriptor even on error; it must never be retried or dropped again.
    unsafe { rustix::io::try_close(descriptor) }.map_err(io::Error::from)
}

#[cfg(windows)]
fn close_file(file: std::fs::File) -> io::Result<()> {
    use std::os::windows::io::IntoRawHandle;
    let handle = file.into_raw_handle();
    // SAFETY: this is the owned, drained file's native handle. No other owner
    // can close it; ownership was consumed by into_raw_handle.
    if unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Normal errors take the explicit fallible discard path. Dropped caller
        // futures still get best-effort cleanup; the existing file is untouched.
        self.writer.take();
        let _ = self.cleanup();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{Stage, close_file};
    use crate::CancellationToken;
    use std::os::fd::AsRawFd;

    fn temporary_directory() -> tempfile::TempDir {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".tmp");
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    fn assert_descriptor_released(native_link: &str, original: &std::path::Path) {
        // Another thread may reuse the closed descriptor number immediately.
        match std::fs::read_link(native_link) {
            Ok(current) => assert_ne!(current, original, "original file remains open"),
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}"),
        }
    }

    #[tokio::test]
    async fn initialization_failure_reports_cleanup_and_recovery_separately() {
        use std::os::unix::fs::PermissionsExt;
        for deny_access in [false, true] {
            let temporary = temporary_directory();
            let root =
                cap_std::fs::Dir::open_ambient_dir(temporary.path(), cap_std::ambient_authority())
                    .unwrap();
            let mut stage = Stage::new(&root, std::path::Path::new("project/file")).unwrap();
            close_file(stage.writer.take().unwrap().into_std().await).unwrap();
            let recovery = temporary.path().join(&stage.work_path);
            if deny_access {
                std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o000))
                    .unwrap();
            }
            let error = stage.initialize_writer(None).unwrap_err();
            if deny_access {
                std::fs::set_permissions(&recovery, std::fs::Permissions::from_mode(0o700))
                    .unwrap();
                assert!(
                    matches!(error, crate::Error::Cleanup { cause, staging, cleanup }
                    if matches!(*cause, crate::Error::Io(ref error) if error.kind() == std::io::ErrorKind::PermissionDenied)
                    && staging == stage.work_path && cleanup.kind() == std::io::ErrorKind::PermissionDenied)
                );
                assert_eq!(std::fs::read(recovery.join("content")).unwrap(), b"");
            } else {
                assert!(
                    matches!(error, crate::Error::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists)
                );
                assert!(!recovery.exists());
            }
            assert!(!temporary.path().join("project/file").exists());
        }
    }

    #[tokio::test]
    async fn flush_and_sync_errors_retain_writer_until_explicit_discard() {
        use tokio::io::AsyncWriteExt;
        for (device, buffered) in [("/dev/full", true), ("/dev/null", false)] {
            let temporary = temporary_directory();
            let root =
                cap_std::fs::Dir::open_ambient_dir(temporary.path(), cap_std::ambient_authority())
                    .unwrap();
            let mut stage = Stage::new(&root, std::path::Path::new("project/file")).unwrap();
            close_file(stage.writer.take().unwrap().into_std().await).unwrap();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(device)
                .unwrap();
            let descriptor = file.as_raw_fd();
            let native_link = format!("/proc/self/fd/{descriptor}");
            let original = std::fs::read_link(&native_link).unwrap();
            stage.writer = Some(tokio::fs::File::from_std(file));
            if buffered {
                stage
                    .writer
                    .as_mut()
                    .unwrap()
                    .write_all(b"pending bytes")
                    .await
                    .unwrap();
            }
            let error = stage
                .commit_inner(&CancellationToken::new())
                .await
                .unwrap_err();
            assert!(
                stage.writer.is_some(),
                "lost writer after {device} failure: {error}"
            );
            let error = stage.discard(error).await;
            assert!(matches!(error, crate::Error::Io(_)), "{error}");
            assert_descriptor_released(&native_link, &original);
            assert_eq!(
                std::fs::read_dir(temporary.path().join("project"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }

    #[test]
    fn explicit_close_releases_the_owned_native_file_descriptor() {
        let temporary = temporary_directory();
        let path = temporary.path().join("content");
        let file = std::fs::File::create(&path).unwrap();
        let descriptor = file.as_raw_fd();
        let native_link = format!("/proc/self/fd/{descriptor}");
        assert_eq!(std::fs::read_link(&native_link).unwrap(), path);
        close_file(file).unwrap();
        assert_descriptor_released(&native_link, &path);
    }
}
