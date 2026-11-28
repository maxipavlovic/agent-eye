fn main() {
    if let Err(e) = agent_eye::cli::run() {
        eprintln!("eye: {}", agent_eye::db::explain(&e));
        std::process::exit(1);
    }
}
