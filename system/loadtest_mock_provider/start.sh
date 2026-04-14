mkdir -p system/loadtest_mock_provider/artifacts/loadtest
docker compose -f system/loadtest_mock_provider/docker-compose.yaml up -d --build
cd voicebot
cargo build --release -p voicebot-loadtest
cargo run --release -p voicebot-loadtest -- ../system/loadtest_mock_provider/loadtest.toml