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

use async_trait::async_trait;
use futures::executor;
use log::error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::mpsc::Sender;
use tokio::sync::{oneshot, Mutex, RwLock as TokioRwLock, RwLockReadGuard, RwLockWriteGuard};
use up_rust::{UCode, UListener, UMessage, UStatus, UUri, UUID};

mod determine_message_type;
mod extern_fn_registry;
mod message_conversions;
mod rpc_correlation;
mod transport_inner;
use crate::determine_message_type::RegistrationType;
use crate::extern_fn_registry::MockableExternFnRegistry;
use crate::listener_registry::ListenerRegistry;
use crate::rpc_correlation::RpcCorrelation2;
use crate::transport_inner::{TransportCommand, UPTransportVsomeipInnerHandle};
use crate::vsomeip_offered_requested::VsomeipOfferedRequested2;
use transport_inner::UPTransportVsomeipInnerEngine;
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;

mod vsomeip_config;
mod vsomeip_offered_requested;

pub(crate) mod listener_registry;
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

#[async_trait]
pub(crate) trait UPTransportVsomeipStorage: Send + Sync {
    fn get_local_authority(&self) -> AuthorityName;

    fn get_remote_authority(&self) -> AuthorityName;

    fn get_ue_id(&self) -> UeId;

    async fn get_registry(&self) -> Arc<TokioRwLock<ListenerRegistry>>;

    async fn get_extern_fn_registry(&self) -> Arc<TokioRwLock<Arc<dyn MockableExternFnRegistry>>>;

    async fn get_rpc_correlation(&self) -> Arc<TokioRwLock<RpcCorrelation2>>;

    async fn get_vsomeip_offered_requested(&self) -> Arc<TokioRwLock<VsomeipOfferedRequested2>>;
}

#[async_trait]
pub(crate) trait MockableUPTransportVsomeipInner {
    // the below are commands we can send to the inner engine
    fn get_storage(&self) -> Arc<dyn UPTransportVsomeipStorage>;

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus>;

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus>;

    async fn send(&self, message: UMessage) -> Result<(), UStatus>;
}

pub struct UPTransportVsomeip {
    transport_inner: Arc<UPTransportVsomeipInnerHandle>,
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
        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        let transport_inner = {
            if let Some(config_path) = config_path {
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
        };
        let transport_inner = Arc::new(transport_inner);
        Ok(Self { transport_inner })
    }
}
