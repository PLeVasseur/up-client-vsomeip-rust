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

use crate::storage::UPTransportVsomeipStorage;
use async_trait::async_trait;
use std::sync::Arc;
use up_rust::{UListener, UMessage, UStatus, UUri};

pub mod transport_inner_engine;
pub mod transport_inner_handle;

pub const UP_CLIENT_VSOMEIP_TAG: &str = "UPClientVsomeipInner";
pub const UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP: &str = "app_event_loop";
pub const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL: &str = "register_listener_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL: &str = "send_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL: &str =
    "initialize_new_app_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_START_APP: &str = "start_app";
pub const UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP: &str = "stop_app";

const INTERNAL_FUNCTION_TIMEOUT: u64 = 3;
/// Trait to make testing of [crate::UPTransportVsomeip] more decoupled and simpler.
#[async_trait]
pub(crate) trait MockableUPTransportVsomeipInner: Sync + Send {
    /// Returns us a [UPTransportVsomeipStorage] trait object with wich to interact with
    /// the internal state of the transport
    fn get_storage(&self) -> Arc<dyn UPTransportVsomeipStorage>;

    /// Holds internal implementation of [up_rust::UTransport] for vsomeip
    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus>;

    /// Holds internal implementation of [up_rust::UTransport] for vsomeip
    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus>;

    /// Holds internal implementation of [up_rust::UTransport] for vsomeip
    async fn send(&self, message: UMessage) -> Result<(), UStatus>;
    /// Print sync primitive lock wait times
    async fn print_rwlock_times(&self);
}
