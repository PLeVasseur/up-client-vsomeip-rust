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

use crate::message_conversions::VsomeipMessageToUMessage;
use crate::storage::UPTransportVsomeipStorage;
use crate::utils::TimedStdRwLock;
use crate::ListenerId;
use async_trait::async_trait;
use cxx::SharedPtr;
use lazy_static::lazy_static;
use log::{error, trace, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{mpsc, Arc, Weak};
use tokio::runtime::Runtime;
use tokio::task::LocalSet;
use up_rust::UListener;
use up_rust::{UCode, UMessage, UStatus};
use vsomeip_proc_macro::generate_message_handler_extern_c_fns;
use vsomeip_sys::glue::{make_message_wrapper, MessageHandlerFnPtr};
use vsomeip_sys::vsomeip;

const THREAD_NUM: usize = 10;

lazy_static! {
    /// A [tokio::runtime::Runtime] onto which to run [up_rust::UListener] within the context of
    /// the callbacks registered with vsomeip when a message is received
    static ref CB_RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(THREAD_NUM)
        .enable_all()
        .build()
        .expect("Unable to create callback runtime");
}

static RUNTIME: Lazy<Arc<Runtime>> =
    Lazy::new(|| Arc::new(Runtime::new().expect("Failed to create Tokio runtime")));

/// Stand-alone [tokio::runtime::Runtime] used to run async code within single thread context
/// within the callback registered with vsomeip
fn get_runtime() -> Arc<Runtime> {
    Arc::clone(&RUNTIME)
}

generate_message_handler_extern_c_fns!(10000);

type ListenerIdToTransportStorage =
    HashMap<ListenerId, Weak<dyn UPTransportVsomeipStorage + Send + Sync>>;
lazy_static! {
    /// A mapping from extern "C" fn [ListenerId] onto [std::sync::Weak] references to [UPTransportVsomeipStorage]
    ///
    /// Used within the context of the proc macro crate (vsomeip-proc-macro) generated [call_shared_extern_fn]
    /// to obtain the state of the transport needed to perform ingestion of vsomeip messages from
    /// within callback functions registered with vsomeip
    static ref LISTENER_ID_TO_TRANSPORT_STORAGE: TimedStdRwLock<ListenerIdToTransportStorage> =
        TimedStdRwLock::new(HashMap::new());
}

/// A facade struct from which the proc macro crate (vsomeip-proc-macro) generated `call_shared_extern_fn`
/// can access state related to the transport
struct ProcMacroMessageHandlerAccess;

impl ProcMacroMessageHandlerAccess {
    /// Gets a trait object holding transport storage
    ///
    /// # Parameters
    ///
    /// * `listener_id` - The ListenerId
    fn get_listener_id_transport(
        listener_id: ListenerId,
    ) -> Option<Arc<dyn UPTransportVsomeipStorage + Send + Sync>> {
        let listener_id_transport_shim = LISTENER_ID_TO_TRANSPORT_STORAGE.read();
        let transport = listener_id_transport_shim.get(&listener_id)?;

        transport.upgrade()
    }
}

/// A trait to make mocking the registry of extern "C" fns easier
///
/// Provides functions for manipulating the state of registry of extern "C" fns
#[async_trait]
pub trait MockableMessageHandlerRegistry: Send + Sync {
    /// Get the statically allocated extern "C" fn associated with this [ListenerId]
    ///
    /// # Parameters
    ///
    /// * `listener_id` - The ListenerId
    ///
    /// Reference the proc macro crate (vsomeip-proc-macro) for further details
    fn get_message_handler(&self, listener_id: ListenerId) -> MessageHandlerFnPtr {
        MessageHandlerFnPtr(message_handler_proc_macro::get_extern_fn(listener_id))
    }
    /// Inserts [ListenerId] and [UPTransportVsomeipStorage] trait object
    fn insert_listener_id_transport(
        &self,
        listener_id: ListenerId,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus>;
    /// Removes [UPTransportVsomeipStorage] trait object based on [ListenerId]
    fn remove_listener_id_transport(&self, listener_id: ListenerId) -> Result<(), UStatus>;
    /// Frees up a [ListenerId] so that we can use the associated extern "C" fn when creating
    /// a callback to register with vsomeip
    fn free_listener_id(&self, listener_id: ListenerId) -> Result<(), UStatus>;
    /// Obtain a free [ListenerId] so that we can use the associated extern "C" fn when creating
    /// a callback to register with vsomeip
    fn find_available_listener_id(&self) -> Result<ListenerId, UStatus>;
    async fn print_rwlock_times(&self);
}

/// Concrete implementation of a registry of extern "C" fns
pub struct MessageHandlerRegistry;

impl MessageHandlerRegistry {
    /// Provides a [MockableMessageHandlerRegistry] trait object based on a [MessageHandlerRegistry]
    pub fn new_trait_obj() -> Arc<dyn MockableMessageHandlerRegistry> {
        Arc::new(MessageHandlerRegistry)
    }
}

#[async_trait]
impl MockableMessageHandlerRegistry for MessageHandlerRegistry {
    fn insert_listener_id_transport(
        &self,
        listener_id: usize,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus> {
        let mut listener_id_transport_shim = LISTENER_ID_TO_TRANSPORT_STORAGE.write();
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

    fn remove_listener_id_transport(&self, listener_id: usize) -> Result<(), UStatus> {
        let mut listener_id_transport_shim = LISTENER_ID_TO_TRANSPORT_STORAGE.write();
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

    fn free_listener_id(&self, listener_id: usize) -> Result<(), UStatus> {
        let mut free_ids = message_handler_proc_macro::FREE_LISTENER_IDS.write();
        free_ids.insert(listener_id);

        trace!("free_listener_id: {listener_id}");

        Ok(())
    }

    fn find_available_listener_id(&self) -> Result<usize, UStatus> {
        let mut free_ids = message_handler_proc_macro::FREE_LISTENER_IDS.write();
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

    async fn print_rwlock_times(&self) {
        #[cfg(feature = "timing")]
        {
            println!("FREE_LISTENER_IDS:");
            println!("reads: {:?}", FREE_LISTENER_IDS.read_durations());
            println!("writes: {:?}", FREE_LISTENER_IDS.write_durations());

            println!("LISTENER_ID_TRANSPORT_SHIM:");
            println!(
                "reads: {:?}",
                LISTENER_ID_TO_TRANSPORT_STORAGE.read_durations()
            );
            println!(
                "writes: {:?}",
                LISTENER_ID_TO_TRANSPORT_STORAGE.write_durations()
            );
        }
    }
}

// TODO: Add unit tests
