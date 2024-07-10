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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use up_rust::{UCode, UListener, UStatus, UUri, UUID};

mod determine_message_type;
mod message_conversions;
mod registry;
mod rpc_correlation;
mod transport_inner;
use transport_inner::UPTransportVsomeipInner;
mod vsomeip_config;
mod vsomeip_offered_requested;

pub mod transport;

// TODO: use function from up-rust when merged
pub(crate) fn any_uuri() -> UUri {
    UUri {
        authority_name: "*".to_string(),
        ue_id: 0x0000_FFFF,     // any instance, any service
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
        ..Default::default()
    }
}

// TODO: upstream into up-rust
pub(crate) fn any_uuri_fixed_authority_id(authority_name: &AuthorityName, ue_id: UeId) -> UUri {
    UUri {
        authority_name: authority_name.to_string(),
        ue_id: ue_id as u32,
        ue_version_major: 0xFF, // any
        resource_id: 0xFFFF,    // any
        ..Default::default()
    }
}

// TODO: upstream this into up-rust
pub(crate) fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

// TODO: upstream this into up-rust
pub(crate) fn split_u32_to_u8(value: u32) -> (u8, u8, u8, u8) {
    let byte1 = (value >> 24) as u8;
    let byte2 = (value >> 16 & 0xFF) as u8;
    let byte3 = (value >> 8 & 0xFF) as u8;
    let byte4 = (value & 0xFF) as u8;
    (byte1, byte2, byte3, byte4)
}

// TODO: upstream this into up-rust
pub(crate) fn create_request_id(client_id: ClientId, session_id: SessionId) -> RequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}

type ApplicationName = String;
type AuthorityName = String;
type UeId = u16;
type ClientId = u16;
type ReqId = UUID;
type SessionId = u16;
type RequestId = u32;
type EventId = u16;
type ServiceId = u16;
type InstanceId = u16;
type MethodId = u16;

pub struct UPTransportVsomeip {
    instance_id: uuid::Uuid,
    inner_transport: UPTransportVsomeipInner,
    authority_name: AuthorityName,
    remote_authority_name: AuthorityName,
    ue_id: UeId,
    config_path: Option<PathBuf>,
    // if this is not None, indicates that we are in a dedicated point-to-point mode
    point_to_point_listener: RwLock<Option<Arc<dyn UListener>>>,
}

impl UPTransportVsomeip {
    pub fn new_with_config(
        authority_name: &AuthorityName,
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
            authority_name,
            remote_authority_name,
            ue_id,
            Some(config_path),
        )
    }

    pub fn new(
        authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
    ) -> Result<Self, UStatus> {
        Self::new_internal(authority_name, remote_authority_name, ue_id, None)
    }

    fn new_internal(
        authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: Option<&Path>,
    ) -> Result<Self, UStatus> {
        let inner_transport = UPTransportVsomeipInner::new(config_path);
        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        Ok(Self {
            instance_id: uuid::Uuid::new_v4(),
            inner_transport,
            authority_name: authority_name.to_string(),
            remote_authority_name: remote_authority_name.to_string(),
            ue_id,
            point_to_point_listener: None.into(),
            config_path,
        })
    }
}
