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

mod test_lib;

use log::{info, trace};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
use up_rust::{
    PayloadEncoding, UListener, UMessage, UMessageBuilder, UMessageType, UPayloadFormat,
    UTransport, UUri,
};
use up_transport_vsomeip::{TransportConfig, UPTransportVsomeip};

const TEST_DURATION: u64 = 2000;
const MAX_ITERATIONS: usize = 100;

struct NotifyeeListener {
    received_notifications: AtomicUsize,
    last_message: Mutex<Option<UMessage>>,
}

impl NotifyeeListener {
    fn new() -> Self {
        Self {
            received_notifications: AtomicUsize::new(0),
            last_message: Mutex::new(None),
        }
    }

    fn received_notifications(&self) -> usize {
        self.received_notifications.load(Ordering::SeqCst)
    }

    fn last_message(&self) -> Option<UMessage> {
        self.last_message.lock().expect("lock").clone()
    }
}

#[async_trait::async_trait]
impl UListener for NotifyeeListener {
    async fn on_receive(&self, msg: UMessage) {
        trace!("NotifyeeListener received:\n{msg:?}");
        self.received_notifications.fetch_add(1, Ordering::SeqCst);
        *self.last_message.lock().expect("lock") = Some(msg);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn notifier_notifyee() {
    test_lib::before_test();
    let network = test_lib::VsomeipTestNetwork::new("notifier_notifyee");

    let authority_name = "foo";
    let remote_authority = "me_authority".to_string();

    let notifier_app_id: u16 = 0x0343;
    let notifyee_ue_id = 20;
    let ue_version_major = 1;
    let notification_event_sentinel = 0x8000;

    let notifier_topic = UUri::try_from_parts(
        &remote_authority,
        u32::from(notifier_app_id),
        ue_version_major,
        notification_event_sentinel,
    )
    .unwrap();

    let notifyee_config = network.config("notifyee_app", 0x0344);
    let cfg = std::fs::read_to_string(notifyee_config.path()).expect("read notifyee cfg");
    let cfg = cfg.replace(
        r#""applications": [{ "name": "notifyee_app", "id": "0x0344" }]"#,
        r#""applications": [{ "name": "notifyee_app", "id": "0x0344" }, { "name": "notifier_app", "id": "0x0343" }]"#,
    );
    std::fs::write(notifyee_config.path(), cfg).expect("write notifyee cfg");

    let notifyee_uri =
        UUri::try_from_parts(authority_name, notifyee_ue_id, ue_version_major, 0).unwrap();

    let notifyee = UPTransportVsomeip::new_with_config_and_transport_config(
        notifyee_uri.clone(),
        &remote_authority,
        notifyee_config.path(),
        None,
        TransportConfig::new(PayloadEncoding::TEXT),
    )
    .expect("Unable to establish notifyee");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let notifyee_listener_check = Arc::new(NotifyeeListener::new());
    let notifyee_listener: Arc<dyn UListener> = notifyee_listener_check.clone();

    notifyee
        .register_listener(&notifier_topic, Some(&notifyee_uri), notifyee_listener)
        .await
        .expect("Unable to register notification listener");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let notifier_config = network.config("notifier_app", notifier_app_id);
    let notifier_own_uri = UUri::try_from_parts(
        authority_name,
        u32::from(notifier_app_id),
        ue_version_major,
        0,
    )
    .unwrap();
    let notifier = UPTransportVsomeip::new_with_config_and_transport_config(
        notifier_own_uri,
        &remote_authority,
        notifier_config.path(),
        None,
        TransportConfig::new(PayloadEncoding::TEXT),
    )
    .expect("Unable to establish notifier");

    tokio::time::sleep(Duration::from_millis(500)).await;

    notifier
        .send(
            UMessageBuilder::notification(notifier_topic.clone(), notifyee_uri.clone())
                .build_with_payload(b"warmup_notification".to_vec(), UPayloadFormat::Text)
                .expect("failed to create warm-up notification UMessage"),
        )
        .await
        .expect("failed to send warm-up notification UMessage");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let baseline_received = notifyee_listener_check.received_notifications();

    let duration = Duration::from_millis(TEST_DURATION);
    let start_time = Instant::now();
    let mut iterations = 0;

    while (Instant::now().duration_since(start_time) < duration) && (iterations < MAX_ITERATIONS) {
        let payload = format!("notification_message@i={iterations}").into_bytes();
        let notification_msg =
            UMessageBuilder::notification(notifier_topic.clone(), notifyee_uri.clone())
                .build_with_payload(payload, UPayloadFormat::Text)
                .expect("Unable to create Notification UMessage");

        trace!("Notification message we're about to send:\n{notification_msg:?}");

        notifier
            .send(notification_msg)
            .await
            .expect("Unable to send Notification UMessage");

        tokio::time::sleep(Duration::from_millis(20)).await;
        iterations += 1;
    }

    tokio::time::sleep(Duration::from_millis(1000)).await;

    let received = notifyee_listener_check.received_notifications();
    info!("notifications received: {received} (baseline {baseline_received}, sent {iterations})");
    assert!(
        received > baseline_received,
        "notifyee received no notifications beyond warm-up"
    );

    let last = notifyee_listener_check
        .last_message()
        .expect("at least one notification recorded");
    let attributes = last.attributes();
    assert_eq!(attributes.type_(), UMessageType::Notification);
    assert_eq!(attributes.source(), &notifier_topic);
    assert_eq!(attributes.sink(), Some(&notifyee_uri));
    assert_eq!(attributes.payload_format(), Some(UPayloadFormat::Text));
    assert!(last
        .payload()
        .expect("notification carries payload")
        .starts_with(b"notification_message@i="));
}
