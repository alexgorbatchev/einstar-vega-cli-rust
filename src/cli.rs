use crate::{Error, Result, output, workflow};
use clap::builder::TypedValueParser;
use clap::{Arg, ArgAction, ArgGroup, Command, value_parser};
use cli_command_kit::{HelpOptions, Mode};
use einstar_vega::{CancellationToken, Client};
use std::{ffi::OsString, io::Write, path::PathBuf};

pub const SKILL_POINTER: &str = "Install the vega-cli skill using the Skills CLI:\n\n  npx skills add alexgorbatchev/einstar-vega-cli --skill vega-cli\n\nSkill:\n\n  https://github.com/alexgorbatchev/einstar-vega-cli/tree/main/skills/vega-cli\n";

pub fn help_options() -> HelpOptions {
    HelpOptions {
        skill: Some(SKILL_POINTER.into()),
        ..HelpOptions::default()
    }
}

pub fn command(mode: Mode) -> Command {
    let root = Command::new("vega-cli").about("Browse and download Einstar Vega scanner projects")
        .long_about("Browse scanner files interactively, inspect device information, or download projects without device registration.")
        .arg(Arg::new("version").long("version").short('V').action(ArgAction::SetTrue).help("Print the version"))
        .arg(Arg::new("ip").long("ip").global(true).env("VEGA_IP").default_value("").help("Device IP address or host name"))
        .arg(Arg::new("port").long("port").short('p').global(true).env("VEGA_PORT").default_value("8080").value_name("PORT:1-65535").help("Device or mock listener port"))
        .arg(Arg::new("output").long("output").short('o').global(true).env("VEGA_OUTPUT").default_value("projects").value_parser(value_parser!(PathBuf)).help("Directory for downloaded projects"))
        .subcommand(Command::new("device").about("Inspect the scanner").subcommand_required(true).subcommand(Command::new("inspect").about("Show hardware and firmware information")))
        .subcommand(Command::new("project").about("Browse and download projects").subcommand_required(true)
            .subcommand(Command::new("list").about("List project names, paths, and dates"))
            .subcommand(Command::new("download").about("Download one project or every project")
                .arg(Arg::new("name").value_name("NAME").help("Exact project name"))
                .arg(Arg::new("all").long("all").action(ArgAction::SetTrue).help("Download all projects"))
                .group(ArgGroup::new("selection").args(["name", "all"]).required(true))))
        .subcommand(Command::new("mock").about("Run an offline scanner simulation").subcommand_required(true).subcommand(Command::new("serve").about("Serve synthetic scanner projects on all interfaces")))
        .subcommand(Command::new("completion").about("Print shell completion setup")
            .arg(Arg::new("shell").value_name("SHELL").required(true).value_parser(value_parser!(clap_complete::Shell)).help("Shell: bash, elvish, fish, powershell, or zsh")));
    cli_command_kit::setup(root, mode, &help_options())
        .expect("fixed command tree has no skill collision")
}

pub fn exit_status(error: &Error) -> u8 {
    match error {
        Error::Arguments(_) | Error::Invalid(_) => 64,
        Error::Cancelled | Error::Vega(einstar_vega::Error::Cancelled) => 130,
        Error::Partial { .. } => 6,
        Error::Download { summary, cause } => {
            if matches!(cause.as_ref(), Error::Cancelled) {
                130
            } else if summary.published > 0 {
                6
            } else {
                exit_status(cause)
            }
        }
        Error::Restoration { cause, .. } => exit_status(cause),
        Error::Shutdown { cause, shutdown } => {
            if exit_status(cause) == 6 || exit_status(shutdown) == 6 {
                6
            } else {
                exit_status(cause)
            }
        }
        _ => 1,
    }
}

pub async fn execute<I, S>(
    args: I,
    mode: Mode,
    cancel: &CancellationToken,
    out: &mut impl Write,
) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString> + Clone,
{
    let mut root = command(mode);
    let matches = match root.try_get_matches_from_mut(args) {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            out.write_all(error.to_string().as_bytes())?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if matches.get_flag("version") {
        writeln!(out, "{}", env!("CARGO_PKG_VERSION"))?;
        return Ok(());
    }
    if cli_command_kit::write_skill(&matches, &help_options(), out)? {
        return Ok(());
    }
    if let Some(("completion", child)) = matches.subcommand() {
        let shell = *child
            .get_one::<clap_complete::Shell>("shell")
            .expect("required typed shell");
        // clap_complete writes to an infallible fmt boundary; generate into
        // memory so the actual output write remains fallible and observable.
        let mut bytes = Vec::new();
        clap_complete::generate(shell, &mut root, "vega-cli", &mut bytes);
        out.write_all(&bytes)?;
        return Ok(());
    }
    // Validate after Clap resolves global flags over environment defaults.
    // Typed global env parsers otherwise reject an overridden env value at
    // an ancestor command (clap-rs/clap#5588).
    let port = value_parser!(u16).range(1..).parse_ref(
        &root,
        root.get_arguments().find(|arg| arg.get_id() == "port"),
        matches
            .get_one::<String>("port")
            .expect("default port")
            .as_ref(),
    )?;
    if matches.subcommand_name() == Some("mock") {
        let listener =
            tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
        writeln!(out, "mock:0.0.0.0:{}", listener.local_addr()?.port())?;
        out.flush()?;
        crate::mock::serve(listener, cancel.clone()).await?;
        return Ok(());
    }
    let ip = matches.get_one::<String>("ip").expect("default ip");
    if ip.trim().is_empty() {
        return Err(Error::Invalid("--ip is required (or set VEGA_IP)".into()));
    }
    let address = if ip.contains(':') && !ip.starts_with('[') {
        format!("[{ip}]:{port}")
    } else {
        format!("{ip}:{port}")
    };
    let client = Client::new(&address).map_err(|error| match error {
        einstar_vega::Error::InvalidPath(_) => {
            Error::Invalid("invalid device host or IP address".into())
        }
        error => error.into(),
    })?;
    let output = matches
        .get_one::<PathBuf>("output")
        .expect("default output");
    match matches.subcommand() {
        None => crate::ui::run(client, output.clone(), cancel.clone()).await,
        Some(("device", _)) => {
            client.connect(cancel).await?;
            output::device(out, mode, &client.device_info(cancel).await?)?;
            Ok(())
        }
        Some(("project", child)) if child.subcommand_name() == Some("list") => {
            let projects = client.projects(cancel).await?;
            output::table(
                out,
                mode,
                &["NAME", "PATH", "DATE TIME"],
                projects
                    .into_iter()
                    .map(|(name, info)| vec![name, info.path.to_string(), info.date_time])
                    .collect(),
            )?;
            Ok(())
        }
        Some(("project", child)) => {
            let child = child
                .subcommand_matches("download")
                .expect("validated project action");
            let jobs = workflow::jobs(
                &client,
                child.get_one::<String>("name").map(String::as_str),
                cancel,
            )
            .await?;
            let summary = workflow::download(&client, output, &jobs, cancel, |event| match event {
                workflow::DownloadEvent::Progress { path, progress } => {
                    let path = output::safe(&cli_command_kit::display_path(&path));
                    if mode == Mode::Agent {
                        writeln!(
                            out,
                            "progress:{path}:{}/{}",
                            progress.downloaded, progress.total
                        )
                    } else {
                        writeln!(
                            out,
                            "[INFO] {path}: {} / {}",
                            output::size(Some(progress.downloaded)),
                            output::size(Some(progress.total))
                        )
                    }
                }
                workflow::DownloadEvent::Published(path) => {
                    let path = output::safe(&cli_command_kit::display_path(&path));
                    if mode == Mode::Agent {
                        writeln!(out, "published:{path}")
                    } else {
                        writeln!(out, "[OK] Downloaded {path}")
                    }
                }
                workflow::DownloadEvent::Failed { path, error } => {
                    let path = output::safe(&cli_command_kit::display_path(&path));
                    let error = output::safe(&error);
                    if mode == Mode::Agent {
                        writeln!(out, "failed:{path}:{error}")
                    } else {
                        writeln!(out, "[WARN] Could not finish {path}: {error}")
                    }
                }
            })
            .await?;
            let write_result = if mode == Mode::Agent {
                writeln!(out, "{summary}")
            } else {
                writeln!(
                    out,
                    "[INFO] Downloaded {} files; {} published; {} failed.",
                    summary.completed,
                    summary.published,
                    summary.failures.len()
                )
            };
            if let Err(error) = write_result {
                return Err(Error::Download {
                    summary,
                    cause: Box::new(error.into()),
                });
            }
            if !summary.failures.is_empty() {
                return Err(Error::Partial {
                    completed: summary.completed,
                    failures: summary.failures.join("; "),
                });
            }
            Ok(())
        }
        _ => unreachable!("clap validated the command tree"),
    }
}
