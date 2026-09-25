// Prevents a console window on Windows builds.
#![windows_subsystem = "windows"]


fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Invoked by an agent's hook: `agentpet hook --agent <kind>` (reads stdin).
    if args.get(1).map(String::as_str) == Some("hook") {
        agentpet_lib::cli::run_hook(&args[2..]);
        return;
    }
    // `agentpet run [--session id] [--project path] [--agent kind] -- <cmd...>`:
    // wraps any CLI agent , working while it runs, done when it exits.
    if args.get(1).map(String::as_str) == Some("run") {
        agentpet_lib::cli::run_wrapper(&args[2..]);
    }
    agentpet_lib::run();
}
