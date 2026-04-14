use voicebot_loadtest::config::LoadtestConfig;
use voicebot_loadtest::run_campaign;

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

    match run_campaign(&config).await {
        Ok(summary) => {
            println!("campaign complete: {}", summary.artifact_dir);
            println!(
                "calls: {} total / {} successful / {} failed",
                summary.total_calls, summary.successful, summary.failed
            );
            println!("duration: {:.1}s", summary.duration_ms as f64 / 1000.0);
            match summary.p50_first_response_ms {
                Some(p50) => println!(
                    "first_response p50/p90/p99: {}/{}/{} ms",
                    p50,
                    summary
                        .p90_first_response_ms
                        .map_or("—".into(), |v| v.to_string()),
                    summary
                        .p99_first_response_ms
                        .map_or("—".into(), |v| v.to_string()),
                ),
                None => println!("first_response: no responses detected"),
            }
            println!("avg_longest_gap_ms: {}", summary.avg_longest_gap_ms);
            println!("total_stutter_count: {}", summary.total_stutter_count);
            println!(
                "reports: {}/report.md and {}/report.html",
                summary.artifact_dir, summary.artifact_dir
            );
        }
        Err(error) => {
            eprintln!("FATAL: campaign failed: {error}");
            std::process::exit(1);
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true))
        .init();
}
