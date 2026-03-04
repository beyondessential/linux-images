# CI/CD

r[ci.shellcheck]
All shell scripts in the repository must pass shellcheck with no errors.

r[ci.unit-test]
Unit tests must be checked in CI.

r[ci.uptodate]
All `uses:` actions must be up to date.

r[ci.rust-stable]
Rustup must be used to install and select the latest stable Rust version.
The dtolnay/rust-toolchain action must not be used.

r[ci.rust-cache]
The "swatinem" rust caching system must be used.

r[ci.output-arch]
CI must produce at least `amd64` and `arm64` outputs.

r[ci.output-suite]
CI must produce images based on Ubuntu Server 24.04 LTS.
