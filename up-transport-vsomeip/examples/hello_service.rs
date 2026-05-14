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
use log::trace;
use std::fs::canonicalize;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::thread;
use up_rust::{
    UEncoding, UFrameMetadata, UOwnedFrame, UOwnedListener, UOwnedTransport, UStatus, UUri,
};
use up_transport_vsomeip::UPTransportVsomeip;

const HELLO_SERVICE_ID: u16 = 0x6000;
const HELLO_INSTANCE_ID: u32 = 0x0001;
const HELLO_METHOD_ID: u16 = 0x7FFF;
const HELLO_SERVICE_MAJOR: u8 = 1;

const HELLO_SERVICE_AUTHORITY: &str = "linux";
const HELLO_SERVICE_UE_ID: u32 = (HELLO_INSTANCE_ID << 16_u32) | HELLO_SERVICE_ID as u32;
const HELLO_SERVICE_RESOURCE_ID: u16 = HELLO_METHOD_ID;

const CLIENT_AUTHORITY: &str = "me_authority";

struct ServiceRequestHandler {
    transport: Weak<UPTransportVsomeip>,
}

impl ServiceRequestHandler {
    fn new(transport: Arc<UPTransportVsomeip>) -> Self {
        Self {
            transport: Arc::downgrade(&transport),
        }
    }
}

#[async_trait]
impl UOwnedListener for ServiceRequestHandler {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        let request = std::str::from_utf8(frame.payload_bytes()).unwrap_or("<non-UTF8 request>");
        println!("ServiceRequestHandler received request: {request}");

        let Some(reply_to) = Some(frame.metadata().attributes().source().clone()) else {
            return;
        };
        let Some(invoked_method) = frame.metadata().attributes().sink().cloned() else {
            return;
        };
        let response = UOwnedFrame::new(
            UFrameMetadata::response(
                reply_to,
                frame.metadata().attributes().id().clone(),
                invoked_method,
            )
            .with_encoding(UEncoding::from_content_type("text/plain")),
            format!("The response to the request: {request}").into_bytes(),
        );

        if let Some(transport) = self.transport.upgrade() {
            transport
                .send_owned(response)
                .await
                .expect("failed to send response");
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), UStatus> {
    env_logger::init();

    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let vsomeip_config = PathBuf::from(crate_dir).join("vsomeip_configs/hello_service.json");
    let vsomeip_config = canonicalize(vsomeip_config).ok();
    trace!("vsomeip_config: {vsomeip_config:?}");

    let service_uuri =
        UUri::try_from_parts(HELLO_SERVICE_AUTHORITY, HELLO_SERVICE_UE_ID, 1, 0).unwrap();
    let service = Arc::new(UPTransportVsomeip::new_with_config(
        service_uuri,
        &CLIENT_AUTHORITY.to_string(),
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
    service
        .register_owned_listener(
            &UUri::any(),
            Some(&method),
            Arc::new(ServiceRequestHandler::new(service.clone())),
        )
        .await?;

    thread::park();
    Ok(())
}
