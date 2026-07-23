@echo off
cd /d "%~dp0"
cargo run --package hypergate --bin hypergate -- start --config hypergate.toml --state hypergate.state.json
pause
