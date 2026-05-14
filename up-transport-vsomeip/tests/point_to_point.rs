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

use log::{error, info, trace};
use std::env::current_dir;
use std::fs::canonicalize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::time::Instant;
use up_rust::{
    UEncoding, UFrameHeader, UMessageType, UOwnedFrame, UOwnedListener, UOwnedTransport, UUri, UUID,
};
use up_transport_vsomeip::UPTransportVsomeip;

const TEST_DURATION: u64 = 500;
const MAX_ITERATIONS: usize = 100;
const STREAMER_UE_ID: u32 = 0x9876;

const CLIENT_AUTHORITY_NAME: &str = "foo";
const CLIENT_UE_ID: u32 = 0x1234;
const CLIENT_UE_VERSION_NUMBER: u32 = 1;

const PTP_AUTHORITY_NAME: &str = "foo";
const PTP_UE_ID: u32 = 0x2345;
const PTP_UE_VERSION_NUMBER: u32 = 1;
const PTP_METHOD_RESOURCE_ID: u32 = 0x0421;

const SERVICE_AUTHORITY_NAME: &str = "foo";
const SERVICE_UE_ID: u32 = 0x3456;
const SERVICE_UE_VERSION_NUMBER: u32 = 1;
const SERVICE_METHOD_RESOURCE_ID: u32 = 0x0421;

const NON_POINT_TO_POINT_LISTENED_AUTHORITY: &str = "oops";

fn client_reply_uuri() -> UUri {
    UUri {
        authority_name: CLIENT_AUTHORITY_NAME.to_string(),
        ue_id: CLIENT_UE_ID,
        ue_version_major: CLIENT_UE_VERSION_NUMBER,
        resource_id: 0x0000,
    }
}

fn ptp_reply_uuri() -> UUri {
    UUri {
        authority_name: PTP_AUTHORITY_NAME.to_string(),
        ue_id: PTP_UE_ID,
        ue_version_major: PTP_UE_VERSION_NUMBER,
        resource_id: 0x0000,
    }
}

fn ptp_method_uuri() -> UUri {
    UUri {
        authority_name: PTP_AUTHORITY_NAME.to_string(),
        ue_id: PTP_UE_ID,
        ue_version_major: PTP_UE_VERSION_NUMBER,
        resource_id: PTP_METHOD_RESOURCE_ID,
    }
}

fn service_uuri() -> UUri {
    UUri {
        authority_name: SERVICE_AUTHORITY_NAME.to_string(),
        ue_id: SERVICE_UE_ID,
        ue_version_major: SERVICE_UE_VERSION_NUMBER,
        resource_id: SERVICE_METHOD_RESOURCE_ID,
    }
}

fn text_encoding() -> UEncoding {
    UEncoding::from_content_type("text/plain")
}

fn request_frame(method: UUri, reply_to: UUri, payload: Vec<u8>) -> UOwnedFrame {
    UOwnedFrame::new(
        UFrameHeader::request(method, reply_to, 1000).with_encoding(text_encoding()),
        payload,
    )
}

fn response_frame(
    reply_to: UUri,
    request_id: UUID,
    invoked_method: UUri,
    payload: Vec<u8>,
) -> UOwnedFrame {
    UOwnedFrame::new(
        UFrameHeader::response(reply_to, request_id, invoked_method).with_encoding(text_encoding()),
        payload,
    )
}

pub struct PointToPointListener {
    client: Weak<UPTransportVsomeip>,
    received_request: AtomicUsize,
    received_response: AtomicUsize,
}

impl PointToPointListener {
    pub fn new(client: Arc<UPTransportVsomeip>) -> Self {
        Self {
            client: Arc::downgrade(&client),
            received_request: AtomicUsize::new(0),
            received_response: AtomicUsize::new(0),
        }
    }

    pub fn received_request(&self) -> usize {
        self.received_request.load(Ordering::SeqCst)
    }

    pub fn received_response(&self) -> usize {
        self.received_response.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl UOwnedListener for PointToPointListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        info!("Received in point-to-point listener:\n{:?}", frame);

        let received_source_authority = frame.header().attributes().source().authority_name.clone();
        if received_source_authority == NON_POINT_TO_POINT_LISTENED_AUTHORITY {
            panic!(
                "Received a message on point to point listener that we should not have:\n{frame:?}"
            );
        }

        let Some(client) = self.client.upgrade() else {
            panic!("Unable to get ahold of the transport within PointToPointListener");
        };

        match frame.header().attributes().message_type() {
            UMessageType::Request => {
                trace!("PointToPointListener got a request");
                self.received_request.fetch_add(1, Ordering::SeqCst);

                let original_id = frame.header().attributes().id().clone();
                let forwarding_request = request_frame(
                    service_uuri(),
                    ptp_reply_uuri(),
                    original_id.to_hyphenated_string().into_bytes(),
                );

                client
                    .send_owned(forwarding_request)
                    .await
                    .unwrap_or_else(|err| {
                        error!("Unable to forward request: {err:?}");
                        panic!("Unable to forward request: {err:?}");
                    });
            }
            UMessageType::Response => {
                trace!("PointToPointListener got a response: {:?}", frame);
                self.received_response.fetch_add(1, Ordering::SeqCst);

                let original_id = std::str::from_utf8(frame.payload_bytes())
                    .expect("forwarded response payload is not UTF-8")
                    .parse::<UUID>()
                    .expect("forwarded response payload is not a UUID");
                let response = response_frame(
                    client_reply_uuri(),
                    original_id,
                    ptp_method_uuri(),
                    Vec::new(),
                );

                client.send_owned(response).await.unwrap_or_else(|err| {
                    panic!("Unable to forward response: {err:?}");
                });
            }
            UMessageType::Publish => {
                panic!("uProtocol PUBLISH received. This shouldn't happen!");
            }
            UMessageType::Notification => {
                panic!("Not supported message type: NOTIFICATION");
            }
        }
    }
}

pub struct ResponseListener {
    received_response: AtomicUsize,
}

impl ResponseListener {
    pub fn new() -> Self {
        Self {
            received_response: AtomicUsize::new(0),
        }
    }

    pub fn received_response(&self) -> usize {
        self.received_response.load(Ordering::SeqCst)
    }
}

impl Default for ResponseListener {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl UOwnedListener for ResponseListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        info!("ResponseListener: Received Response:\n{:?}", frame);
        self.received_response.fetch_add(1, Ordering::SeqCst);
    }
}

pub struct RequestListener {
    client: Weak<UPTransportVsomeip>,
    received_request: AtomicUsize,
}

impl RequestListener {
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

        let original_id = std::str::from_utf8(frame.payload_bytes())
            .expect("forwarded request payload is not UTF-8")
            .to_string();
        let reply_to = frame.header().attributes().source().clone();
        let invoked_method = frame
            .header()
            .attributes()
            .sink()
            .expect("Request frame has no invoked method")
            .clone();
        let response = response_frame(
            reply_to,
            frame.header().attributes().id().clone(),
            invoked_method,
            original_id.into_bytes(),
        );

        if let Some(client) = self.client.upgrade() {
            client.send_owned(response).await.unwrap_or_else(|err| {
                panic!("Unable to send service response frame: {err:?}");
            });
        }
    }
}

fn any_from_authority(authority_name: &str) -> UUri {
    let mut any_with_authority = UUri::any();
    any_with_authority.authority_name = authority_name.to_string();
    any_with_authority
}

#[tokio::test(flavor = "multi_thread")]
async fn point_to_point() {
    env_logger::init();

    let current_dir = current_dir();
    info!("{current_dir:?}");

    let vsomeip_config_path = "vsomeip_configs/point_to_point_integ.json";
    let abs_vsomeip_config_path = canonicalize(vsomeip_config_path).ok();
    info!("abs_vsomeip_config_path: {abs_vsomeip_config_path:?}");

    let point_to_point_uri =
        UUri::try_from_parts(PTP_AUTHORITY_NAME, STREAMER_UE_ID, 1, 0).unwrap();
    let point_to_point_client = UPTransportVsomeip::new_with_config(
        point_to_point_uri,
        &PTP_AUTHORITY_NAME.to_string(),
        &abs_vsomeip_config_path.unwrap(),
        None,
    )
    .unwrap_or_else(|err| panic!("Unable to establish owned transport: {err:?}"));
    let point_to_point_client = Arc::new(point_to_point_client);

    let source = any_from_authority(PTP_AUTHORITY_NAME);
    let legacy_source = UUri::any();
    let sink = any_from_authority(PTP_AUTHORITY_NAME);

    let point_to_point_listener_check =
        Arc::new(PointToPointListener::new(point_to_point_client.clone()));
    let point_to_point_listener: Arc<dyn UOwnedListener> = point_to_point_listener_check.clone();
    let reg_res = point_to_point_client
        .register_owned_listener(&source, Some(&sink), point_to_point_listener.clone())
        .await;
    if let Err(err) = reg_res {
        panic!("Unable to register with owned transport: {err}");
    }

    let fallback_reg_res = point_to_point_client
        .register_owned_listener(&legacy_source, Some(&sink), point_to_point_listener)
        .await;
    if let Err(err) = fallback_reg_res {
        trace!("Fallback point-to-point listener registration not applied: {err}");
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let client_config = "vsomeip_configs/client.json";
    let client_config = canonicalize(client_config).ok();
    info!("client_config: {client_config:?}");

    let client_uri = UUri::try_from_parts(CLIENT_AUTHORITY_NAME, CLIENT_UE_ID, 1, 0).unwrap();
    let client = UPTransportVsomeip::new_with_config(
        client_uri,
        &CLIENT_AUTHORITY_NAME.to_string(),
        &client_config.unwrap(),
        None,
    )
    .unwrap_or_else(|err| panic!("Unable to establish client: {err:?}"));

    tokio::time::sleep(Duration::from_millis(200)).await;

    let response_listener_check = Arc::new(ResponseListener::new());
    let response_listener: Arc<dyn UOwnedListener> = response_listener_check.clone();

    let reg_res_1 = client
        .register_owned_listener(
            &ptp_method_uuri(),
            Some(&client_reply_uuri()),
            response_listener.clone(),
        )
        .await;
    if let Err(err) = reg_res_1 {
        panic!("Unable to register for returning Response: {err:?}");
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let service_config = "vsomeip_configs/service.json";
    let service_config = canonicalize(service_config).ok();
    info!("service_config: {service_config:?}");

    let service_uri = UUri::try_from_parts(SERVICE_AUTHORITY_NAME, SERVICE_UE_ID, 1, 0).unwrap();
    let service = UPTransportVsomeip::new_with_config(
        service_uri,
        &SERVICE_AUTHORITY_NAME.to_string(),
        &service_config.unwrap(),
        None,
    )
    .unwrap_or_else(|err| panic!("Unable to establish service: {err:?}"));

    tokio::time::sleep(Duration::from_millis(200)).await;

    let service = Arc::new(service);

    let request_listener_check = Arc::new(RequestListener::new(service.clone()));
    let request_listener: Arc<dyn UOwnedListener> = request_listener_check.clone();

    let reg_service_1 = service
        .register_owned_listener(
            &UUri::any(),
            Some(&service_uuri()),
            request_listener.clone(),
        )
        .await;

    if let Err(err) = reg_service_1 {
        error!("Unable to register: {err:?}");
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let duration = Duration::from_millis(TEST_DURATION);
    let start_time = Instant::now();
    let mut iterations = 0;

    while (Instant::now().duration_since(start_time) < duration) && (iterations < MAX_ITERATIONS) {
        let request_msg = request_frame(ptp_method_uuri(), client_reply_uuri(), Vec::new());
        trace!("Sending message from client: {request_msg:?}");
        client
            .send_owned(request_msg)
            .await
            .unwrap_or_else(|err| panic!("Unable to send message: {err:?}"));

        iterations += 1;
    }

    tokio::time::sleep(Duration::from_millis(2000)).await;

    assert_eq!(iterations, request_listener_check.received_request());
    assert_eq!(iterations, point_to_point_listener_check.received_request());
    assert_eq!(
        iterations,
        point_to_point_listener_check.received_response()
    );
    assert_eq!(iterations, response_listener_check.received_response());
}
