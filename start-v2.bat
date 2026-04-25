@echo off
cd /d "%~dp0"
cargo run --package hypergate-version-app --bin hypergate-version-v2
pause
