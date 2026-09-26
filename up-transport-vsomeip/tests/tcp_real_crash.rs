/********************************************************************************
 * Copyright (c) 2023 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

//! # Regression test – SOME/IP Return Code bug over real TCP
//!
//! The SOME/IP header carries a **Return Code** field at byte offset 15.
//! vsomeip validates this field inside `tcp_client_endpoint_impl::receive_cbk`:
//!
//! ```cpp
//! if (_recv_buffer_size > VSOMEIP_RETURN_CODE_POS /* = 15 */) {
//!     if (!utility::is_valid_return_code(_recv_buffer[15])) {
//!         stop_endpoint();   // resets the TCP connection
//!     }
//! }
//! ```
//!
//! The check fires only when the TCP receive buffer already holds > 15 bytes,
//! which happens on the **2nd response** after the buffer was retained at 16
//! bytes from the 1st response (see buffer mechanics below).
//!
//! ## Root cause
//!
//! `UPTransportVsomeip` left `commstatus = None` on `MT_RESPONSE` messages.
//! `message_conversions.rs` mapped `None → UCode::UNIMPLEMENTED`, serialised
//! as vsomeip `E_UNKNOWN = 0xFF`.  `0xFF` is **not** a valid SOME/IP Return
//! Code → vsomeip resets TCP → every subsequent RPC hangs until timeout.
//!
//! ## Fix
//!
//! `commstatus = None` on `MT_RESPONSE` now defaults to `UCode::OK` → `0x00`
//! (`E_OK`, always valid) → TCP stays alive → all RPCs complete normally.
//!
//! ## Buffer mechanics
//!
//! ```
//! recv_buffer_size_initial_ = 8
//!
//! Response 1 – 0-byte payload → 16-byte SOME/IP packet:
//!   read 8 → resize → read 8 more → full message processed
//!   buffer capacity retained: 16 bytes
//!
//! Response 2 – 2-byte payload → 18-byte SOME/IP packet:
//!   buffer = 16 → first read = 16 bytes → _recv_buffer_size = 16
//!   16 > 15 → validate byte 15 (Return Code):
//!     0xFF → invalid → stop_endpoint() → TCP RESET  ← BUG
//!     0x00 → valid  → read 2 more bytes → deliver   ← FIX
//! ```
//!
//! ## Test setup
//!
//! A **raw Rust TCP server** (no vsomeip) listens on `0.0.0.0:30511`.
//! vsomeip is configured with service unicast `127.0.0.2` (≠ own `127.0.0.1`)
//! so it treats the service as **remote** and opens a real TCP connection to
//! `127.0.0.2:30511`, processing responses through `tcp_client_endpoint_impl`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use up_rust::UCode;
use up_rust::{PayloadEncoding, UListener, UMessage, UMessageBuilder, UTransport, UUri};
use up_transport_vsomeip::{TransportConfig, UPTransportVsomeip};

const PORT: u16 = 30511;
/// Per-RPC polling timeout – shows the "stuck" effect on failed RPCs.
const RPC_TIMEOUT_MS: u64 = 800;

// ─────────────────────────────────────────────────────────────────────────────
// Server events (emitted to the test thread for in-sequence printing)
// ─────────────────────────────────────────────────────────────────────────────
enum Ev {
    Connected,
    Sent { rc: u8 },
    Closed,
}

fn ucode_to_return_code_mock(ucode: UCode) -> u8 {
    match ucode {
        UCode::Ok => 0x00,
        UCode::Unimplemented => 0xFF,
        _ => 0x01,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Raw SOME/IP TCP server (no vsomeip – just binary protocol)
// ─────────────────────────────────────────────────────────────────────────────
struct RawServer {
    rx: Receiver<Ev>,
    _t: std::thread::JoinHandle<()>,
}

impl RawServer {
    /// `bug_on`: if `Some(n)`, request #n gets invalid Return Code 0xFF.
    fn start(bug_on: Option<usize>) -> Self {
        let listener =
            TcpListener::bind(format!("0.0.0.0:{}", PORT)).expect("bind raw SOME/IP server");
        let (tx, rx): (Sender<Ev>, Receiver<Ev>) = mpsc::channel();

        let t = std::thread::spawn(move || {
            let (mut s, addr) = listener.accept().expect("accept");

            let local_addr = s.local_addr().unwrap();
            let is_ip = local_addr.is_ipv4() || local_addr.is_ipv6();

            println!("TCP connection established:");
            println!("    - Local OS socket : {}", local_addr);
            println!("    - Remote OS socket: {}", addr);
            println!(
                "    - Protocol Stack  : {}\n",
                if is_ip {
                    "TCP/IP (Network Stack)"
                } else {
                    "IPC"
                }
            );

            // Verify the OS-reported socket address family.
            assert!(
                is_ip,
                "The socket must be a TCP/IP socket, but an IPC was detected!"
            );

            tx.send(Ev::Connected).ok();

            let mut n = 0usize;
            loop {
                // SOME/IP 16-byte header:
                //   [0-1]  Service ID  [2-3]  Method ID
                //   [4-7]  Length  (= bytes remaining after offset 7)
                //   [8-9]  Client ID  [10-11] Session ID  ← echo these
                //   [12]   Proto Ver  [13]    Iface Ver
                //   [14]   Msg Type   [15]    Return Code ← THE CRITICAL BYTE
                let mut hdr = [0u8; 16];
                if s.read_exact(&mut hdr).is_err() {
                    tx.send(Ev::Closed).ok();
                    break;
                }
                let extra = (u32::from_be_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize)
                    .saturating_sub(8);
                let mut pl = vec![0u8; extra];
                if extra > 0 && s.read_exact(&mut pl).is_err() {
                    tx.send(Ev::Closed).ok();
                    break;
                }
                n += 1;

                // Negative path: craft the historical bad SOME/IP return code directly.
                // Positive path: always emit a valid return code and let the pipeline run end-to-end.
                let rc = if bug_on == Some(n) {
                    ucode_to_return_code_mock(UCode::Unimplemented)
                } else {
                    ucode_to_return_code_mock(UCode::Ok)
                };

                // Response size:
                //   Request 1  (0-byte payload) → 16-byte response → buffer = 16
                //   Requests 2+ (2-byte payload) → 18-byte response → triggers 16>15 check
                let resp_pl: &[u8] = if n == 1 { b"" } else { b"\x00\x00" };
                let len = (8u32 + resp_pl.len() as u32).to_be_bytes();
                let mut resp = vec![
                    hdr[0], hdr[1], hdr[2], hdr[3], len[0], len[1], len[2], len[3], hdr[8], hdr[9],
                    hdr[10], hdr[11], // Client+Session ID echo
                    0x01, 0x01, 0x80, // Proto, Iface, RESPONSE
                    rc,   // Return Code
                ];
                resp.extend_from_slice(resp_pl);

                if s.write_all(&resp).is_err() {
                    tx.send(Ev::Closed).ok();
                    break;
                }
                tx.send(Ev::Sent { rc }).ok();
            }
        });

        RawServer { rx, _t: t }
    }

    fn wait(&self) -> Option<Ev> {
        self.rx.recv_timeout(Duration::from_secs(5)).ok()
    }
    fn try_next(&self) -> Option<Ev> {
        self.rx.try_recv().ok()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// uProtocol response listener
// ─────────────────────────────────────────────────────────────────────────────
struct Listener(AtomicUsize);
impl Listener {
    fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}
#[async_trait::async_trait]
impl UListener for Listener {
    async fn on_receive(&self, _: UMessage) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────
async fn build_client() -> (Arc<UPTransportVsomeip>, UUri, UUri) {
    let cfg = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("vsomeip_configs/tcp_client_raw_remote.json");
    std::env::set_var("VSOMEIP_CONFIGURATION", cfg.to_str().unwrap());
    let t = Arc::new(
        UPTransportVsomeip::new_with_config_and_transport_config(
            UUri::try_from_parts("bar", 0x0345u32, 1u8, 0u16).unwrap(),
            &"foo".to_string(),
            &cfg,
            None,
            TransportConfig::new(PayloadEncoding::RAW),
        )
        .expect("start transport"),
    );
    let client = UUri::try_from_parts("bar", 0x0345u32, 1u8, 0u16).unwrap();
    let service = UUri::try_from_parts("foo", 0x1234u32, 1u8, 0x0421u16).unwrap();
    (t, client, service)
}

async fn rpc(
    client: &Arc<UPTransportVsomeip>,
    svc: &UUri,
    src: &UUri,
    payload: Vec<u8>,
    listener: &Arc<Listener>,
    server: &RawServer,
    n: usize,
) {
    let prev = listener.count();
    let start = std::time::Instant::now();
    let pl_len = payload.len();

    let msg = UMessageBuilder::request(svc.clone(), src.clone(), 5000)
        .build_with_payload(payload, PayloadEncoding::RAW)
        .expect("build");
    client.send(msg).await.expect("send");

    // Poll until response arrives or per-RPC timeout expires
    let deadline = std::time::Instant::now() + Duration::from_millis(RPC_TIMEOUT_MS);
    let ok = loop {
        if listener.count() > prev {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    let ms = start.elapsed().as_millis();

    // Collect server event (if any arrived during the poll)
    let srv = server.try_next();

    if ok {
        let rc = match srv {
            Some(Ev::Sent { rc }) => rc,
            _ => 0x00,
        };
        println!(
            "  RPC {n}  ({pl_len} bytes)  rc=0x{rc:02X}  \
             ✓  delivered in  {ms:>4} ms"
        );
    } else {
        let label = match srv {
            Some(Ev::Sent { rc: 0xFF }) => {
                "rc=0xFF  ✗  TCP RESET  (see vsomeip [error] above)".to_string()
            }
            _ => format!("         ✗  no TCP — timeout after {ms:>4} ms"),
        };
        println!("  RPC {n}  ({pl_len} bytes)  {label}");
    }
}

fn bar(ch: char) {
    println!("{}", std::iter::repeat_n(ch, 60).collect::<String>());
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 1 – WITHOUT FIX
//
// RPC 1 succeeds (buffer → 16 bytes).
// RPC 2: server sends 0xFF → vsomeip resets TCP → response dropped.
// RPCs 3-5: no TCP → each waits RPC_TIMEOUT_MS ms → timeout.
//
// Expected: 1 / 5 responses received.  Total time ≈ 1 × fast + 4 × 800 ms.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[allow(clippy::await_holding_lock)]
async fn real_tcp_crash_without_fix() {
    let _ = tracing_subscriber::fmt::try_init();

    println!();
    bar('═');
    println!("  BUG SCENARIO  |  SOME/IP Return Code 0xFF  →  TCP reset");
    bar('═');

    let server = RawServer::start(Some(2)); // bug on request #2: crafted invalid return code

    let (client, cu, su) = build_client().await;
    let listener = Arc::new(Listener::new());
    client
        .register_listener(&su, Some(&cu), listener.clone() as _)
        .await
        .expect("reg");

    tokio::time::sleep(Duration::from_millis(800)).await;
    if let Some(Ev::Connected) = server.wait() {
        println!("  [TCP]  vsomeip connected  →  raw server 127.0.0.2:{PORT}");
        println!("         (uses real TCP: service unicast ≠ client unicast)");
    }
    println!();

    let t0 = std::time::Instant::now();

    // RPC 1: 0-byte payload → 16-byte SOME/IP response → buffer = 16
    rpc(&client, &su, &cu, vec![], &listener, &server, 1).await;
    // RPC 2: 2-byte payload → 18-byte response → 16>15 → rc=0xFF → TCP RESET
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 2).await;
    // RPCs 3-5: TCP gone → each times out
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 3).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 4).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 5).await;

    let total = t0.elapsed();
    println!();
    bar('─');
    println!(
        "  Received: {} / 5    Total time: {:.1}s",
        listener.count(),
        total.as_secs_f32()
    );
    bar('─');
    println!();

    assert_eq!(
        listener.count(),
        1,
        "WITHOUT FIX: only RPC 1 must be delivered; vsomeip rejects 0xFF and resets TCP"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// TEST 2 – WITH FIX
//
// All 5 RPCs get Return Code 0x00 (E_OK).
// vsomeip validates byte 15 = 0x00 on every response → valid → deliver.
// TCP stays alive. All RPCs complete in tens of milliseconds each.
//
// Expected: 5 / 5 responses received.  Total time ≈ 5 × fast.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
#[allow(clippy::await_holding_lock)]
async fn real_tcp_stable_with_fix() {
    let _ = tracing_subscriber::fmt::try_init();

    println!();
    bar('═');
    println!("  FIX SCENARIO  |  SOME/IP Return Code 0x00  →  TCP stable");
    bar('═');

    let server = RawServer::start(None); // always valid return code

    let (client, cu, su) = build_client().await;
    let listener = Arc::new(Listener::new());
    client
        .register_listener(&su, Some(&cu), listener.clone() as _)
        .await
        .expect("reg");

    tokio::time::sleep(Duration::from_millis(800)).await;
    if let Some(Ev::Connected) = server.wait() {
        println!("  [TCP]  vsomeip connected  →  raw server 127.0.0.2:{PORT}");
        println!("         (uses real TCP: service unicast ≠ client unicast)");
    }
    println!();

    let t0 = std::time::Instant::now();

    // All 5 RPCs succeed — 16>15 check passes on each because rc=0x00
    rpc(&client, &su, &cu, vec![], &listener, &server, 1).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 2).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 3).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 4).await;
    rpc(&client, &su, &cu, vec![0x08, 0x01], &listener, &server, 5).await;

    let total = t0.elapsed();
    println!();
    bar('─');
    println!(
        "  Received: {} / 5    Total time: {:.1}s    (no vsomeip error logs)",
        listener.count(),
        total.as_secs_f32()
    );
    bar('─');
    println!();

    assert_eq!(
        listener.count(),
        5,
        "WITH FIX: all 5 RPCs must be delivered (Return Code 0x00 is always valid)"
    );
}
