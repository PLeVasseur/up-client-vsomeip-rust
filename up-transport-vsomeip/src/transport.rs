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
