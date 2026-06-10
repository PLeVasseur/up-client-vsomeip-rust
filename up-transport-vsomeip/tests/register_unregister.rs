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

pub mod test_lib;
use up_transport_vsomeip::UPTransportVsomeip;

#[cfg(test)]
mod tests {
    use crate::test_lib::PrintingListener;
    use crate::{test_lib, UPTransportVsomeip};
    use log::error;
    use std::sync::Arc;
    use std::time::Duration;
    use up_rust::{UListener, UTransport, UUri};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_registering_unregistering_publish() {
        test_lib::before_test();

        let network = test_lib::VsomeipTestNetwork::new("register_unregister_publish");
        let config = network.config("reg_unreg_publish_test", 0x0123);
        let client_uri = UUri::try_from_parts("foo", 10, 1, 0).unwrap();
        let client = UPTransportVsomeip::new_with_config(
            client_uri,
            &"me_authority".to_string(),
            config.path(),
            None,
        )
        .unwrap();

        let source_filter = UUri::try_from_parts("foo", 0x01, 1, 0x8001).unwrap();
        let printing_helper: Arc<dyn UListener> = Arc::new(PrintingListener);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let reg_res = client
            .register_listener(&source_filter, None, printing_helper.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(200)).await;

        if let Err(ref err) = reg_res {
            error!("Issue registering listener for publishes: {:?}", err);
        }

        assert!(reg_res.is_ok());

        let unreg_res = client
            .unregister_listener(&source_filter, None, printing_helper)
            .await;

        if let Err(ref err) = unreg_res {
            error!("Issue unregistering listener for publishes: {:?}", err);
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(unreg_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_registering_unregistering_request() {
        test_lib::before_test();

        let network = test_lib::VsomeipTestNetwork::new("register_unregister_request");
        let config = network.config("reg_unreg_request_test", 0x0124);
        let client_uri = UUri::try_from_parts("foo", 10, 1, 0).unwrap();
        let client = UPTransportVsomeip::new_with_config(
            client_uri,
            &"me_authority".to_string(),
            config.path(),
            None,
        )
        .unwrap();

        let source_filter = UUri::try_from_parts("foo", 0x01, 1, 10).unwrap();
        let sink_filter = UUri::try_from_parts("bar", 0x02, 1, 20).unwrap();
        let printing_helper: Arc<dyn UListener> = Arc::new(PrintingListener);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let reg_res = client
            .register_listener(&source_filter, Some(&sink_filter), printing_helper.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = reg_res {
            error!("Issue registering listener for publishes: {:?}", err);
        }

        assert!(reg_res.is_ok());

        let unreg_res = client
            .unregister_listener(&source_filter, Some(&sink_filter), printing_helper)
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = unreg_res {
            error!("Issue unregistering listener for publishes: {:?}", err);
        }

        assert!(unreg_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_registering_unregistering_response() {
        test_lib::before_test();

        let network = test_lib::VsomeipTestNetwork::new("register_unregister_response");
        let config = network.config("reg_unreg_response_test", 0x0125);
        let client_uri = UUri::try_from_parts("foo", 10, 1, 0).unwrap();
        let client = UPTransportVsomeip::new_with_config(
            client_uri,
            &"me_authority".to_string(),
            config.path(),
            None,
        )
        .unwrap();

        let source_filter = UUri::try_from_parts("foo", 0x01, 1, 10).unwrap();
        let sink_filter = UUri::try_from_parts("bar", 0x02, 1, 0).unwrap();
        let printing_helper: Arc<dyn UListener> = Arc::new(PrintingListener);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let reg_res = client
            .register_listener(&source_filter, Some(&sink_filter), printing_helper.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = reg_res {
            error!("Issue registering listener for publishes: {:?}", err);
        }

        assert!(reg_res.is_ok());

        tokio::time::sleep(Duration::from_millis(100)).await;

        let unreg_res = client
            .unregister_listener(&source_filter, Some(&sink_filter), printing_helper)
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = unreg_res {
            error!("Issue unregistering listener for publishes: {:?}", err);
        }

        assert!(unreg_res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_registering_unregistering_all_point_to_point() {
        test_lib::before_test();

        let network = test_lib::VsomeipTestNetwork::new("register_unregister_point_to_point");
        let config = network.config_with_services("2345_app", 0x2345, &[(0x2345, 0x0001)]);
        let client_uri = UUri::try_from_parts("foo", 10, 1, 0).unwrap();
        let client = UPTransportVsomeip::new_with_config(
            client_uri,
            &"me_authority".to_string(),
            config.path(),
            None,
        )
        .unwrap();

        let source_filter = UUri::any();
        let sink_filter = UUri::try_from_parts("me_authority", 0x0000_FFFF, 0xFF, 0xFFFF).unwrap();
        let printing_helper: Arc<dyn UListener> = Arc::new(PrintingListener);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let reg_res = client
            .register_listener(&source_filter, Some(&sink_filter), printing_helper.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = reg_res {
            error!("Issue registering listener for publishes: {:?}", err);
        }

        assert!(reg_res.is_ok());

        tokio::time::sleep(Duration::from_millis(100)).await;

        let unreg_res = client
            .unregister_listener(&source_filter, Some(&sink_filter), printing_helper)
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        if let Err(ref err) = unreg_res {
            error!("Issue unregistering listener for publishes: {:?}", err);
        }

        assert!(unreg_res.is_ok());
    }
}
