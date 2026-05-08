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

r[ci.installer-target]
The installer binary must be built with the `*-unknown-linux-gnu` Rust
target (not musl). The binary runs inside the live ISO rootfs, which is an
Ubuntu system with glibc, so static linking is unnecessary. The CI runner's
glibc version must be less than or equal to the glibc in the ISO rootfs
(set by `r[ci.output-suite]`), because glibc is forward-compatible but not
backward-compatible.

r[ci.output-arch]
CI must produce at least `amd64` and `arm64` outputs.

r[ci.output-suite]
CI must produce images for each supported Ubuntu Server suite. The currently
supported suites are `noble` (24.04 LTS) and `resolute` (26.04). Every suite
must be built for every supported architecture (`r[ci.output-arch]`).

> r[ci.release.aws-ami]
> On a tagged release, the cloud variant image for each supported (suite,
> architecture) combination must be registered as an AWS AMI in the
> `ap-southeast-2` region.
>
> Each registered AMI must be named
> `ubuntu-<ubuntu-version>-bes-cloud-<arch>-<version>`, where
> `<ubuntu-version>` is the numeric Ubuntu release (e.g. `24.04` or `26.04`)
> corresponding to the suite, `<arch>` is the image architecture, and
> `<version>` is the release version without the leading `v`. AMIs from
> different suites must therefore not collide on a name even when registered
> from the same release tag.
>
> Each registered AMI must carry the following AWS resource tags: `Name`, `Os`,
> `OsVersion`, `Variant`, `Architecture`, `Version`, `Features`, and `Builder`.
> `OsVersion` must hold the numeric Ubuntu release (`<ubuntu-version>` above).
