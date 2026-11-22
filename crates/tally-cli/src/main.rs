//! `tally` – консольный клиент Tally.

use std::process::ExitCode;

fn main() -> ExitCode {
    let code = tally_cli::run(
        std::env::args_os(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    ExitCode::from(code)
}
