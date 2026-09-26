# Rust based SOME/IP Transport Library for Eclipse uProtocol&trade;

This crate implements the SOME/IP transport as specified in [uProtocol v1.6.0-alpha.7](https://github.com/eclipse-uprotocol/up-spec/tree/v1.6.0-alpha.7) based on the [COVESA vsomeip library](https://github.com/COVESA/vsomeip).

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

This library leverages the [uProtocol Rust Language Library](https://github.com/eclipse-uprotocol/up-rust) for data types and models specified by uProtocol.

### Payload Encoding Convention

SOME/IP carries payload bytes but no uProtocol payload-encoding identifier.
`TransportConfig` therefore defines one assumed nonzero `PayloadEncoding` for a
transport. Incoming nonempty payloads receive that identity, and outgoing
payload-bearing messages are rejected unless their declared identity matches.
Payloadless messages remain identity-free.

Use `UPTransportVsomeip::new_with_transport_config` or
`UPTransportVsomeip::new_with_config_and_transport_config` when the convention
must be explicit. The compatibility constructors use the default
`PayloadEncoding::PROTOBUF` convention.

SOME/IP has no payload-presence bit. A zero-length wire payload cannot preserve
the distinction between absent and present-empty payloads, so receive maps zero
bytes to payload absence rather than inventing an encoding declaration.

Directed Notifications use SOME/IP `REQUEST_NO_RETURN`. The destination maps to
the SOME/IP service, instance, and method, but SOME/IP exposes only the sender's
client ID rather than its original uProtocol event URI. Receive therefore
reconstructs an event source from that client ID; the original source URI is not
preserved on the wire.

### Running the Tests

To run the tests:

```bash
VSOMEIP_INSTALL_PATH=<path/to/vsomeip/install> LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$VSOMEIP_INSTALL_PATH/lib cargo test
```

Breaking this down:
* Details about the environment variables can be found in `vsomeip-sys/README.md`.

The bundled native role smoke can also be run with:

```bash
scripts/native-smoke.sh --clean
```

### Using the Library

The library contains the following modules:

| Package   | [uProtocol spec](https://github.com/eclipse-uprotocol/uprotocol-spec) | Purpose |
| :-------- | :-------------------------------------------------------------------- | :------ |
| transport | [uP-L1 Specifications](https://github.com/eclipse-uprotocol/up-spec/blob/v1.6.0-alpha.7/up-l1/README.adoc) | Implementation of the `UTransport` trait used for bidirectional point-2-point communication between uEntities.
