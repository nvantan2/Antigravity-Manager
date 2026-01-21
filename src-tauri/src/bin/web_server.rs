#[tokio::main]
async fn main() {
    if let Err(err) = antigravity_tools_lib::web_server::run().await {
        eprintln!("Web server failed: {}", err);
        std::process::exit(1);
    }
}
