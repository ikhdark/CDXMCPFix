use cdxcore::{run_cli, Cli};
use clap::Parser;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = match run_cli(cli).await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("cdxcore: {err:#}");
            4
        }
    };
    std::process::exit(code);
}
