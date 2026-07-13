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

use std::time::Duration;
use up_rust::{PayloadEncoding, UMessageBuilder, UPayloadFormat, UTransport, UUri};
use up_transport_vsomeip::{TransportConfig, UPTransportVsomeip};

#[tokio::test(flavor = "multi_thread")]
async fn first_publish_without_subscriber_remains_bounded_and_successful() {
    test_lib::before_test();
    let network = test_lib::VsomeipTestNetwork::new("publish_without_subscriber");
    let config = network.config("publisher_app", 0x0343);
    let topic = UUri::try_from_parts("foo", 10, 1, 0x8001).unwrap();
    let publisher = UPTransportVsomeip::new_with_config_and_transport_config(
        UUri::try_from_parts("foo", 10, 1, 0).unwrap(),
        &"me_authority".to_string(),
        config.path(),
        None,
        TransportConfig::new(PayloadEncoding::TEXT),
    )
    .expect("Unable to establish publisher");

    tokio::time::timeout(
        Duration::from_secs(2),
        publisher.send(
            UMessageBuilder::publish(topic)
                .build_with_payload(b"no_subscriber".to_vec(), UPayloadFormat::Text)
                .expect("failed to create publish UMessage"),
        ),
    )
    .await
    .expect("first publish exceeded its bounded readiness window")
    .expect("publishing without a subscriber must remain best effort");
}
