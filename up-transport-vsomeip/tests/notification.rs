/********************************************************************************
 * Copyright (c) 2026 Contributors to the Eclipse Foundation
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

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use up_rust::{
    PayloadEncoding, UListener, UMessage, UMessageBuilder, UMessageType, UTransport, UUri,
};
use up_transport_vsomeip::{TransportConfig, UPTransportVsomeip, VsomeipApplicationConfig};

struct ChannelListener(UnboundedSender<UMessage>);

#[async_trait::async_trait]
impl UListener for ChannelListener {
    async fn on_receive(&self, message: UMessage) {
        self.0.send(message).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn directed_notification_uses_request_no_return() {
    let _ = env_logger::builder().is_test(true).try_init();
    let encoding = PayloadEncoding::TEXT;
    let sink = UUri::try_from_parts("receiver", 0x6200, 1, 0).unwrap();
    let source = UUri::try_from_parts("sender", 0x6201, 1, 0x8000).unwrap();

    let receiver = UPTransportVsomeip::new_with_transport_config(
        VsomeipApplicationConfig::new("notification_receiver", 0x6200),
        sink.clone(),
        &"sender".to_string(),
        None,
        TransportConfig::new(encoding),
    )
    .unwrap();
    let source_filter = UUri::try_from_parts("sender", 0xFFFF_FFFF, 0xFF, 0xFFFF).unwrap();
    let (tx, mut rx) = unbounded_channel();
    let listener: Arc<dyn UListener> = Arc::new(ChannelListener(tx));
    receiver
        .register_listener(&source_filter, Some(&sink), listener.clone())
        .await
        .unwrap();

    let sender_uri = UUri::try_from_parts("sender", 0x6201, 1, 0).unwrap();
    let sender = UPTransportVsomeip::new_with_transport_config(
        VsomeipApplicationConfig::new("notification_sender", 0x6201),
        sender_uri,
        &"receiver".to_string(),
        None,
        TransportConfig::new(encoding),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    sender
        .send(
            UMessageBuilder::notification(source, sink.clone())
                .build_with_payload("directed", encoding)
                .unwrap(),
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("directed notification should arrive")
        .expect("listener channel should remain open");
    assert_eq!(received.type_(), UMessageType::Notification);
    assert_eq!(received.sink(), Some(&sink));
    assert!(received.source().is_event());
    assert_eq!(received.source().authority_name(), "sender");
    assert_eq!(received.payload().as_deref(), Some(b"directed".as_slice()));
    assert_eq!(received.payload_encoding(), Some(encoding));

    receiver
        .unregister_listener(&source_filter, Some(&sink), listener)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn publish_without_subscriber_times_out_readiness_and_succeeds() {
    let _ = env_logger::builder().is_test(true).try_init();
    let encoding = PayloadEncoding::TEXT;
    let publisher = UPTransportVsomeip::new_with_transport_config(
        VsomeipApplicationConfig::new("publisher_without_subscriber", 0x6300),
        UUri::try_from_parts("publisher", 0x6300, 1, 0).unwrap(),
        &"remote".to_string(),
        None,
        TransportConfig::new(encoding),
    )
    .unwrap();
    let topic = UUri::try_from_parts("publisher", 0x6300, 1, 0x8001).unwrap();

    let start = tokio::time::Instant::now();
    publisher
        .send(
            UMessageBuilder::publish(topic)
                .build_with_payload("no subscriber", encoding)
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(start.elapsed() < Duration::from_secs(2));
}
