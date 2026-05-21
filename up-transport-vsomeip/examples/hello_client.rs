/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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

use async_trait::async_trait;
use log::{info, trace};
use std::fs::canonicalize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use up_rust::{
    PayloadEncoding, UFrameMetadata, UOwnedFrame, UOwnedListener, UOwnedTransport, UStatus, UUri,
};
use up_transport_vsomeip::UPTransportVsomeip;

const HELLO_SERVICE_ID: u16 = 0x6000;
const HELLO_INSTANCE_ID: u16 = 0x0001;
const HELLO_METHOD_ID: u16 = 0x7FFF;
const HELLO_SERVICE_MAJOR: u8 = 1;

const HELLO_SERVICE_AUTHORITY: &str = "linux";
const HELLO_SERVICE_UE_ID: u32 = ((HELLO_INSTANCE_ID as u32) << 16) | HELLO_SERVICE_ID as u32;
const HELLO_SERVICE_RESOURCE_ID: u16 = HELLO_METHOD_ID;

const CLIENT_AUTHORITY: &str = "me_authority";
const CLIENT_UE_ID: u32 = 0x5678;
const CLIENT_UE_VERSION_MAJOR: u8 = 1;
const CLIENT_RESOURCE_ID: u16 = 0;

const REQUEST_TTL: u32 = 1000;

struct LoggingResponseListener;

#[async_trait]
impl UOwnedListener for LoggingResponseListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        let payload = std::str::from_utf8(frame.payload_bytes()).unwrap_or("<non-UTF8 response>");
        info!("Received response: {payload}");
    }
}

#[tokio::main]
async fn main() -> Result<(), UStatus> {
    env_logger::init();

    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let vsomeip_config = PathBuf::from(crate_dir).join("vsomeip_configs/hello_service.json");
    let vsomeip_config = canonicalize(vsomeip_config).ok();
    trace!("vsomeip_config: {vsomeip_config:?}");

    let client_uuri = UUri::try_from_parts(
        CLIENT_AUTHORITY,
        CLIENT_UE_ID,
        CLIENT_UE_VERSION_MAJOR,
        CLIENT_RESOURCE_ID,
    )
    .unwrap();
    let client = Arc::new(UPTransportVsomeip::new_with_config(
        client_uuri.clone(),
        &HELLO_SERVICE_AUTHORITY.to_string(),
        &vsomeip_config.unwrap(),
        None,
    )?);

    let method = UUri::try_from_parts(
        HELLO_SERVICE_AUTHORITY,
        HELLO_SERVICE_UE_ID,
        HELLO_SERVICE_MAJOR,
        HELLO_SERVICE_RESOURCE_ID,
    )
    .unwrap();

    client
        .register_owned_listener(
            &method,
            Some(&client_uuri),
            Arc::new(LoggingResponseListener),
        )
        .await?;

    let mut i = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        let payload = format!("me_client@i={i}").into_bytes();
        i += 1;
        let frame = UOwnedFrame::new(
            UFrameMetadata::request(method.clone(), client_uuri.clone(), REQUEST_TTL)
                .with_encoding(PayloadEncoding::from_content_type("text/plain")),
            payload,
        );
        client.send_owned(frame).await?;
    }
}
