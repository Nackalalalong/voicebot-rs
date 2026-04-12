use voicebot_loadtest::config::LoadtestConfig;
use voicebot_loadtest::run_phase1;

#[tokio::main]
async fn main() {
    init_tracing();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "loadtest.phase1.toml".into());

    let config = match LoadtestConfig::load_from_path(&config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("FATAL: configuration error: {error}");
            std::process::exit(1);
        }
    };

    match run_phase1(&config).await {
        Ok(summary) => {
            println!("phase 1 loadtest complete");
            println!("summary: {}", summary.artifact_dir);
            println!("outcome: {}", summary.outcome);
            match summary.analysis.first_response_ms {
                Some(first_response_ms) => {
                    println!("first_response_ms: {}", first_response_ms);
                }
                None => {
                    println!("first_response_ms: none");
                }
            }
            println!("longest_gap_ms: {}", summary.analysis.longest_gap_ms);
        }
        Err(error) => {
            eprintln!("FATAL: phase 1 loadtest failed: {error}");
            std::process::exit(1);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();
}