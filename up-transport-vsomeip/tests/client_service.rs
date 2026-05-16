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

mod test_lib;

use log::{error, info};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::time::Instant;
use up_rust::{
    UCode, UEncoding, UFrameMetadata, UOwnedFrame, UOwnedListener, UOwnedTransport, UUri,
};
use up_transport_vsomeip::UPTransportVsomeip;

const TEST_DURATION: u64 = 2000;
const MAX_ITERATIONS: usize = 100;

pub struct ResponseListener {
    received_response: AtomicUsize,
}
impl ResponseListener {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            received_response: AtomicUsize::new(0),
        }
    }

    pub fn received_response(&self) -> usize {
        self.received_response.load(Ordering::SeqCst)
    }
}
#[async_trait::async_trait]
impl UOwnedListener for ResponseListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        info!("Received Response:\n{:?}", frame);

        let payload_bytes = frame.payload_bytes().to_vec();
        info!("Received response payload_bytes of: {payload_bytes:?}");
        let Ok(response_payload_string) = std::str::from_utf8(&payload_bytes) else {
            panic!("unable to convert payload_bytes to string");
        };
        info!("Response payload_string: {response_payload_string}");

        self.received_response.fetch_add(1, Ordering::SeqCst);
    }
}
pub struct RequestListener {
    client: Weak<UPTransportVsomeip>,
    received_request: AtomicUsize,
}

impl RequestListener {
    #[allow(clippy::new_without_default)]
    pub fn new(client: Arc<UPTransportVsomeip>) -> Self {
        Self {
            client: Arc::downgrade(&client),
            received_request: AtomicUsize::new(0),
        }
    }

    pub fn received_request(&self) -> usize {
        self.received_request.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl UOwnedListener for RequestListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        self.received_request.fetch_add(1, Ordering::SeqCst);
        info!("Received Request:\n{:?}", frame);

        let payload_bytes = frame.payload_bytes().to_vec();
        info!("Received request payload_bytes of: {payload_bytes:?}");
        let Ok(payload_string) = std::str::from_utf8(&payload_bytes) else {
            panic!("Unable to unpack string from payload_bytes");
        };
        info!("Request payload_string: {payload_string}");

        let response_payload_string = format!("Here's a response to: {payload_string}");
        let response_payload_bytes = response_payload_string.into_bytes();

        let reply_to = frame.metadata().attributes().source().clone();
        let invoked_method = frame
            .metadata()
            .attributes()
            .sink()
            .expect("Request frame has no invoked method")
            .clone();
        let response_header = UFrameMetadata::response(
            reply_to,
            frame.metadata().attributes().id().clone(),
            invoked_method,
        )
        .with_encoding(UEncoding::from_content_type("text/plain"));
        let response_header = UFrameMetadata::new(
            response_header
                .attributes()
                .clone()
                .with_comm_status(UCode::OK),
            response_header.encoding().cloned(),
        );
        let response_msg = UOwnedFrame::new(response_header, response_payload_bytes);
        if let Some(client) = self.client.upgrade() {
            let send_res = client.send_owned(response_msg).await;
            if let Err(err) = send_res {
                panic!("Unable to send response frame: {:?}", err);
            }
        }
    }
}

fn request_frame(method: UUri, reply_to: UUri, ttl: u32, payload: Vec<u8>) -> UOwnedFrame {
    UOwnedFrame::new(
        UFrameMetadata::request(method, reply_to, ttl)
            .with_encoding(UEncoding::from_content_type("text/plain")),
        payload,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn client_service() {
    test_lib::before_test();
    // console_subscriber::init();
    let network = test_lib::VsomeipTestNetwork::new("client_service");

    let service_authority_name = "foo";
    let streamer_ue_id = 0x7878;

    let service_1_ue_id = 0x1234;
    let service_1_ue_version_major = 1;
    let service_1_resource_id_a = 0x0421;

    let client_authority_name = "bar";
    let client_ue_id = 0x0345;
    let client_ue_version_major = 1;
    let client_resource_id = 0x0000;

    let client_config = network.config("837", 0x0345);

    let client_uuri = UUri::try_from_parts(client_authority_name, streamer_ue_id, 1, 0).unwrap();
    let client_res = UPTransportVsomeip::new_with_config(
        client_uuri,
        &service_authority_name.to_string(),
        client_config.path(),
        None,
    );

    let Ok(client) = client_res else {
        panic!(
            "Unable to establish client: {:?}",
            client_res.err().unwrap()
        );
    };

    tokio::time::sleep(Duration::from_millis(200)).await;

    let client_uuri = UUri::try_from_parts(
        client_authority_name,
        client_ue_id as u32,
        client_ue_version_major,
        client_resource_id,
    )
    .unwrap();

    let service_1_uuri_method_a = UUri::try_from_parts(
        service_authority_name,
        service_1_ue_id as u32,
        service_1_ue_version_major,
        service_1_resource_id_a,
    )
    .unwrap();

    let response_listener_check = Arc::new(ResponseListener::new());
    let response_listener: Arc<dyn UOwnedListener> = response_listener_check.clone();

    let reg_res_1 = client
        .register_owned_listener(
            &service_1_uuri_method_a,
            Some(&client_uuri),
            response_listener.clone(),
        )
        .await;
    if let Err(err) = reg_res_1 {
        panic!("Unable to register for returning Response: {:?}", err);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let service_config = network.config("4660", 0x1234);

    let service_uuri = UUri::try_from_parts(service_authority_name, streamer_ue_id, 1, 0).unwrap();
    let service_res = UPTransportVsomeip::new_with_config(
        service_uuri,
        &client_authority_name.to_string(),
        service_config.path(),
        None,
    );

    let Ok(service) = service_res else {
        panic!("Unable to establish subscriber");
    };

    tokio::time::sleep(Duration::from_millis(200)).await;

    let service = Arc::new(service);

    let service_1_uuri = UUri::try_from_parts(
        service_authority_name,
        service_1_ue_id as u32,
        service_1_ue_version_major,
        service_1_resource_id_a,
    )
    .unwrap();

    let request_listener_check = Arc::new(RequestListener::new(service.clone()));
    let request_listener: Arc<dyn UOwnedListener> = request_listener_check.clone();

    let reg_service_1 = service
        .register_owned_listener(
            &UUri::any(),
            Some(&service_1_uuri),
            request_listener.clone(),
        )
        .await;

    if let Err(err) = reg_service_1 {
        error!("Unable to register: {:?}", err);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Track the start time and set the duration for the loop
    let duration = Duration::from_millis(TEST_DURATION);
    let start_time = Instant::now();
    let mut iterations = 0;
    let mut i = 20;

    // limit with iterations to ensure socket transactions can complete during test
    while (Instant::now().duration_since(start_time) < duration) && (iterations < MAX_ITERATIONS) {
        let payload_string = format!("request@i={i}");
        let payload = payload_string.into_bytes();
        let request_msg_1_a = request_frame(
            service_1_uuri_method_a.clone(),
            client_uuri.clone(),
            10000,
            payload,
        );

        let send_res_1_a = client.send_owned(request_msg_1_a).await;

        if let Err(err) = send_res_1_a {
            panic!("Unable to send Request frame: {:?}", err);
        }

        iterations += 1;
        i += 1;
    }

    tokio::time::sleep(Duration::from_millis(2000)).await;

    println!("iterations: {}", iterations);
    println!(
        "request_listener_check.received_request(): {}",
        request_listener_check.received_request()
    );
    println!(
        "response_listener_check.received_response(): {}",
        response_listener_check.received_response()
    );

    assert_eq!(iterations, request_listener_check.received_request());
    assert_eq!(iterations, response_listener_check.received_response());
}
