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
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::timeout;

use log::{error, info, trace};

use up_rust::{ComparableListener, UCode, UListener, UMessage, UStatus, UTransport, UUri};
use vsomeip_proc_macro::generate_message_handler_extern_c_fns;
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{make_application_wrapper, make_message_wrapper, make_runtime_wrapper};
use vsomeip_sys::safe_glue::get_pinned_runtime;
use vsomeip_sys::vsomeip;

use crate::is_point_to_point_message;
use crate::{convert_vsomeip_msg_to_umsg, TransportCommand};
use crate::{determine_registration_type, RegistrationType, UPClientVsomeip};
use crate::{ClientId, ReqId, RequestId, SessionId};

const INTERNAL_FUNCTION_TIMEOUT: u64 = 2;

generate_message_handler_extern_c_fns!(10000);

async fn await_internal_function(
    rx: oneshot::Receiver<Result<(), UStatus>>,
) -> Result<(), UStatus> {
    match timeout(Duration::from_secs(INTERNAL_FUNCTION_TIMEOUT), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(UStatus::fail_with_code(
            UCode::INTERNAL,
            "Unable to receive status back for send_internal()",
        )),
        Err(_) => Err(UStatus::fail_with_code(
            UCode::DEADLINE_EXCEEDED,
            format!(
                "Unable to receive status back from send_internal() within {} second window.",
                INTERNAL_FUNCTION_TIMEOUT
            ),
        )),
    }
}

#[async_trait]
impl UTransport for UPClientVsomeip {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        // implementation goes here
        println!("Sending message: {:?}", message);

        let (tx, rx) = oneshot::channel();

        // consider using a worker pool for these, otherwise this will block
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::Send(message, tx))
            .await;
        await_internal_function(rx).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        // implementation goes here
        let sink_filter_str = {
            if let Some(sink_filter) = sink_filter {
                format!("{sink_filter:?}")
            } else {
                "".parse().unwrap()
            }
        };
        info!(
            "Registering listener for source filter: {:?} sink_filter: {}",
            source_filter, sink_filter_str
        );

        let registration_type_res =
            determine_registration_type(source_filter, &sink_filter.cloned());
        let Ok(registration_type) = registration_type_res else {
            return Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "Invalid source and sink filters for registerable types: Publish, Request, Response, AllPointToPoint"));
        };

        let listener_id = {
            let mut free_ids = FREE_LISTENER_IDS.lock().unwrap();
            if let Some(&id) = free_ids.iter().next() {
                free_ids.remove(&id);
                id
            } else {
                return Err(UStatus::fail_with_code(
                    UCode::RESOURCE_EXHAUSTED,
                    "No more extern C fns available",
                ));
            }
        };

        trace!("Obtained listener_id: {}", listener_id);

        if registration_type == RegistrationType::AllPointToPoint {
            let mut point_to_point_listeners = POINT_TO_POINT_LISTENERS.lock().unwrap();
            point_to_point_listeners.insert(listener_id);
        }

        let comp_listener = ComparableListener::new(Arc::clone(&listener));
        let key = (source_filter.clone(), sink_filter.cloned(), comp_listener);

        {
            let mut id_map = LISTENER_ID_MAP.lock().unwrap();
            id_map.insert(key, listener_id);
        }

        trace!("Inserted into LISTENER_ID_MAP");

        LISTENER_REGISTRY
            .lock()
            .unwrap()
            .insert(listener_id, listener);

        let extern_fn = get_extern_fn(listener_id);
        // let extern_fn = get_extern_fn_dummy(listener_id);
        let msg_handler = MessageHandlerFnPtr(extern_fn);
        let src = source_filter.clone();
        let sink = sink_filter.cloned();

        trace!("Obtained extern_fn");

        // consider using a worker pool for these, otherwise this will block
        let (tx, rx) = oneshot::channel();
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::RegisterListener(
                src,
                sink,
                msg_handler,
                tx,
            ))
            .await;
        await_internal_function(rx).await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        // implementation goes here
        let sink_filter_str = {
            if let Some(sink_filter) = sink_filter {
                format!("{sink_filter:?}")
            } else {
                "".parse().unwrap()
            }
        };
        println!(
            "Unregistering listener for source filter: {:?}{}",
            source_filter, sink_filter_str
        );
        let src = source_filter.clone();
        let sink = sink_filter.cloned();

        let registration_type_res =
            determine_registration_type(source_filter, &sink_filter.cloned());
        let Ok(registration_type) = registration_type_res else {
            return Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "Invalid source and sink filters for registerable types: Publish, Request, Response, AllPointToPoint"));
        };

        let comp_listener = ComparableListener::new(listener);

        // consider using a worker pool for these, otherwise this will block
        let (tx, rx) = oneshot::channel();
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::UnregisterListener(src, sink, tx))
            .await;
        await_internal_function(rx).await?;

        let listener_id = {
            let mut id_map = LISTENER_ID_MAP.lock().unwrap();
            if let Some(&id) = id_map.get(&(
                source_filter.clone(),
                sink_filter.cloned(),
                comp_listener.clone(),
            )) {
                id_map.remove(&(source_filter.clone(), sink_filter.cloned(), comp_listener));
                id
            } else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Listener not found",
                )); // Custom error indicating listener not found
            }
        };

        {
            let mut registry = LISTENER_REGISTRY.lock().unwrap();
            registry.remove(&listener_id);
        }

        {
            let mut free_ids = FREE_LISTENER_IDS.lock().unwrap();
            free_ids.insert(listener_id);
        }

        if registration_type == RegistrationType::AllPointToPoint {
            let mut point_to_point_listeners = POINT_TO_POINT_LISTENERS.lock().unwrap();
            point_to_point_listeners.remove(&listener_id);
        }

        Ok(())
    }

    async fn receive(
        &self,
        _source_filter: &UUri,
        _sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        Err(UStatus::fail_with_code(
            UCode::UNIMPLEMENTED,
            "This method is not implemented for vsomeip. Use register_listener instead.",
        ))
    }
}

#[cfg(test)]
mod tests {}
