//! foorm: a full-screen ASCII visualizer fed by OSC. nooise is the primary
//! producer (`nooise --osc 127.0.0.1:9000`); foorm listens, gives each event a
//! shape, and keeps its settings one keypress away.

use std::error::Error;
use std::net::SocketAddr;

use clap::Parser;

mod app;
mod gesture;
mod osc;
mod scene;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// UDP address to receive OSC on. nooise sends here with `--osc ADDR`.
    #[arg(long, default_value = "127.0.0.1:9000", value_name = "ADDR")]
    listen: SocketAddr,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let events = osc::listen(cli.listen)?;
    app::run(cli.listen, events)
}
