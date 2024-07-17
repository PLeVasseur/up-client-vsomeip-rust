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

pub mod application_state_handler_registry;
pub mod listener_registry;
pub mod message_handler_registry;
pub mod rpc_correlation;
pub mod vsomeip_offered_requested;

use crate::storage::{
    application_state_handler_registry::{
        ApplicationStateAvailabilityRegistry, MockableApplicationAvailableRegistry,
    },
    listener_registry::ListenerRegistry,
    message_handler_registry::{MessageHandlerRegistry, MockableMessageHandlerRegistry},
    rpc_correlation::RpcCorrelation,
    vsomeip_offered_requested::VsomeipOfferedRequested,
};
use crate::{AuthorityName, UeId};
use std::sync::Arc;

/// Trait to make testing of [crate::UPTransportVsomeip] more decoupled and simpler.
///
/// Holds all state necessary for the functioning of a [crate::transport_inner::MockableUPTransportVsomeipInner]
pub trait UPTransportVsomeipStorage: Send + Sync {
    /// Returns the [up_rust::UUri::authority_name] local authority of this device
    fn get_local_authority(&self) -> AuthorityName;

    /// Returns the [up_rust::UUri::authority_name] remote authority which should be associated to
    /// messages coming from the SOME/IP network
    fn get_remote_authority(&self) -> AuthorityName;

    /// Returns this uEntity's [up_rust::UUri::ue_id]
    fn get_ue_id(&self) -> UeId;

    /// Returns [ListenerRegistry] for manipulating the state of listeners
    fn get_listener_registry(&self) -> Arc<ListenerRegistry>;

    /// Returns [MockableMessageHandlerRegistry] for manipulating the state of extern "C" fn
    /// registry that is needed to create callbacks to give to vsomeip library
    fn get_message_handler_registry(&self) -> Arc<dyn MockableMessageHandlerRegistry>;

    fn get_application_state_handler_registry(
        &self,
    ) -> Arc<dyn MockableApplicationAvailableRegistry>;

    /// Returns [RpcCorrelation] for manipulating the state regarding RPC Request to Response
    /// flows
    fn get_rpc_correlation(&self) -> Arc<RpcCorrelation>;

    /// Returns [VsomeipOfferedRequested] for manipulating the state regarding which
    /// SOME/IP services and events have been offered and requested
    fn get_vsomeip_offered_requested(&self) -> Arc<VsomeipOfferedRequested>;
}

pub struct UPTransportVsomeipInnerHandleStorage {
    ue_id: UeId,
    local_authority: AuthorityName,
    remote_authority: AuthorityName,
    message_handler_registry: Arc<dyn MockableMessageHandlerRegistry>,
    application_state_handler_registry: Arc<dyn MockableApplicationAvailableRegistry>,
    listener_registry: Arc<ListenerRegistry>,
    rpc_correlation: Arc<RpcCorrelation>,
    vsomeip_offered_requested: Arc<VsomeipOfferedRequested>,
}

impl UPTransportVsomeipInnerHandleStorage {
    pub fn new(
        local_authority: AuthorityName,
        remote_authority: AuthorityName,
        ue_id: UeId,
    ) -> Self {
        let message_handler_registry = MessageHandlerRegistry::new_trait_obj();
        let application_state_handler_registry =
            ApplicationStateAvailabilityRegistry::new_trait_obj();

        Self {
            ue_id,
            local_authority,
            remote_authority,
            message_handler_registry,
            application_state_handler_registry,
            listener_registry: Arc::new(ListenerRegistry::new()),
            rpc_correlation: Arc::new(RpcCorrelation::new()),
            vsomeip_offered_requested: Arc::new(VsomeipOfferedRequested::new()),
        }
    }
}

impl UPTransportVsomeipStorage for UPTransportVsomeipInnerHandleStorage {
    fn get_local_authority(&self) -> AuthorityName {
        self.local_authority.clone()
    }

    fn get_remote_authority(&self) -> AuthorityName {
        self.remote_authority.clone()
    }

    fn get_ue_id(&self) -> UeId {
        self.ue_id
    }

    fn get_listener_registry(&self) -> Arc<ListenerRegistry> {
        self.listener_registry.clone()
    }

    fn get_message_handler_registry(&self) -> Arc<dyn MockableMessageHandlerRegistry> {
        self.message_handler_registry.clone()
    }

    fn get_application_state_handler_registry(
        &self,
    ) -> Arc<dyn MockableApplicationAvailableRegistry> {
        self.application_state_handler_registry.clone()
    }

    fn get_rpc_correlation(&self) -> Arc<RpcCorrelation> {
        self.rpc_correlation.clone()
    }

    fn get_vsomeip_offered_requested(&self) -> Arc<VsomeipOfferedRequested> {
        self.vsomeip_offered_requested.clone()
    }
}
