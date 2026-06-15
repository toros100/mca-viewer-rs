RUSTFLAGS="-Clink-arg=-Wl,--no-rosegment -Ctarget-cpu=native -Cforce-frame-pointers=yes" cargo flamegraph --profile prof --bin sim -- "$@" && xdg-open flamegraph.svg
