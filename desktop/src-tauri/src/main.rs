fn main() {
    if let Err(error) = silent_disco_desktop_lib::run() {
        eprintln!("Silent Disco desktop failed to start: {error}");
        std::process::exit(1);
    }
}
