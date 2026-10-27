use cli_command_kit::Mode;
use einstar_vega::CancellationToken;
use einstar_vega_cli::{cli, output};
use std::{io, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    let token = CancellationToken::new();
    let listener = match einstar_vega_cli::signals::Listener::new() {
        Ok(listener) => listener,
        Err(error) => {
            let _ = output::report_error(&mut io::stderr(), Mode::from_env(), &error);
            return ExitCode::FAILURE;
        }
    };
    let signal = tokio::spawn(listener.wait(token.clone()));
    let result = cli::execute(
        std::env::args_os(),
        Mode::from_env(),
        &token,
        &mut io::stdout(),
    )
    .await;
    token.cancel();
    let signal_result = signal
        .await
        .map_err(einstar_vega_cli::Error::from)
        .and_then(|result| result.map_err(einstar_vega_cli::Error::from));
    let result = match (result, signal_result) {
        (Err(cause), Err(shutdown)) => Err(einstar_vega_cli::Error::Shutdown {
            cause: Box::new(cause),
            shutdown: Box::new(shutdown),
        }),
        (Err(error), _) | (_, Err(error)) => Err(error),
        _ => Ok(()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let status = cli::exit_status(&error);
            let _ = output::report_error(&mut io::stderr(), Mode::from_env(), &error);
            ExitCode::from(status)
        }
    }
}
