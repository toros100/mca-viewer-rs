RUSTFLAGS="-Clink-arg=-Wl,--no-rosegment -Ctarget-cpu=native -Cforce-frame-pointers=yes" cargo build --profile prof --bin sim && samply record ./target/prof/sim "$@"
