check:
    cargo clippy --all-targets --all-features -- -D warnings
    cargo nextest run --no-capture
