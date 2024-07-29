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

pub mod application_registry;
pub mod application_state_availability_handler_registry;
pub mod message_handler_registry;
pub mod rpc_correlation;
pub mod vsomeip_offered_requested;

use crate::storage::{
    application_registry::ApplicationRegistry,
    application_state_availability_handler_registry::{
        ApplicationStateAvailabilityHandlerExternFnRegistry,
        ApplicationStateAvailabilityHandlerRegistry,
    },
    message_handler_registry::MessageHandlerRegistry,
    rpc_correlation::RpcCorrelation,
    vsomeip_offered_requested::VsomeipOfferedRequested,
};
use crate::{AuthorityName, UeId};
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct UPTransportVsomeipStorage {
    ue_id: UeId,
    local_authority: AuthorityName,
    remote_authority: AuthorityName,
    runtime_handle: Handle,
    message_handler_registry: Arc<MessageHandlerRegistry>,
    application_state_handler_registry: Arc<dyn ApplicationStateAvailabilityHandlerRegistry>,
    application_registry: Arc<ApplicationRegistry>,
    rpc_correlation: Arc<RpcCorrelation>,
    vsomeip_offered_requested: Arc<VsomeipOfferedRequested>,
}

impl UPTransportVsomeipStorage {
    pub fn new(
        local_authority: AuthorityName,
        remote_authority: AuthorityName,
        ue_id: UeId,
        runtime_handle: Handle,
    ) -> Self {
        let application_state_handler_registry =
            ApplicationStateAvailabilityHandlerExternFnRegistry::new_trait_obj();

        Self {
            ue_id,
            local_authority,
            remote_authority,
            runtime_handle,
            message_handler_registry: Arc::new(MessageHandlerRegistry::new()),
            application_state_handler_registry,
            application_registry: Arc::new(ApplicationRegistry::new()),
            rpc_correlation: Arc::new(RpcCorrelation::new()),
            vsomeip_offered_requested: Arc::new(VsomeipOfferedRequested::new()),
        }
    }

    pub fn get_runtime_handle(&self) -> Handle {
        self.runtime_handle.clone()
    }
    pub fn get_local_authority(&self) -> AuthorityName {
        self.local_authority.clone()
    }

    pub fn get_remote_authority(&self) -> AuthorityName {
        self.remote_authority.clone()
    }

    pub fn get_ue_id(&self) -> UeId {
        self.ue_id
    }

    pub fn get_application_registry(&self) -> Arc<ApplicationRegistry> {
        self.application_registry.clone()
    }

    pub fn get_message_handler_registry(&self) -> Arc<MessageHandlerRegistry> {
        self.message_handler_registry.clone()
    }

    pub fn get_application_state_handler_registry(
        &self,
    ) -> Arc<dyn ApplicationStateAvailabilityHandlerRegistry> {
        self.application_state_handler_registry.clone()
    }

    pub fn get_rpc_correlation(&self) -> Arc<RpcCorrelation> {
        self.rpc_correlation.clone()
    }

    pub fn get_vsomeip_offered_requested(&self) -> Arc<VsomeipOfferedRequested> {
        self.vsomeip_offered_requested.clone()
    }
}
