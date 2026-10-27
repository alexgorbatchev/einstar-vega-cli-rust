use super::{Action, Model, Update, render};
use crate::{Error, Result, workflow};
use crossterm::event::{self, Event};
use einstar_vega::{CancellationToken, Client, NodeKind, ProjectTree};
use ratatui::{Terminal, backend::Backend};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

struct Restore;
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = ratatui::try_restore();
    }
}

/// Runs the real terminal browser and restores terminal state after all workers
/// have stopped. Noninteractive invocations fail before changing terminal modes.
pub async fn run(client: Client, output: PathBuf, token: CancellationToken) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(Error::Invalid(
            "interactive browsing requires a terminal; use project list or project download".into(),
        ));
    }
    let restore = Restore;
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => return restore_result(Err(error.into()), restore),
    };
    let result = run_loop(&mut terminal, client, output, &token, || {
        if event::poll(Duration::ZERO)? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    })
    .await;
    restore_result(result, restore)
}

fn restore_result(result: Result<()>, restore: Restore) -> Result<()> {
    let restored = ratatui::try_restore();
    // A failed explicit restore is retried by Drop; a successful restore needs
    // no second operation that could obscure its already-observed outcome.
    if restored.is_ok() {
        std::mem::forget(restore);
    }
    match (result, restored) {
        (result, Ok(())) => result,
        (Ok(()), Err(error)) => Err(error.into()),
        (Err(cause), Err(restore)) => Err(Error::Restoration {
            cause: Box::new(cause),
            restore,
        }),
    }
}

async fn send(tx: &mpsc::Sender<Update>, token: &CancellationToken, update: Update) -> bool {
    tokio::select! { biased; _ = token.cancelled() => false, result = tx.send(update) => result.is_ok() }
}

async fn load(client: Client, token: CancellationToken, tx: mpsc::Sender<Update>) {
    let result = tokio::time::timeout(Duration::from_secs(10), client.connect(&token)).await;
    match result {
        Ok(Ok(())) => {
            if !send(
                &tx,
                &token,
                Update::Log("INFO: Connected successfully".into()),
            )
            .await
            {
                return;
            }
        }
        Ok(Err(error)) => {
            send(
                &tx,
                &token,
                Update::Log(format!("ERROR: Could not connect: {error}")),
            )
            .await;
            return;
        }
        Err(error) => {
            send(
                &tx,
                &token,
                Update::Log(format!("ERROR: Could not connect: {error}")),
            )
            .await;
            return;
        }
    }
    match client.device_info(&token).await {
        Ok(info) => {
            if !send(&tx, &token, Update::Info(info)).await {
                return;
            }
        }
        Err(error) => {
            if !send(
                &tx,
                &token,
                Update::Log(format!("ERROR: fetching device info: {error}")),
            )
            .await
            {
                return;
            }
        }
    }
    let projects = match client.projects(&token).await {
        Ok(projects) => projects,
        Err(error) => {
            send(
                &tx,
                &token,
                Update::Log(format!("ERROR: fetching projects: {error}")),
            )
            .await;
            return;
        }
    };
    for (name, project) in projects {
        if name == "Logs" {
            continue;
        }
        let paths = match client.list_files(&project.path, &token).await {
            Ok(paths) => paths,
            Err(error) => {
                if !send(
                    &tx,
                    &token,
                    Update::Log(format!("ERROR: listing {name}: {error}")),
                )
                .await
                {
                    return;
                }
                continue;
            }
        };
        let tree = match ProjectTree::new(&name, project, paths) {
            Ok(tree) => tree,
            Err(error) => {
                if !send(
                    &tx,
                    &token,
                    Update::Log(format!("ERROR: building {name}: {error}")),
                )
                .await
                {
                    return;
                }
                continue;
            }
        };
        let files = tree
            .nodes()
            .iter()
            .filter(|node| node.kind == NodeKind::File)
            .map(|node| (node.id, node.path.clone()))
            .collect::<Vec<_>>();
        if !send(&tx, &token, Update::Tree(tree)).await {
            return;
        }
        for (node, path) in files {
            let update = match client.file_size(&path, &token).await {
                Ok(size) => Update::Size {
                    project: name.clone(),
                    node,
                    size,
                },
                Err(error) => Update::Log(format!("ERROR: getting size of {path}: {error}")),
            };
            if !send(&tx, &token, update).await {
                return;
            }
            tokio::select! { biased; _ = token.cancelled() => return, _ = tokio::time::sleep(Duration::from_millis(20)) => {} }
        }
    }
}

struct Workers {
    loader: Option<JoinHandle<()>>,
    download: Option<JoinHandle<Result<workflow::Summary>>>,
}

impl Workers {
    async fn reap(&mut self, model: &mut Model) -> Result<()> {
        if self.loader.as_ref().is_some_and(JoinHandle::is_finished) {
            self.loader.take().expect("present loader").await?;
        }
        if self.download.as_ref().is_some_and(JoinHandle::is_finished) {
            let result = self.download.take().expect("present download").await?;
            finish_download(model, result);
        }
        Ok(())
    }

    async fn shutdown(mut self) -> Result<()> {
        // Observe both joins even when the first worker panicked.
        let loader = if let Some(loader) = self.loader.take() {
            loader.await.map_err(Error::from)
        } else {
            Ok(())
        };
        let download = if let Some(download) = self.download.take() {
            match download.await {
                Err(error) => Err(error.into()),
                Ok(Ok(summary)) if !summary.failures.is_empty() => Err(Error::Partial {
                    completed: summary.completed,
                    failures: summary.failures.join("; "),
                }),
                Ok(Err(Error::Download { summary, cause }))
                    if matches!(cause.as_ref(), Error::Cancelled)
                        && summary.failures.is_empty() =>
                {
                    Ok(())
                }
                Ok(Err(Error::Cancelled)) | Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(error),
            }
        } else {
            Ok(())
        };
        match (loader, download) {
            (Err(cause), Err(shutdown)) => Err(Error::Shutdown {
                cause: Box::new(cause),
                shutdown: Box::new(shutdown),
            }),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }
}

fn finish_download(model: &mut Model, result: Result<workflow::Summary>) {
    match result {
        Ok(summary) => {
            for failure in &summary.failures {
                model.apply(Update::Log(format!("ERROR: {failure}")));
            }
            model.apply(Update::DownloadDone {
                completed: summary.completed,
                failures: summary.failures.len(),
            });
        }
        Err(error) => {
            let (completed, failures) = if let Error::Download { summary, .. } = &error {
                (summary.completed, summary.failures.len().max(1))
            } else {
                (0, 1)
            };
            model.apply(Update::Log(format!("ERROR: {error}")));
            model.apply(Update::DownloadDone {
                completed,
                failures,
            });
        }
    }
}

/// The native terminal backend and event source are supplied by the caller,
/// allowing the same owned-worker loop to run with Ratatui's test backend.
pub async fn run_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    client: Client,
    output: PathBuf,
    parent: &CancellationToken,
    mut next_event: impl FnMut() -> io::Result<Option<Event>>,
) -> Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let token = parent.child_token();
    let (tx, mut rx) = mpsc::channel(128);
    let mut workers = Workers {
        loader: Some(tokio::spawn(load(
            client.clone(),
            token.clone(),
            tx.clone(),
        ))),
        download: None,
    };
    let mut model = Model::new();
    let result: Result<()> = async {
        let mut ticker = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {
                _ = token.cancelled() => return Err(Error::Cancelled),
                Some(update) = rx.recv() => model.apply(update),
                _ = ticker.tick() => {
                    workers.reap(&mut model).await?;
                    for _ in 0..64 {
                        let Some(event) = next_event()? else { break };
                        if let Event::Key(key) = event {
                            match model.key(key) {
                                Action::Quit => return Ok(()),
                                Action::None => {},
                                Action::Download(jobs) => {
                                    // The model prevents overlapping work; ownership stays here
                                    // until the old worker's completion has been observed.
                                    if workers.download.is_some() { model.apply(Update::Log("A download worker is still shutting down".into())); continue; }
                                    let client = client.clone(); let output = output.clone(); let token = token.clone(); let tx = tx.clone();
                                    workers.download = Some(tokio::spawn(async move {
                                        workflow::download(&client, &output, &jobs, &token, |event| {
                                            let update = match event {
                                                workflow::DownloadEvent::Progress { path, progress } => Update::Progress { path, progress },
                                                workflow::DownloadEvent::Published(path) => Update::Log(format!("Published {}", cli_command_kit::display_path(&path))),
                                                workflow::DownloadEvent::Failed { path, error } => Update::Log(format!("ERROR: {}: {error}", cli_command_kit::display_path(&path))),
                                            };
                                            // Progress is transient: a full bounded UI queue
                                            // may coalesce it. Results remain in the owned join.
                                            let _ = tx.try_send(update);
                                            Ok(())
                                        }).await
                                    }));
                                }
                            }
                        }
                    }
                    terminal.draw(|frame| render(&mut model, frame)).map_err(io::Error::other)?;
                }
            }
        }
    }.await;
    token.cancel();
    match (result, workers.shutdown().await) {
        (Err(cause), Err(shutdown)) => Err(Error::Shutdown {
            cause: Box::new(cause),
            shutdown: Box::new(shutdown),
        }),
        (Err(error), _) | (_, Err(error)) => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "../../tests/unit/controller.rs"]
mod tests;
