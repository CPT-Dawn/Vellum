mod tui;

use std::io;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    tui::run_entrypoint().await
}
