use crate::{Error, Result};
use einstar_vega::{CancellationToken, Client, DownloadSpec, OutputDirectory, Progress};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum DownloadEvent {
    Progress { path: PathBuf, progress: Progress },
    Published(PathBuf),
    Failed { path: PathBuf, error: String },
}

#[derive(Debug, Default)]
pub struct Summary {
    pub completed: usize,
    pub published: usize,
    pub failures: Vec<String>,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "downloaded:{} published:{} failed:{}",
            self.completed,
            self.published,
            self.failures.len()
        )
    }
}

fn interrupted(summary: Summary, cause: Error) -> Error {
    Error::Download {
        summary,
        cause: Box::new(cause),
    }
}

/// Resolves all remote identities before touching the output directory.
pub async fn jobs(
    client: &Client,
    name: Option<&str>,
    token: &CancellationToken,
) -> Result<Vec<DownloadSpec>> {
    client.connect(token).await?;
    let mut projects = client.projects(token).await?;
    if let Some(name) = name {
        let project = projects.remove(name).ok_or_else(|| Error::MissingProject {
            name: name.into(),
            available: projects.keys().cloned().collect::<Vec<_>>().join(", "),
        })?;
        projects.clear();
        projects.insert(name.to_owned(), project);
    }
    let mut jobs = Vec::new();
    for (name, project) in projects {
        for path in client
            .list_files(&project.path, token)
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>()
        {
            jobs.push(DownloadSpec::new(&name, project.path.clone(), path)?);
        }
    }
    Ok(jobs)
}

/// Each file is independently atomically published. A failure never becomes a
/// successful all-files result; cancellation awaits staging cleanup.
pub async fn download(
    client: &Client,
    output: &Path,
    jobs: &[DownloadSpec],
    token: &CancellationToken,
    mut event: impl FnMut(DownloadEvent) -> std::io::Result<()>,
) -> Result<Summary> {
    let directory = OutputDirectory::open(output)?;
    let token = token.child_token();
    let mut summary = Summary::default();
    for job in jobs {
        if token.is_cancelled() {
            return Err(interrupted(summary, Error::Cancelled));
        }
        let path = output.join(job.relative_path());
        let mut write_error = None;
        let result = directory
            .download(client, job, &token, |progress| {
                if write_error.is_none()
                    && let Err(error) = event(DownloadEvent::Progress {
                        path: path.clone(),
                        progress,
                    })
                {
                    write_error = Some(error);
                    token.cancel();
                }
            })
            .await;
        if let Some(error) = write_error {
            let cause = match result {
                Err(einstar_vega::Error::Cancelled) => error.into(),
                Err(cleanup) => {
                    if matches!(cleanup, einstar_vega::Error::Published { .. }) {
                        summary.published += 1;
                    }
                    summary
                        .failures
                        .push(format!("{}: {cleanup}", path.display()));
                    Error::Shutdown {
                        cause: Box::new(error.into()),
                        shutdown: Box::new(cleanup.into()),
                    }
                }
                Ok(_) => {
                    summary.completed += 1;
                    summary.published += 1;
                    error.into()
                }
            };
            return Err(interrupted(summary, cause));
        }
        match result {
            Ok(_) => {
                summary.completed += 1;
                summary.published += 1;
                if let Err(error) = event(DownloadEvent::Published(path)) {
                    return Err(interrupted(summary, error.into()));
                }
            }
            Err(einstar_vega::Error::Cancelled) => {
                return Err(interrupted(summary, Error::Cancelled));
            }
            Err(error) => {
                if matches!(error, einstar_vega::Error::Published { .. }) {
                    summary.published += 1;
                }
                let message = format!("{}: {error}", path.display());
                summary.failures.push(message);
                if let Err(error) = event(DownloadEvent::Failed {
                    path,
                    error: error.to_string(),
                }) {
                    return Err(interrupted(summary, error.into()));
                }
            }
        }
    }
    Ok(summary)
}
