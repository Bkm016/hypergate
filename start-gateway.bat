@echo off
cd /d "%~dp0"
cargo run --package hypergate --bin hypergate
pause
