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

use crate::listener_registry::ListenerRegistry;
use crate::message_conversions::convert_vsomeip_msg_to_umsg;
use crate::UPTransportVsomeip;
use crate::{
    ApplicationName, AuthorityName, ClientId, MockableUPTransportVsomeipInner,
    UPTransportVsomeipStorage,
};
use async_trait::async_trait;
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use log::{error, info, trace};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{mpsc, Arc, RwLock, Weak};
use tokio::runtime::Runtime;
use tokio::sync::RwLock as TokioRwLock;
use tokio::task::LocalSet;
use tokio::time::Instant;
use up_rust::{ComparableListener, UListener, UUri};
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
    static ref LISTENER_ID_TRANSPORT_SHIM: TokioRwLock<HashMap<usize, Weak<dyn UPTransportVsomeipStorage + Send + Sync>>> =
        TokioRwLock::new(HashMap::new());
}

#[async_trait]
pub(crate) trait MockableExternFnRegistry: Send + Sync {
    async fn insert_listener_id_transport(
        &self,
        listener_id: usize,
        transport: Arc<dyn UPTransportVsomeipStorage + Send + Sync>,
    ) -> Result<(), UStatus>;
    async fn remove_listener_id_transport(&self, listener_id: usize) -> Result<(), UStatus>;
    async fn get_listener_id_transport(
        &self,
        listener_id: usize,
    ) -> Option<Arc<dyn UPTransportVsomeipStorage + Send + Sync>>;
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
        if !listener_id_transport_shim.contains_key(&listener_id) {
            listener_id_transport_shim.insert(listener_id, Arc::downgrade(&transport));
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

    async fn get_listener_id_transport(
        &self,
        listener_id: usize,
    ) -> Option<Arc<dyn UPTransportVsomeipStorage + Send + Sync>> {
        let listener_id_transport_shim = LISTENER_ID_TRANSPORT_SHIM.read().await;
        let Some(transport) = listener_id_transport_shim.get(&listener_id) else {
            return None;
        };

        transport.upgrade()
    }

    async fn free_listener_id(&self, listener_id: usize) -> Result<(), UStatus> {
        let mut free_ids = FREE_LISTENER_IDS.write().await;
        free_ids.insert(listener_id);

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

impl ExternFnRegistry {
    pub fn new() -> Arc<dyn MockableExternFnRegistry> {
        Arc::new(ExternFnRegistry)
    }
}

type ListenerIdMap = TokioRwLock<HashMap<(UUri, Option<UUri>, ComparableListener), usize>>;
lazy_static! {
    static ref LISTENER_ID_AUTHORITY_NAME: TokioRwLock<HashMap<usize, AuthorityName>> =
        TokioRwLock::new(HashMap::new());
    static ref LISTENER_ID_REMOTE_AUTHORITY_NAME: TokioRwLock<HashMap<usize, AuthorityName>> =
        TokioRwLock::new(HashMap::new());
    static ref LISTENER_REGISTRY: TokioRwLock<HashMap<usize, (UUri, Option<UUri>, ComparableListener)>> =
        TokioRwLock::new(HashMap::new());
    static ref LISTENER_ID_MAP: ListenerIdMap = TokioRwLock::new(HashMap::new());
    static ref LISTENER_ID_CLIENT_ID_MAPPING: TokioRwLock<HashMap<usize, ClientId>> =
        TokioRwLock::new(HashMap::new());
    static ref CLIENT_ID_TO_LISTENER_ID_MAPPING: TokioRwLock<HashMap<ClientId, HashSet<usize>>> =
        TokioRwLock::new(HashMap::new());
    static ref CLIENT_ID_APP_MAPPING: TokioRwLock<HashMap<ClientId, String>> =
        TokioRwLock::new(HashMap::new());
    static ref TRANSPORT_INSTANCE_TO_LISTENER_ID: TokioRwLock<HashMap<uuid::Uuid, HashSet<usize>>> =
        TokioRwLock::new(HashMap::new());
    static ref TRANSPORT_INSTANCE_TO_CLIENT_ID: TokioRwLock<HashMap<uuid::Uuid, HashSet<ClientId>>> =
        TokioRwLock::new(HashMap::new());
}

#[derive(Debug)]
pub(crate) enum CloseVsomeipApp {
    False,
    True(ClientId, ApplicationName),
}

pub(crate) struct Registry;

impl Registry {
    pub(crate) async fn insert_instance_client_id(
        transport_instance_id: uuid::Uuid,
        client_id: ClientId,
    ) -> Result<(), UStatus> {
        let mut transport_instance_to_client_id = TRANSPORT_INSTANCE_TO_CLIENT_ID.write().await;

        let client_ids = transport_instance_to_client_id
            .entry(transport_instance_id)
            .or_default();
        client_ids.insert(client_id);

        Ok(())
    }

    pub(crate) async fn get_instance_client_ids(
        transport_instance_id: uuid::Uuid,
    ) -> HashSet<ClientId> {
        let transport_instance_to_client_id = TRANSPORT_INSTANCE_TO_CLIENT_ID.read().await;

        return match transport_instance_to_client_id.get(&transport_instance_id) {
            None => HashSet::new(),
            Some(client_ids) => client_ids.clone(),
        };
    }

    pub(crate) async fn get_instance_listener_ids(
        transport_instance_id: uuid::Uuid,
    ) -> HashSet<usize> {
        let transport_instance_to_listener_id = TRANSPORT_INSTANCE_TO_LISTENER_ID.read().await;

        return match transport_instance_to_listener_id.get(&transport_instance_id) {
            None => HashSet::new(),
            Some(listener_ids) => listener_ids.clone(),
        };
    }

    pub(crate) async fn get_listener_configuration(
        listener_id: usize,
    ) -> Option<(UUri, Option<UUri>, ComparableListener)> {
        let listener_registry = LISTENER_REGISTRY.read().await;

        return match listener_registry.get(&listener_id) {
            None => None,
            Some((src, sink, comparable_listener)) => {
                Some((src.clone(), sink.clone(), comparable_listener.clone()))
            }
        };
    }

    pub(crate) async fn get_listener_authority(listener_id: usize) -> Option<AuthorityName> {
        let listener_id_authority_name = LISTENER_ID_AUTHORITY_NAME.read().await;

        return match listener_id_authority_name.get(&listener_id) {
            None => None,
            Some(authority_name) => Some(authority_name.clone()),
        };
    }

    pub(crate) async fn get_listener_remote_authority(listener_id: usize) -> Option<AuthorityName> {
        let listener_id_remote_authority_name = LISTENER_ID_REMOTE_AUTHORITY_NAME.read().await;

        return match listener_id_remote_authority_name.get(&listener_id) {
            None => None,
            Some(remote_authority_name) => Some(remote_authority_name.clone()),
        };
    }

    pub(crate) async fn free_listener_id(listener_id: usize) -> UStatus {
        info!("listener_id was not used since we already have registered for this");
        let mut free_ids = FREE_LISTENER_IDS.write().await;
        free_ids.insert(listener_id);
        UStatus::fail_with_code(
            UCode::ALREADY_EXISTS,
            "Already have registered with this source, sink and listener",
        )
    }

    pub(crate) async fn insert_into_listener_id_map(
        transport_instance_id: uuid::Uuid,
        authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        key: (UUri, Option<UUri>, ComparableListener),
        listener_id: usize,
    ) -> Result<(), UStatus> {
        trace!(
            "authority_name: {}, remote_authority_name: {}, listener_id: {}",
            authority_name,
            remote_authority_name,
            listener_id
        );

        // TODO: Should write unit tests to ensure that we don't record a partial transaction by rolling back any pieces which succeeded if a latter part fails

        let mut id_map = LISTENER_ID_MAP.write().await;
        let mut listener_id_authority_name = LISTENER_ID_AUTHORITY_NAME.write().await;
        let mut listener_id_remote_authority_name = LISTENER_ID_REMOTE_AUTHORITY_NAME.write().await;
        let mut transport_instance_to_listener_id = TRANSPORT_INSTANCE_TO_LISTENER_ID.write().await;

        if id_map.insert(key.clone(), listener_id).is_some() {
            let msg = format!("Not inserted into LISTENER_ID_MAP since we already have registered for this Request: key-uuri: {:?} key-option-uuri: {:?} listener_id: {}",
                              key.0, key.1, listener_id);
            trace!("{msg}");
            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, msg));
        } else {
            trace!("Inserted into LISTENER_ID_MAP");
        }

        trace!(
            "checking listener_id_authority_name: {:?}",
            *listener_id_authority_name
        );

        if listener_id_authority_name
            .insert(listener_id, authority_name.to_string())
            .is_some()
        {
            let msg = "Not inserted into LISTENER_ID_AUTHORITY_NAME since we already have registered for this Request";
            trace!("{msg}");

            // if we fail here we should roll-back the insertion into id_map
            id_map.remove(&key);

            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, msg));
        } else {
            trace!("Inserted into LISTENER_ID_AUTHORITY_NAME");
        }

        if listener_id_remote_authority_name
            .insert(listener_id, remote_authority_name.to_string())
            .is_some()
        {
            let msg = "Not inserted into LISTENER_ID_REMOTE_AUTHORITY_NAME since we already have registered for this Request";
            trace!("{msg}");

            // if we fail here we should roll-back the insertion into id_map and listener_id_authority_name
            id_map.remove(&key);
            listener_id_authority_name.remove(&listener_id);

            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, msg));
        } else {
            trace!("Inserted into LISTENER_ID_REMOTE_AUTHORITY_NAME");
        }

        let listener_ids = transport_instance_to_listener_id
            .entry(transport_instance_id)
            .or_default();
        listener_ids.insert(listener_id);

        Ok(())
    }

    pub(crate) async fn find_available_listener_id() -> Result<usize, UStatus> {
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

    pub(crate) async fn release_listener_id(
        source_filter: &UUri,
        sink_filter: &Option<&UUri>,
        comp_listener: &ComparableListener,
    ) -> Result<CloseVsomeipApp, UStatus> {
        let listener_id = {
            let mut id_map = LISTENER_ID_MAP.write().await;
            if let Some(&id) = id_map.get(&(
                source_filter.clone(),
                sink_filter.cloned(),
                comp_listener.clone(),
            )) {
                id_map.remove(&(
                    source_filter.clone(),
                    sink_filter.cloned(),
                    comp_listener.clone(),
                ));
                id
            } else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Listener not found",
                ));
            }
        };

        let (maybe_client_id, errs) = {
            let mut errs = Vec::new();

            let mut listener_id_authority_name = LISTENER_ID_AUTHORITY_NAME.write().await;
            let mut listener_id_remote_authority_name =
                LISTENER_ID_REMOTE_AUTHORITY_NAME.write().await;
            let mut listener_registry = LISTENER_REGISTRY.write().await;
            let mut free_ids = FREE_LISTENER_IDS.write().await;

            trace!(
                "checking listener_id_authority_name: {:?}",
                *listener_id_authority_name
            );

            let removal_result = listener_id_authority_name.remove(&listener_id);
            if removal_result.is_none() {
                errs.push(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to locate authority_name for listener_id: {listener_id}"),
                ));
            }

            let removal_result = listener_id_remote_authority_name.remove(&listener_id);
            if removal_result.is_none() {
                errs.push(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!(
                        "Unable to locate remote_authority_name for listener_id: {listener_id}"
                    ),
                ))
            }

            let removal_result = listener_registry.remove(&listener_id);
            if removal_result.is_none() {
                errs.push(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to locate UListener for listener_id: {listener_id}"),
                ));
            }

            let insertion_success = free_ids.insert(listener_id);
            if insertion_success {
                errs.push(
                    UStatus::fail_with_code(UCode::INTERNAL, format!("Unable to re-insert listener_id back into free listeners, listener_id: {listener_id}"))
                );
            }

            let mut listener_client_id_mapping = LISTENER_ID_CLIENT_ID_MAPPING.write().await;
            let client_id_result = listener_client_id_mapping.remove(&listener_id);

            if client_id_result.is_none() {
                errs.push(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!(
                        "Unable to locate client_id (i.e. for app) for listener_id: {listener_id}"
                    ),
                ));
            }

            (client_id_result, errs)
        };

        let concat_errs = {
            let err_msgs = {
                let mut err_msgs = String::new();

                if !errs.is_empty() {
                    for err in errs {
                        err_msgs.push_str(&*err.message.unwrap());
                    }
                }

                err_msgs
            };

            UStatus::fail_with_code(UCode::INTERNAL, err_msgs)
        };

        let Some(client_id) = maybe_client_id else {
            return Err(concat_errs);
        };

        let mut errs = Vec::new();

        let mut client_id_to_listener_ids = CLIENT_ID_TO_LISTENER_ID_MAPPING.write().await;
        if !client_id_to_listener_ids.contains_key(&client_id) {
            errs.push(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Unable to find any listener_ids for client_id: {client_id}"),
            ))
        }

        client_id_to_listener_ids
            .entry(client_id)
            .and_modify(|listener_ids| {
                listener_ids.remove(&listener_id);
            });

        if client_id_to_listener_ids
            .entry(client_id)
            .or_default()
            .is_empty()
        {
            client_id_to_listener_ids.remove(&client_id);

            let mut client_id_app_mapping = CLIENT_ID_APP_MAPPING.write().await;
            let removal_result = client_id_app_mapping.remove(&client_id);
            match removal_result {
                None => {
                    errs.push(UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!("Unable to find app_name for client_id: {client_id}"),
                    ));

                    let mut err_msgs = String::new();

                    if !errs.is_empty() {
                        for err in errs {
                            err_msgs.push_str(&*err.message.unwrap());
                        }
                    }

                    return Err(UStatus::fail_with_code(UCode::INTERNAL, err_msgs));
                }
                Some(app_name) => {
                    client_id_to_listener_ids.remove(&client_id);
                    return Ok(CloseVsomeipApp::True(client_id, app_name));
                }
            }
        }

        Ok(CloseVsomeipApp::False)
    }

    pub(crate) async fn register_listener_id_with_listener(
        listener_id: usize,
        listener_configuration: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<(), UStatus> {
        LISTENER_REGISTRY
            .write()
            .await
            .insert(listener_id, listener_configuration)
            .map(|_| {
                Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to register the same listener_id and listener twice",
                ))
            })
            .unwrap_or(Ok(()))?;
        Ok(())
    }

    pub(crate) async fn get_client_id_from_listener_id(listener_id: usize) -> Option<ClientId> {
        let mut listener_client_id_mapping = LISTENER_ID_CLIENT_ID_MAPPING.read().await;

        match listener_client_id_mapping.get(&listener_id) {
            None => None,
            Some(client_id) => Some(*client_id),
        }
    }

    pub(crate) async fn map_listener_id_to_client_id(
        client_id: ClientId,
        listener_id: usize,
    ) -> Result<(), UStatus> {
        let mut errs = Vec::new();

        let mut listener_client_id_mapping = LISTENER_ID_CLIENT_ID_MAPPING.write().await;
        let mut client_id_to_listener_id_mapping = CLIENT_ID_TO_LISTENER_ID_MAPPING.write().await;

        let insertion_result = listener_client_id_mapping.insert(listener_id, client_id);
        if insertion_result.is_some() {
            errs.push(
                UStatus::fail_with_code(
                    UCode::INTERNAL,
                    format!("Unable to have the same listener_id with a different client_id, i.e. tied to app: listener_id: {} client_id: {}", listener_id, client_id),
                )
            );
        }

        if !client_id_to_listener_id_mapping.contains_key(&client_id) {
            client_id_to_listener_id_mapping.insert(client_id.clone(), HashSet::new());
        }

        client_id_to_listener_id_mapping.entry(client_id).and_modify(|listener_ids| {
            let already_exists = !listener_ids.insert(listener_id);
            if already_exists {
                errs.push(
                    UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("There already exists a listener_id under this client_id. listener_id: {} client_id: {}", listener_id, client_id))
                );
            }
        });

        if !errs.is_empty() {
            let mut err_msgs = String::new();
            for err in errs {
                err_msgs.push_str(&*err.message.unwrap());
            }

            return Err(UStatus::fail_with_code(UCode::INTERNAL, err_msgs));
        }

        Ok(())
    }

    pub(crate) async fn find_app_name(client_id: ClientId) -> Result<ApplicationName, UStatus> {
        let client_id_app_mapping = CLIENT_ID_APP_MAPPING.read().await;
        if let Some(app_name) = client_id_app_mapping.get(&client_id) {
            Ok(app_name.clone())
        } else {
            Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("There was no app_name found for client_id: {}", client_id),
            ))
        }
    }

    pub(crate) async fn add_client_id_app_name(
        client_id: ClientId,
        app_name: &ApplicationName,
    ) -> Result<(), UStatus> {
        let mut client_id_app_mapping = CLIENT_ID_APP_MAPPING.write().await;
        if let std::collections::hash_map::Entry::Vacant(e) = client_id_app_mapping.entry(client_id)
        {
            e.insert(app_name.clone());
            trace!(
                "Inserted client_id: {} and app_name: {} into CLIENT_ID_APP_MAPPING",
                client_id,
                app_name
            );
            Ok(())
        } else {
            let err_msg = format!(
                "Already had key. Somehow we already had an application running for client_id: {}",
                client_id
            );
            Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, err_msg))
        }
    }
}

// TODO: Add unit tests
