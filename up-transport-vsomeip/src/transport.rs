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

use crate::MockableUPTransportVsomeipInner;
use crate::UPTransportVsomeip;
use async_trait::async_trait;
use std::sync::Arc;
use up_rust::{UCode, UListener, UMessage, UStatus, UTransport, UUri};

// TODO: Decide whether to keep
// const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER: &str = "register_listener";

#[async_trait]
impl UTransport for UPTransportVsomeip {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        self.transport_inner.send(message).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.transport_inner
            .register_listener(source_filter, sink_filter, listener)
            .await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.transport_inner
            .unregister_listener(source_filter, sink_filter, listener)
            .await
    }

    async fn receive(
        &self,
        _source_filter: &UUri,
        _sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        Err(UStatus::fail_with_code(
            UCode::UNIMPLEMENTED,
            "This method is not implemented for vsomeip. Use register_listener instead.",
        ))
    }
}

// TODO: Update this
impl Drop for UPTransportVsomeip {
    fn drop(&mut self) {
        // let instance_id = self.instance_id.clone();
        // error!(
        //     "dropping UPTransportVsomeip with instance_id: {}",
        //     instance_id.hyphenated().to_string()
        // );
        // let transport_command_sender = self.inner_transport.transport_command_sender.clone();
        //
        // // Create a oneshot channel to wait for task completion
        // let (tx, rx) = oneshot::channel();
        //
        // // Get the handle of the current runtime
        // let handle = Handle::current();
        //
        // std::thread::spawn(move || {
        //     handle.block_on(async move {
        //         Self::delete_registry_items_internal(instance_id, transport_command_sender).await;
        //         // Notify that the task is complete
        //         let _ = tx.send(());
        //     });
        // });
        //
        // // Wait for the task to complete
        // let _ = executor::block_on(rx);
    }
}
