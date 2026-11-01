fn main() {
    if let Some("--version" | "-V") = std::env::args().nth(1).as_deref() {
        println!("streams-tools {}", env!("CARGO_PKG_VERSION"));
    }
}
