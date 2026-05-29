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

use log::{info, trace};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use up_rust::{
    PayloadEncoding, UAttributes, UFrameMetadata, UMessageType, UOwnedFrame, UOwnedListener,
    UOwnedTransport, UUri, UUID,
};
use up_transport_vsomeip::UPTransportVsomeip;

const TEST_DURATION: u64 = 2000;
const MAX_ITERATIONS: usize = 100;

pub struct SubscriberListener {
    received_publish: AtomicUsize,
}
impl SubscriberListener {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            received_publish: AtomicUsize::new(0),
        }
    }

    pub fn received_publish(&self) -> usize {
        self.received_publish.load(Ordering::SeqCst)
    }
}
#[async_trait::async_trait]
impl UOwnedListener for SubscriberListener {
    async fn on_receive_owned(&self, frame: UOwnedFrame) {
        trace!("{:?}", frame);
        self.received_publish.fetch_add(1, Ordering::SeqCst);

        let Ok(payload_string) = std::str::from_utf8(frame.payload_bytes()) else {
            panic!("Unable to convert back to payload_string");
        };

        info!("We received payload_string: {payload_string}");
    }
}

fn publish_frame(topic: UUri, payload: Vec<u8>) -> UOwnedFrame {
    UOwnedFrame::try_with_payload(
        UFrameMetadata::try_new(
            UAttributes::try_new(UUID::build(), topic, None, UMessageType::Publish)
                .expect("valid publish attributes"),
            PayloadEncoding::from_content_type("text/plain"),
        )
        .expect("valid publish metadata"),
        payload,
    )
    .expect("valid publish frame")
}

pub async fn spawn_artifical_load(duration: Duration) {
    let start = Instant::now();
    while Instant::now().duration_since(start) < duration {
        let mut dummy = 0u64;
        for i in 0..500_000 {
            dummy = dummy.wrapping_add(i);
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn publisher_subscriber() {
    test_lib::before_test();
    let network = test_lib::VsomeipTestNetwork::new("publisher_subscriber");

    let authority_name = "foo";

    let ue_id = 10;
    let subscriber_ue_id = 20;
    let ue_version_major = 1;
    let resource_id = 0x8001;

    let publisher_topic =
        UUri::try_from_parts(authority_name, ue_id, ue_version_major, resource_id).unwrap();

    let subscriber_config = network.config("subscriber_app", 0x0344);
    let subscriber_uri =
        UUri::try_from_parts(authority_name, subscriber_ue_id, ue_version_major, 0).unwrap();

    let subscriber_res = UPTransportVsomeip::new_with_config(
        subscriber_uri,
        &"me_authority".to_string(),
        subscriber_config.path(),
        None,
    );

    let Ok(subscriber) = subscriber_res else {
        panic!("Unable to establish subscriber");
    };

    tokio::time::sleep(Duration::from_millis(500)).await;

    let subscriber_listener_check = Arc::new(SubscriberListener::new());
    let subscriber_listener: Arc<dyn UOwnedListener> = subscriber_listener_check.clone();

    let reg_res = subscriber
        .register_owned_listener(&publisher_topic, None, subscriber_listener)
        .await;

    if let Err(err) = reg_res {
        panic!("Unable to register: {:?}", err);
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    let publisher_config = network.config("publisher_app", 0x0343);
    let publisher_uri = UUri::try_from_parts(authority_name, ue_id, 1, 0).unwrap();
    let publisher_res = UPTransportVsomeip::new_with_config(
        publisher_uri,
        &"me_authority".to_string(),
        publisher_config.path(),
        None,
    );

    let Ok(publisher) = publisher_res else {
        panic!("Unable to establish publisher");
    };

    let load_handle = tokio::spawn(async {
        spawn_artifical_load(Duration::from_secs(2)).await;
    });

    tokio::time::sleep(Duration::from_millis(500)).await;

    publisher
        .send_owned(publish_frame(
            publisher_topic.clone(),
            b"warmup_publish".to_vec(),
        ))
        .await
        .expect("failed to send warm-up publish frame");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let baseline_received = subscriber_listener_check.received_publish();

    // Track the start time and set the duration for the loop
    let duration = Duration::from_millis(TEST_DURATION);
    let start_time = Instant::now();
    let mut iterations = 0;

    // limit with iterations to ensure socket transactions can complete during test
    while (Instant::now().duration_since(start_time) < duration) && (iterations < MAX_ITERATIONS) {
        let publish_payload_string = format!("publish_message@i={iterations}");
        let publish_payload = publish_payload_string.into_bytes();

        let publish_msg = publish_frame(publisher_topic.clone(), publish_payload);

        trace!("Publish message we're about to send:\n{publish_msg:?}");

        let send_res = publisher.send_owned(publish_msg).await;

        if let Err(err) = send_res {
            panic!("Unable to send Publish frame: {:?}", err);
        }

        tokio::time::sleep(Duration::from_millis(20)).await;

        iterations += 1;
    }
    println!("iterations: {}", iterations);

    let mut attempts = 0;
    const MAX_WAIT_ATTEMPTS: usize = 10;
    while subscriber_listener_check.received_publish() < baseline_received + iterations
        && attempts < MAX_WAIT_ATTEMPTS
    {
        tokio::time::sleep(Duration::from_millis(200)).await;
        attempts += 1;
    }

    println!(
        "subscriber_listener_check.received_publish(): {}",
        subscriber_listener_check.received_publish()
    );

    let _ = load_handle.await;

    assert_eq!(
        iterations,
        subscriber_listener_check
            .received_publish()
            .saturating_sub(baseline_received),
        "The number of messages received by the subscriber does not match the number sent."
    );
}
