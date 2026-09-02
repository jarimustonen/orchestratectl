use std::process::ExitCode;

fn main() -> ExitCode {
    orchestratectl::dispatch(orchestratectl::InvocationIdentity::ORCHESTRATECTL)
}
