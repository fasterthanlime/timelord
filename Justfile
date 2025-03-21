check:
    cargo clippy --all-targets --all-features -- -D warnings
    cargo run -- --self-test
