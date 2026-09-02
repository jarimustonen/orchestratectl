use std::process::ExitCode;

fn main() -> ExitCode {
    taskfleet::dispatch(taskfleet::InvocationIdentity::TASKFLEET)
}
