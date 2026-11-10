fn main() {
    if let Err(e) = agent_eye::cli::run() {
        eprintln!("eye: {e:#}");
        std::process::exit(1);
    }
}
