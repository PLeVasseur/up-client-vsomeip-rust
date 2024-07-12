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

use crate::message_conversions::convert_vsomeip_msg_to_umsg;
use crate::TimedRwLock;
use crate::UPTransportVsomeipStorage;
use async_trait::async_trait;
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use log::{error, trace, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{mpsc, Arc, Weak};
use tokio::runtime::Runtime;
use tokio::task::LocalSet;
use tokio::time::Instant;
use up_rust::UListener;
use up_rust::{UCode, UMessage, UStatus};
use vsomeip_proc_macro::generate_message_handler_extern_c_fns;
use vsomeip_sys::glue::{make_application_wrapper, make_message_wrapper, make_runtime_wrapper};
use vsomeip_sys::safe_glue::get_pinned_runtime;
use vsomeip_sys::vsomeip;

const THREAD_NUM: usize = 10;

// Create a separate tokio Runtime for running the callback
lazy_static! {
    static ref CB_RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(THREAD_NUM)
        .enable_all()
        .build()
        .expect("Unable to create callback runtime");
}

static RUNTIME: Lazy<Arc<Runtime>> =
    Lazy::new(|| Arc::new(Runtime::new().expect("Failed to create Tokio runtime")));

fn get_runtime() -> Arc<Runtime> {
    Arc::clone(&RUNTIME)
}

generate_message_handler_extern_c_fns!(10000);

lazy_static! {
    static ref LISTENER_ID_TRANSPORT_SHIM: TimedRwLock<HashMap<usize, Weak<dyn UPTransportVsomeipStorage + Send + Sync>>> =
        TimedRwLock::new(HashMap::new());
}

struct ProcMacroTransportStorage;

impl ProcMacroTransportStorage {
    async fn get_listener_id_transport(
        listener_id: usize,
    ) -> Option<Arc<dyn UPTransportVsomeipStorage + Send + Sync>> {
        let listener_id_transport_shim = LISTENER_ID_TRANSPORT_SHIM.read().await;
        let transport = listener_id_transport_shim.get(&listener_id)?;

        transport.upgrade()
    }
}

#[async_trait]
pub(crate) trait MockableExternFnRegistry: Send + Sync {
    async fn get_extern_fn(
        &self,
        listener_id: usize,
    ) -> extern "C" fn(&SharedPtr<vsomeip::message>) {
        get_extern_fn(listener_id)
    }
    async fn insert_listener_id_transport(
        &self,
        listener_id: usize,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus>;
    async fn remove_listener_id_transport(&self, listener_id: usize) -> Result<(), UStatus>;
    async fn free_listener_id(&self, listener_id: usize) -> Result<(), UStatus>;
    async fn find_available_listener_id(&self) -> Result<usize, UStatus>;
}

pub(crate) struct ExternFnRegistry;

#[async_trait]
impl MockableExternFnRegistry for ExternFnRegistry {
    async fn insert_listener_id_transport(
        &self,
        listener_id: usize,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus> {
        let mut listener_id_transport_shim = LISTENER_ID_TRANSPORT_SHIM.write().await;
        if let std::collections::hash_map::Entry::Vacant(e) =
            listener_id_transport_shim.entry(listener_id)
        {
            e.insert(Arc::downgrade(&transport));
        } else {
            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "LISTENER_ID_TRANSPORT_MAPPING already contains listener_id: {listener_id}"
                ),
            ));
        }

        Ok(())
    }

    async fn remove_listener_id_transport(&self, listener_id: usize) -> Result<(), UStatus> {
        let mut listener_id_transport_shim = LISTENER_ID_TRANSPORT_SHIM.write().await;
        if listener_id_transport_shim.contains_key(&listener_id) {
            listener_id_transport_shim.remove(&listener_id);
        } else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "LISTENER_ID_TRANSPORT_MAPPING does not contain listener_id: {listener_id}"
                ),
            ));
        }

        Ok(())
    }

    async fn free_listener_id(&self, listener_id: usize) -> Result<(), UStatus> {
        let mut free_ids = FREE_LISTENER_IDS.write().await;
        free_ids.insert(listener_id);

        trace!("free_listener_id: {listener_id}");

        Ok(())
    }

    async fn find_available_listener_id(&self) -> Result<usize, UStatus> {
        let mut free_ids = FREE_LISTENER_IDS.write().await;
        if let Some(&id) = free_ids.iter().next() {
            free_ids.remove(&id);
            trace!("find_available_listener_id: {id}");
            Ok(id)
        } else {
            Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "No more extern C fns available",
            ))
        }
    }
}

pub async fn print_extern_fn_registry_rwlock_times() {
    #[cfg(feature = "timing")]
    {
        println!("FREE_LISTENER_IDS:");
        println!(
            "reads: {:?}",
            crate::extern_fn_registry::FREE_LISTENER_IDS
                .read_durations()
                .await
        );
        println!(
            "writes: {:?}",
            crate::extern_fn_registry::FREE_LISTENER_IDS
                .write_durations()
                .await
        );

        println!("LISTENER_ID_TRANSPORT_SHIM:");
        println!(
            "reads: {:?}",
            crate::extern_fn_registry::LISTENER_ID_TRANSPORT_SHIM
                .read_durations()
                .await
        );
        println!(
            "writes: {:?}",
            crate::extern_fn_registry::LISTENER_ID_TRANSPORT_SHIM
                .write_durations()
                .await
        );
    }
}

impl ExternFnRegistry {
    pub fn new_trait_obj() -> Arc<dyn MockableExternFnRegistry> {
        Arc::new(ExternFnRegistry)
    }
}

// TODO: Add unit tests
