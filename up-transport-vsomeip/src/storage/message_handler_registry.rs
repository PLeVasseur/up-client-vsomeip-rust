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
use crate::{ClientId, MessageHandlerId};
use bimap::BiMap;
use cxx::SharedPtr;
use lazy_static::lazy_static;
use log::{debug, error, info, trace, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error;
use std::fmt::{Display, Formatter};
use std::ops::DerefMut;
use std::sync::{mpsc, Arc, Weak};
use tokio::runtime::Runtime;
use tokio::task::LocalSet;
use up_rust::{ComparableListener, UListener, UUri};
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

type MessageHandlerIdToTransportStorage =
    HashMap<MessageHandlerId, Weak<dyn UPTransportVsomeipStorage + Send + Sync>>;
lazy_static! {
    /// A mapping from extern "C" fn [MessageHandlerId] onto [std::sync::Weak] references to [UPTransportVsomeipStorage]
    ///
    /// Used within the context of the proc macro crate (vsomeip-proc-macro) generated [call_shared_extern_fn]
    /// to obtain the state of the transport needed to perform ingestion of vsomeip messages from
    /// within callback functions registered with vsomeip
    static ref MESSAGE_HANDLER_ID_TO_TRANSPORT_STORAGE: TimedStdRwLock<MessageHandlerIdToTransportStorage> =
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
    /// * `message_handler_id`
    fn get_message_handler_id_transport(
        message_handle_id: MessageHandlerId,
    ) -> Option<Arc<dyn UPTransportVsomeipStorage + Send + Sync>> {
        let message_handler_id_transport_shim = MESSAGE_HANDLER_ID_TO_TRANSPORT_STORAGE.read();
        let transport = message_handler_id_transport_shim.get(&message_handle_id)?;

        transport.upgrade()
    }
}

#[derive(Debug)]
pub enum GetMessageHandlerError {
    ListenerIdAlreadyExists(MessageHandlerId),
    ListenerConfigAlreadyExists(MessageHandlerFnPtr),
    OtherError(String),
}

impl Display for GetMessageHandlerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GetMessageHandlerError::ListenerIdAlreadyExists(listener_id) => {
                f.write_fmt(format_args!("Duplicated listener_id: {}", listener_id))
            }
            GetMessageHandlerError::ListenerConfigAlreadyExists(_msg_handler) => {
                f.write_fmt(format_args!("Duplicated listener_config"))
            }
            GetMessageHandlerError::OtherError(err) => {
                f.write_fmt(format_args!("Other error occurred: {err}"))
            }
        }
    }
}

impl error::Error for GetMessageHandlerError {}

#[derive(Debug)]
pub enum MessageHandlerIdAndListenerConfigError {
    MessageHandlerIdAlreadyExists(MessageHandlerId),
    ListenerConfigAlreadyExists,
}

impl Display for MessageHandlerIdAndListenerConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageHandlerIdAndListenerConfigError::MessageHandlerIdAlreadyExists(
                message_handler_id,
            ) => f.write_fmt(format_args!(
                "Duplicated message_handler_id: {}",
                message_handler_id
            )),
            MessageHandlerIdAndListenerConfigError::ListenerConfigAlreadyExists => {
                f.write_fmt(format_args!("Duplicated listener_config"))
            }
        }
    }
}

impl error::Error for MessageHandlerIdAndListenerConfigError {}

pub enum ClientUsage {
    ClientIdInUse,
    ClientIdNotInUse(ClientId),
}

type MessageHandlerIdAndListenerConfig =
    BiMap<MessageHandlerId, (UUri, Option<UUri>, ComparableListener)>;
type MessageHandlerIdToClientId = HashMap<MessageHandlerId, ClientId>;
type ClientIdToMessageHandlerId = HashMap<ClientId, HashSet<MessageHandlerId>>;
pub struct MessageHandlerRegistry {
    message_handler_id_and_listener_config: TimedStdRwLock<MessageHandlerIdAndListenerConfig>,
    message_handler_id_to_client_id: TimedStdRwLock<MessageHandlerIdToClientId>,
    client_id_to_message_handler_id: TimedStdRwLock<ClientIdToMessageHandlerId>,
}

impl MessageHandlerRegistry {
    pub fn new() -> Self {
        Self {
            message_handler_id_and_listener_config: TimedStdRwLock::new(BiMap::new()),
            message_handler_id_to_client_id: TimedStdRwLock::new(HashMap::new()),
            client_id_to_message_handler_id: TimedStdRwLock::new(HashMap::new()),
        }
    }

    /// Gets an unused [MessageHandlerFnPtr] to hand over to a vsomeip application
    pub fn get_message_handler(
        &self,
        client_id: ClientId,
        transport_storage: Arc<dyn UPTransportVsomeipStorage>,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<MessageHandlerFnPtr, GetMessageHandlerError> {
        // Lock all the necessary state at the beginning so we don't have partial transactions
        let mut message_handler_id_to_transport_storage =
            MESSAGE_HANDLER_ID_TO_TRANSPORT_STORAGE.write();
        let mut free_message_handler_ids =
            message_handler_proc_macro::FREE_MESSAGE_HANDLER_IDS.write();

        let mut message_handler_id_and_listener_config =
            self.message_handler_id_and_listener_config.write();
        let mut message_handler_id_to_client_id = self.message_handler_id_to_client_id.write();
        let mut client_id_to_message_handler_id = self.client_id_to_message_handler_id.write();

        let (source_filter, sink_filter, comparable_listener) = listener_config;

        let Ok(message_handler_id) =
            Self::find_available_message_handler_id(free_message_handler_ids.deref_mut())
        else {
            return Err(GetMessageHandlerError::OtherError(format!(
                "{:?}",
                UStatus::fail_with_code(UCode::RESOURCE_EXHAUSTED, "No more available extern fns",)
            )));
        };

        type RollbackSteps<'a> = Vec<Box<dyn FnOnce() + 'a>>;
        let mut rollback_steps: RollbackSteps = Vec::new();

        rollback_steps.push(Box::new(move || {
            if let Err(warn) = Self::free_message_handler_id(
                free_message_handler_ids.deref_mut(),
                message_handler_id,
            ) {
                warn!("rolling back: free_message_handler_id: {warn}");
            }
        }));

        let insert_res = Self::insert_message_handler_id_transport(
            message_handler_id_to_transport_storage.deref_mut(),
            message_handler_id,
            transport_storage,
        );
        if let Err(err) = insert_res {
            for rollback_step in rollback_steps {
                rollback_step();
            }

            return Err(GetMessageHandlerError::OtherError(format!("{:?}", err)));
        }

        rollback_steps.push(Box::new(move || {
            if let Err(warn) = Self::remove_message_handler_id_transport(
                message_handler_id_to_transport_storage.deref_mut(),
                message_handler_id,
            ) {
                warn!("rolling back: remove_listener_id_transport: {warn}");
            }
        }));

        let insert_res = Self::insert_message_handler_id_client_id(
            message_handler_id_to_client_id.deref_mut(),
            client_id_to_message_handler_id.deref_mut(),
            message_handler_id,
            client_id,
        );
        if let Some(previous_entry) = insert_res {
            let message_handler_id = previous_entry.0;
            let client_id = previous_entry.1;

            for rollback_step in rollback_steps {
                rollback_step();
            }

            return Err(GetMessageHandlerError::OtherError(
                format!("{:?}", UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS, format!(
                        "We already had used that listener_id with a client_id. listener_id: {} client_id: {}",
                        message_handler_id, client_id))
                )
            )
            );
        }

        let listener_config = (
            source_filter.clone(),
            sink_filter.clone(),
            comparable_listener.clone(),
        );

        rollback_steps.push(Box::new(move || {
            if Self::remove_client_id_based_on_message_handler_id(
                message_handler_id_to_client_id.deref_mut(),
                client_id_to_message_handler_id.deref_mut(),
                message_handler_id,
            )
            .is_none()
            {
                warn!("No client_id found to remove for message_handler_id: {message_handler_id}");
            }
        }));

        let insert_res = Self::insert_message_handler_id_and_listener_config(
            message_handler_id_and_listener_config.deref_mut(),
            message_handler_id,
            listener_config,
        );
        if let Err(err) = insert_res {
            for rollback_step in rollback_steps {
                rollback_step();
            }

            return Err(match err {
                MessageHandlerIdAndListenerConfigError::MessageHandlerIdAlreadyExists(
                    listener_id,
                ) => GetMessageHandlerError::ListenerIdAlreadyExists(listener_id),
                MessageHandlerIdAndListenerConfigError::ListenerConfigAlreadyExists => {
                    GetMessageHandlerError::ListenerConfigAlreadyExists(MessageHandlerFnPtr(
                        message_handler_proc_macro::get_extern_fn(message_handler_id),
                    ))
                }
            });
        }

        rollback_steps.push(Box::new(move || {
            if let Err(warn) = Self::remove_message_handler_id_and_listener_config_based_on_message_handler_id(message_handler_id_and_listener_config.deref_mut(), message_handler_id)
            {
                warn!("rolling back: remove_listener_id_and_listener_config_based_on_listener_id: {warn}");
            }
        }));

        Ok(MessageHandlerFnPtr(
            message_handler_proc_macro::get_extern_fn(message_handler_id),
        ))
    }

    /// Release a given message handler
    pub fn release_message_handler(
        &self,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<ClientUsage, UStatus> {
        // Lock all the necessary state at the beginning so we don't have partial transactions
        let mut message_handler_id_to_transport_storage =
            MESSAGE_HANDLER_ID_TO_TRANSPORT_STORAGE.write();
        let mut free_message_handler_ids =
            message_handler_proc_macro::FREE_MESSAGE_HANDLER_IDS.write();

        let mut message_handler_id_and_listener_config =
            self.message_handler_id_and_listener_config.write();
        let mut message_handler_id_to_client_id = self.message_handler_id_to_client_id.write();
        let mut client_id_to_message_handler_id = self.client_id_to_message_handler_id.write();

        let message_handler_id = Self::get_message_handler_id_for_listener_config(
            message_handler_id_and_listener_config.deref_mut(),
            listener_config,
        )
        .ok_or(
            // TODO: Update this to include listener_config when we update to up-rust on crates.io with ComparableListener Debug impl
            UStatus::fail_with_code(UCode::NOT_FOUND, "No listener_id for listener_config"), // UStatus::fail_with_code(UCode::NOT_FOUND, format!("No listener_id for listener_config: {listener_config:?}"))
        )?;

        if let Err(warn) =
            Self::free_message_handler_id(free_message_handler_ids.deref_mut(), message_handler_id)
        {
            warn!("{warn}");
        }

        if let Err(warn) = Self::remove_message_handler_id_transport(
            message_handler_id_to_transport_storage.deref_mut(),
            message_handler_id,
        ) {
            warn!("{warn}");
        }

        if let Err(warn) =
            Self::remove_message_handler_id_and_listener_config_based_on_message_handler_id(
                message_handler_id_and_listener_config.deref_mut(),
                message_handler_id,
            )
        {
            warn!("{warn}");
        }

        let client_id = {
            match Self::remove_client_id_based_on_message_handler_id(
                message_handler_id_to_client_id.deref_mut(),
                client_id_to_message_handler_id.deref_mut(),
                message_handler_id,
            ) {
                None => {
                    return Err(UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!(
                            "No client_id found to remove for listener_id: {message_handler_id}"
                        ),
                    ));
                }
                Some(client_id) => client_id,
            }
        };

        if Self::message_handler_count_for_client_id(
            client_id_to_message_handler_id.deref_mut(),
            client_id,
        ) == 0
        {
            Ok(ClientUsage::ClientIdNotInUse(client_id))
        } else {
            Ok(ClientUsage::ClientIdInUse)
        }
    }

    /// Get all listener configs
    pub fn get_all_listener_configs(&self) -> Vec<(UUri, Option<UUri>, ComparableListener)> {
        let all_message_handler_ids = self.get_message_handler_ids();
        let mut listener_configs = Vec::new();
        for message_handler_id in all_message_handler_ids {
            let listener_config =
                self.get_listener_config_for_message_handler_id(message_handler_id);
            let Some(listener_config) = listener_config else {
                warn!(
                    "Unable to find listener_config for message_handler_id: {message_handler_id}"
                );
                continue;
            };
            listener_configs.push(listener_config);
        }
        listener_configs
    }

    /// Get all [MessageHandlerId]s
    fn get_message_handler_ids(&self) -> Vec<usize> {
        let message_handler_id_and_listener_config =
            self.message_handler_id_and_listener_config.read();

        message_handler_id_and_listener_config
            .left_values()
            .copied()
            .collect()
    }

    /// Get [MessageHandlerId] based on a listener configuration
    fn get_message_handler_id_for_listener_config(
        message_handler_id_and_listener_config: &mut BiMap<
            usize,
            (UUri, Option<UUri>, ComparableListener),
        >,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Option<usize> {
        let message_handler_id =
            message_handler_id_and_listener_config.get_by_right(&listener_config)?;

        Some(*message_handler_id)
    }

    /// Get trait object [UListener] for a [MessageHandlerId]
    fn get_listener_for_message_handler_id(
        &self,
        message_handler_id: usize,
    ) -> Option<Arc<dyn UListener>> {
        let listener_id_and_listener_config = self.message_handler_id_and_listener_config.read();

        let (_, _, comp_listener) =
            listener_id_and_listener_config.get_by_left(&message_handler_id)?;

        Some(comp_listener.into_inner())
    }

    fn get_listener_config_for_message_handler_id(
        &self,
        message_handler_id: usize,
    ) -> Option<(UUri, Option<UUri>, ComparableListener)> {
        let message_handler_id_and_listener_config =
            self.message_handler_id_and_listener_config.read();

        let (src, sink, comp_listener) =
            message_handler_id_and_listener_config.get_by_left(&message_handler_id)?;
        Some((src.clone(), sink.clone(), comp_listener.clone()))
    }

    fn insert_message_handler_id_transport(
        message_handler_id_to_transport_storage: &mut HashMap<
            MessageHandlerId,
            Weak<dyn UPTransportVsomeipStorage + Send + Sync>,
        >,
        listener_id: usize,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus> {
        if let std::collections::hash_map::Entry::Vacant(e) =
            message_handler_id_to_transport_storage.entry(listener_id)
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

    fn remove_message_handler_id_transport(
        message_handler_id_to_transport_storage: &mut HashMap<
            usize,
            Weak<(dyn UPTransportVsomeipStorage + Send + Sync + 'static)>,
        >,
        listener_id: usize,
    ) -> Result<(), UStatus> {
        if message_handler_id_to_transport_storage.contains_key(&listener_id) {
            message_handler_id_to_transport_storage.remove(&listener_id);
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

    fn free_message_handler_id(
        free_message_handler_ids: &mut HashSet<usize>,
        listener_id: usize,
    ) -> Result<(), UStatus> {
        free_message_handler_ids.insert(listener_id);

        trace!("free_listener_id: {listener_id}");

        Ok(())
    }

    fn find_available_message_handler_id(
        free_message_handler_ids: &mut HashSet<usize>,
    ) -> Result<usize, UStatus> {
        if let Some(&id) = free_message_handler_ids.iter().next() {
            free_message_handler_ids.remove(&id);
            trace!("find_available_listener_id: {id}");
            Ok(id)
        } else {
            Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "No more extern C fns available",
            ))
        }
    }

    /// Insert [MessageHandlerId] and [ClientId]
    fn insert_message_handler_id_client_id(
        message_handler_id_to_client_id: &mut HashMap<usize, u16>,
        client_id_to_message_handler_id: &mut HashMap<u16, HashSet<usize>>,
        message_handler_id: MessageHandlerId,
        client_id: ClientId,
    ) -> Option<(usize, ClientId)> {
        debug!("before listener_id_to_client_id: {message_handler_id_to_client_id:?}");

        if message_handler_id_to_client_id.contains_key(&message_handler_id) {
            info!("We already used message_handler_id: {message_handler_id}");
            debug!("message_handler_id_to_client_id: {message_handler_id_to_client_id:?}");

            let client_id = message_handler_id_to_client_id
                .get(&message_handler_id)
                .unwrap();
            return Some((message_handler_id, *client_id));
        }

        let insert_res = message_handler_id_to_client_id.insert(message_handler_id, client_id);
        if let Some(client_id) = insert_res {
            info!("We already inserted message_handler_id: {message_handler_id} with client_id: {client_id}");

            return Some((message_handler_id, client_id));
        }

        let message_handler_ids = client_id_to_message_handler_id
            .entry(client_id)
            .or_default();
        let newly_added = message_handler_ids.insert(message_handler_id);
        if !newly_added {
            info!("Attempted to inserted already existing message_handler_id: {message_handler_id} into client_id: {client_id}");

            return Some((message_handler_id, client_id));
        }

        info!("Newly added message_handler_id: {message_handler_id} client_id: {client_id}");
        debug!("after message_handler_id_to_client_id: {message_handler_id_to_client_id:?}");

        None
    }

    /// Remove [MessageHandlerId] and listener configuration
    fn remove_message_handler_id_and_listener_config_based_on_message_handler_id(
        message_handler_id_and_listener_config: &mut BiMap<
            usize,
            (UUri, Option<UUri>, ComparableListener),
        >,
        message_handler_id: MessageHandlerId,
    ) -> Result<(), UStatus> {
        let removed = message_handler_id_and_listener_config.remove_by_left(&message_handler_id);
        if removed.is_none() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("No listener_config for listener_id: {message_handler_id}"),
            ));
        }

        Ok(())
    }

    /// Remove [ClientId] based on [MessageHandlerId]
    fn remove_client_id_based_on_message_handler_id(
        message_handler_id_to_client_id: &mut HashMap<usize, u16>,
        client_id_to_message_handler_id: &mut HashMap<u16, HashSet<usize>>,
        message_handler_id: usize,
    ) -> Option<ClientId> {
        let removed = message_handler_id_to_client_id.remove(&message_handler_id);
        if let Some(client_id) = removed {
            client_id_to_message_handler_id
                .entry(client_id)
                .or_default()
                .remove(&message_handler_id);

            return Some(client_id);
        }

        None
    }

    /// Insert [MessageHandlerId] and listener configuration
    fn insert_message_handler_id_and_listener_config(
        message_handler_id_and_listener_config: &mut BiMap<
            usize,
            (UUri, Option<UUri>, ComparableListener),
        >,
        message_handler_id: usize,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<(), MessageHandlerIdAndListenerConfigError> {
        trace!(
            "insert_listener_id_and_listener_config: message_handler_id: {}",
            message_handler_id
        );

        let insertion_res = message_handler_id_and_listener_config
            .insert_no_overwrite(message_handler_id, listener_config.clone());

        if let Err(_existed) = insertion_res {
            if message_handler_id_and_listener_config.contains_left(&message_handler_id) {
                return Err(
                    MessageHandlerIdAndListenerConfigError::MessageHandlerIdAlreadyExists(
                        message_handler_id,
                    ),
                );
            }

            if message_handler_id_and_listener_config.contains_right(&listener_config) {
                return Err(MessageHandlerIdAndListenerConfigError::ListenerConfigAlreadyExists);
            }
        }

        Ok(())
    }

    /// Find count of listeners registered to [ClientId]
    fn message_handler_count_for_client_id(
        client_id_to_message_handler_id: &mut HashMap<u16, HashSet<usize>>,
        client_id: ClientId,
    ) -> usize {
        let message_handler_ids = client_id_to_message_handler_id.get(&client_id);
        if let Some(listener_ids) = message_handler_ids {
            return listener_ids.len();
        }

        0
    }

    pub async fn print_rwlock_times(&self) {
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

            println!("listener_id_and_listener_config:");
            println!(
                "reads: {:?}",
                self.message_handler_id_and_listener_config.read_durations()
            );
            println!(
                "writes: {:?}",
                self.message_handler_id_and_listener_config
                    .write_durations()
            );

            println!("listener_id_to_client_id:");
            println!(
                "reads: {:?}",
                self.message_handler_id_to_client_id.read_durations()
            );
            println!(
                "writes: {:?}",
                self.message_handler_id_to_client_id.write_durations()
            );

            println!("client_id_to_listener_id:");
            println!(
                "reads: {:?}",
                self.client_id_to_message_handler_id.read_durations()
            );
            println!(
                "writes: {:?}",
                self.client_id_to_message_handler_id.write_durations()
            );
        }
    }
}

// TODO: Add unit tests
