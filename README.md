# Eclipse uProtocol Rust vsomeip Client

## Overview

This library implements a uTransport client for vsomeip in Rust following the uProtocol [uTransport Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc).

## Getting Started

### Building the Library

To build the library, run:
```bash
VSOMEIP_INSTALL_PATH=<path/to/where/to/install/vsomeip> cargo build
```

in the project root directory.

See `vsomeip-sys/README.md` for more details on options.

This library leverages the [up-rust](https://github.com/eclipse-uprotocol/up-rust) library for data types and models specified by uProtocol.

This branch uses native `UOwnedFrame` values instead of generated `UMessage` transport envelopes. The vsomeip binding serializes a compact native-frame prefix before the application payload so it can preserve `UAttributes` and `UEncoding` across SOME/IP. `UEncoding.schema_ref` is preserved distinctly from the payload bytes and participates in typed decoder compatibility checks after receive. The transport remains owned-buffer based; it does not claim `UZeroCopyTransport` capability.

### Running the Tests

To run the tests, run
```bash
VSOMEIP_INSTALL_PATH=<path/to/vsomeip/install> LD_LIBRARY_PATH=$LD_LIBRARY_PATH:<path/to/vsomeip/install>/lib cargo test
```

Breaking this down:
* Details about the environment variables can be found in `vsomeip-sys/README.md`. Please reference there for further detail.
* Tests generate isolated vSomeIP network names and temporary configs, so the default parallel Rust test harness is supported.

### Using the Library

The library contains the following modules:

Package | [uProtocol spec](https://github.com/eclipse-uprotocol/uprotocol-spec) | Purpose
---|---|---
transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/uprotocol-spec/blob/main/up-l1/README.adoc) | Implementation of vsomeip uTransport client used for bidirectional point-2-point communication between uEs.
