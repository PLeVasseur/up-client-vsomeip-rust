# Eclipse uProtocol Rust vsomeip Client

## Overview

This library implements a uTransport client for vsomeip in Rust following the uProtocol [uTransport Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc).

## Getting Started

### Building the Library

To build the library, setup the environment

``` bash
source build/envsetup.sh
```

then run:
```bash
VSOMEIP_INSTALL_PATH=<path/to/where/to/install/vsomeip> cargo build
```

in the project root directory.

See `vsomeip-sys/README.md` for more details on options.

This library leverages the [up-rust](https://github.com/eclipse-uprotocol/up-rust) library for data types and models specified by uProtocol.

### Running the Tests

To run the tests, run
```bash
VSOMEIP_INSTALL_PATH=<path/to/vsomeip/install> LD_LIBRARY_PATH=$LD_LIBRARY_PATH:<path/to/vsomeip/install>/lib cargo test -- --test-threads 1
```

Breaking this down:
* Details about the environment variables can be found in `vsomeip-sys/README.md`. Please reference there for further detail.
* We need to pass in `-- --test-threads 1` because the tests refer to the same configurations and will fall over if they are run simultaneously. So we instruct to use a single thread, i.e. run the tests in serial.

### Native vSomeIP Validation

Use the bundled path by default. It builds the supported COVESA vSomeIP from the bundled source through Cargo's default features, so it does not require a system `libvsomeip3` install.

```bash
source build/envsetup.sh highest
export VSOMEIP_INSTALL_PATH="$PWD/vsomeip-install"
cargo clean -p vsomeip-sys # only when recovering from a stale or empty install
LD_LIBRARY_PATH="$VSOMEIP_INSTALL_PATH/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" cargo test -p vsomeip-sys payload_len --locked
```

The same bundled smoke is available as a script:

```bash
scripts/native-smoke.sh --clean
```

Omit `--clean` for normal reruns after the bundled install exists.

Use the external path only when intentionally validating against an externally installed vSomeIP. Build and install COVESA vSomeIP 3.4.10 first, point `VSOMEIP_INSTALL_PATH` at that install, and disable the bundled feature:

```bash
export VSOMEIP_INSTALL_PATH=/path/to/vsomeip-3.4.10-install
LD_LIBRARY_PATH="$VSOMEIP_INSTALL_PATH/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" cargo test -p vsomeip-sys --no-default-features payload_len --locked
```

Do not use `--no-default-features` for the default bundled smoke. That flag disables the bundled build and expects an external `libvsomeip3` to be available.

### Using the Library

The library contains the following modules:

Package | [uProtocol spec](https://github.com/eclipse-uprotocol/uprotocol-spec) | Purpose
---|---|---
transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of vsomeip uTransport client used for bidirectional point-2-point communication between uEs.
