# bmcd

`bmcd` or 'BMC Daemon' is part of the
[BMC-Firmware](https://www.github.com/turing-machines/BMC-Firmware) and is
responsible for hosting Restful APIs related to node management, and
configuration of a Turing-Pi 2 board.

## Building

This package will be built as part of the buildroot firmware located
[here](https://www.github.com/turing-machines/BMC-Firmware). If you want to
build bmcd in isolation, we recommend to use `cargo cross`. Given you have a
Rust toolchain installed, execute the following commands:

```bash
# Install cross environment
cargo install cross --git https://github.com/cross-rs/cross

# Execute cross build command for the Turing-Pi target.
cross build --target armv7-unknown-linux-gnueabi --release --features vendored
# A self contained binary is build when the "vendored" feature flag is defined.
# i.e. Openssl will be statically linked into the binary. This is not desirable
# when building the actual BMC-Firmware, but works great for debugging scenario's.

# Copy to turing-pi.
scp target/armv7-unknown-linux-gnueabi/release/bmcd root@turingpi.local:/mnt/sdcard/bmcd
```

