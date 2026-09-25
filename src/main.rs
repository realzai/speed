mod speedtest;
mod tui;

use std::io::IsTerminal;

use anyhow::Result;

const HELP: &str = "speed 1.0.0 — a playful Fast.com speed test\n\nUSAGE:\n    speed          Open the interactive speed test\n    speed --once   Run once without the interactive UI\n    speed --help   Show this help\n\nKEYS:\n    Space / Enter  Run the test again\n    q / Esc        Quit\n";

#[tokio::main]
async fn main() -> Result<()> {
    let argument = std::env::args().nth(1);

    match argument.as_deref() {
        Some("-h" | "--help") => print!("{HELP}"),
        Some("-V" | "--version") => println!("speed {}", env!("CARGO_PKG_VERSION")),
        Some("--once") => run_once().await?,
        Some(other) => {
            eprintln!("Unknown option: {other}\n\n{HELP}");
            std::process::exit(2);
        }
        None if !std::io::stdout().is_terminal() => run_once().await?,
        None => tui::run().await?,
    }

    Ok(())
}

async fn run_once() -> Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(speedtest::run(sender));

    while let Some(update) = receiver.recv().await {
        match update {
            speedtest::Update::Phase(phase) => eprintln!("{phase}"),
            speedtest::Update::Sample(_) => {}
            speedtest::Update::Finished(result) => {
                println!(
                    "{:.1} Mbps  •  {} ms ping  •  {}  •  {:.1} MB downloaded",
                    result.speed_mbps,
                    result.latency_ms,
                    result.server,
                    result.bytes as f64 / 1_000_000.0
                );
                return Ok(());
            }
            speedtest::Update::Failed(error) => anyhow::bail!(error),
        }
    }

    task.await??;
    Ok(())
}
