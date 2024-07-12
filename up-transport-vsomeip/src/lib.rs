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
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock as TokioRwLock;
use up_rust::{UCode, UListener, UMessage, UStatus, UUri, UUID};

mod determine_message_type;
mod extern_fn_registry;

pub use extern_fn_registry::print_extern_fn_registry_rwlock_times;

mod message_conversions;
mod rpc_correlation;
mod transport_inner;
use crate::extern_fn_registry::MockableExternFnRegistry;
use crate::listener_registry::ListenerRegistry;
use crate::rpc_correlation::RpcCorrelation2;
use crate::transport_inner::UPTransportVsomeipInnerHandle;
use crate::vsomeip_offered_requested::VsomeipOfferedRequested2;

mod vsomeip_config;
mod vsomeip_offered_requested;

pub(crate) mod listener_registry;
pub mod transport;

#[cfg(feature = "timing")]
mod timing_imports {
    pub use std::time::Duration;
    pub use tokio::sync::Mutex;
    pub use tokio::time::Instant;
}

#[cfg(feature = "timing")]
use timing_imports::*;

pub struct TimedRwLock<T> {
    inner: TokioRwLock<T>,
    #[cfg(feature = "timing")]
    read_durations: Arc<Mutex<Vec<Duration>>>,
    #[cfg(feature = "timing")]
    write_durations: Arc<Mutex<Vec<Duration>>>,
}

impl<T> TimedRwLock<T> {
    pub fn new(value: T) -> Self {
        Self {
            inner: TokioRwLock::new(value),
            #[cfg(feature = "timing")]
            read_durations: Arc::new(Mutex::new(Vec::new())),
            #[cfg(feature = "timing")]
            write_durations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn read(&self) -> tokio::sync::RwLockReadGuard<'_, T> {
        #[cfg(feature = "timing")]
        let start = Instant::now();

        let guard = self.inner.read().await;

        #[cfg(feature = "timing")]
        {
            let duration = start.elapsed();
            let mut read_durations = self.read_durations.lock().await;
            read_durations.push(duration);
        }

        guard
    }

    pub async fn write(&self) -> tokio::sync::RwLockWriteGuard<'_, T> {
        #[cfg(feature = "timing")]
        let start = Instant::now();

        let guard = self.inner.write().await;

        #[cfg(feature = "timing")]
        {
            let duration = start.elapsed();
            let mut write_durations = self.write_durations.lock().await;
            write_durations.push(duration);
        }

        guard
    }

    #[cfg(feature = "timing")]
    pub async fn read_durations(&self) -> Vec<Duration> {
        let read_durations = self.read_durations.lock().await;
        read_durations.clone()
    }

    #[cfg(feature = "timing")]
    pub async fn write_durations(&self) -> Vec<Duration> {
        let write_durations = self.write_durations.lock().await;
        write_durations.clone()
    }
}

impl<T> Deref for TimedRwLock<T> {
    type Target = TokioRwLock<T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for TimedRwLock<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

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

    async fn get_registry(&self) -> Arc<ListenerRegistry>;

    async fn get_extern_fn_registry(&self) -> Arc<dyn MockableExternFnRegistry>;

    async fn get_rpc_correlation(&self) -> Arc<RpcCorrelation2>;

    async fn get_vsomeip_offered_requested(&self) -> Arc<VsomeipOfferedRequested2>;
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

    pub async fn print_rwlock_times(&self) {
        self.transport_inner.print_rwlock_times().await;
    }
}
