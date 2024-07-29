/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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

use crate::transport_inner::transport_inner_handle::UPTransportVsomeipInnerHandle;
use log::trace;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use up_rust::{UCode, UStatus, UUID};

mod determine_message_type;
mod message_conversions;
mod storage;
mod transport;
mod transport_inner;
mod utils;
mod vsomeip_config;

/// A [up_rust::UUri::authority_name]
pub type AuthorityName = String;
/// A [up_rust::UUri::ue_id]
pub type UeId = u16;
/// A [vsomeip_sys::vsomeip::application]'s numeric identifier
pub type ClientId = u16;
/// A [vsomeip_sys::vsomeip::application]'s string-form identifier
pub type ApplicationName = String;

/// A [up_rust::UAttributes::reqid]
pub type UProtocolReqId = UUID;
/// A request ID used with vsomeip. See [vsomeip_sys::vsomeip::request_t]
pub type SomeIpRequestId = u32;
/// A session ID used with vsomeip. See [vsomeip_sys::vsomeip::session_t]
pub type SessionId = u16;

/// A service ID used with vsomeip. See [vsomeip_sys::vsomeip::service_t]
pub type ServiceId = u16;
/// An instance ID used with vsomeip. See [vsomeip_sys::vsomeip::instance_t]
pub type InstanceId = u16;
/// A method ID used with vsomeip. See [vsomeip_sys::vsomeip::method_t]
pub type MethodId = u16;
/// An event ID used with vsomeip. See [vsomeip_sys::vsomeip::event_t]
pub type EventId = u16;
/// Represents the id of an extern "C" fn used with which to register with vsomeip to listen for messages
type MessageHandlerId = usize;

/// UTransport implementation over top of the C++ vsomeip library
///
/// We hold a transport_inner internally which does the nitty-gritty
/// implementation of the transport
///
/// We do so in order to separate the "handle" to the inner transport
/// and the "engine" of the innner transport to allow mocking of them.
pub struct UPTransportVsomeip {
    /// Internally held inner implementation
    transport_inner: Arc<UPTransportVsomeipInnerHandle>,
}

impl UPTransportVsomeip {
    /// Creates a UPTransportVsomeip based on a path provided to a vsomeip configuration JSON file
    ///
    /// # Parameters
    ///
    /// * `local_authority_name` - authority_name of the host device
    /// * `remote_authority_name` - authority_name to attach for messages originating from SOME/IP network.
    ///                             Should be set to `IP:port` of the endpoint mDevice
    /// * `ue_id` - the ue_id of the uEntity
    /// * `config_path` - path to a JSON vsomeip configuration file
    ///
    /// Further details on vsomeip configuration files can be found in the COVESA [vsomeip repo](https://github.com/COVESA/vsomeip)
    pub fn new_with_config(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: &Path,
    ) -> Result<Self, UStatus> {
        if !config_path.exists() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Configuration file not found at: {:?}", config_path),
            ));
        }
        Self::new_internal(
            local_authority_name,
            remote_authority_name,
            ue_id,
            Some(config_path),
        )
    }

    /// Creates a UPTransportVsomeip
    ///
    /// # Parameters
    ///
    /// * `local_authority_name` - authority_name of the host device
    /// * `remote_authority_name` - authority_name to attach for messages originating from SOME/IP network
    ///                             Should be set to `IP:port` of the endpoint mDevice
    /// * `ue_id` - the ue_id of the uEntity
    pub fn new(
        authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
    ) -> Result<Self, UStatus> {
        Self::new_internal(authority_name, remote_authority_name, ue_id, None)
    }

    /// Creates a UPTransportVsomeip whether a vsomeip config file was provided or not
    fn new_internal(
        authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: Option<&Path>,
    ) -> Result<Self, UStatus> {
        let optional_config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        let transport_inner: Arc<UPTransportVsomeipInnerHandle> = Arc::new({
            if let Some(config_path) = optional_config_path {
                let config_path = config_path.as_path();
                let transport_inner_res = UPTransportVsomeipInnerHandle::new_with_config(
                    authority_name,
                    remote_authority_name,
                    ue_id,
                    config_path,
                );
                match transport_inner_res {
                    Ok(transport_inner) => transport_inner,
                    Err(err) => {
                        return Err(err);
                    }
                }
            } else {
                let transport_inner_res = UPTransportVsomeipInnerHandle::new(
                    authority_name,
                    remote_authority_name,
                    ue_id,
                );
                match transport_inner_res {
                    Ok(transport_inner) => transport_inner,
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
        });
        Ok(Self { transport_inner })
    }
}

impl Drop for UPTransportVsomeip {
    fn drop(&mut self) {
        trace!("Running Drop for UPTransportVsomeip");
    }
}
