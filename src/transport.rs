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
use vsomeip_sys::vsomeip::message;

use crate::determinations::{determine_registration_type, is_point_to_point_message};
use crate::message_conversions::convert_vsomeip_msg_to_umsg;
use crate::vsomeip_config::extract_applications;
use crate::{
    ClientId, ReqId, RequestId, SessionId, UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL,
};
use crate::{RegistrationType, UPClientVsomeip};
use crate::{
    TransportCommand, UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL, UP_CLIENT_VSOMEIP_TAG,
};

const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER: &str = "register_listener";

const INTERNAL_FUNCTION_TIMEOUT: u64 = 3;

generate_message_handler_extern_c_fns!(10000);

async fn await_internal_function(
    function_id: &str,
    rx: oneshot::Receiver<Result<(), UStatus>>,
) -> Result<(), UStatus> {
    match timeout(Duration::from_secs(INTERNAL_FUNCTION_TIMEOUT), rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(UStatus::fail_with_code(
            UCode::INTERNAL,
            format!(
                "Unable to receive status back from internal function: {}",
                function_id
            ),
        )),
        Err(_) => Err(UStatus::fail_with_code(
            UCode::DEADLINE_EXCEEDED,
            format!(
                "Unable to receive status back from internal function: {} within {} second window.",
                function_id, INTERNAL_FUNCTION_TIMEOUT
            ),
        )),
    }
}

#[async_trait]
impl UTransport for UPClientVsomeip {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        println!("Sending message: {:?}", message);

        let Some(source_filter) = message.attributes.source.as_ref() else {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "UMessage provided with no source",
            ));
        };

        let sink_filter = message.attributes.sink.as_ref();

        let message_type = determine_registration_type(source_filter, &sink_filter.cloned())?;
        let client_id = match message_type {
            RegistrationType::Publish(client_id) => client_id,
            RegistrationType::Request(client_id) => client_id,
            RegistrationType::Response(client_id) => client_id,
            RegistrationType::AllPointToPoint(client_id) => client_id,
        };

        let app_name = {
            let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
            if let Some(app_name) = client_id_app_mapping.get(&client_id) {
                Ok(app_name.clone())
            } else {
                Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("There was no app_name found for client_id: {}", client_id),
                ))
            }
        };

        if app_name.is_err() {
            let client_id = match message_type {
                RegistrationType::Publish(client_id) => client_id,
                RegistrationType::Request(client_id) => client_id,
                RegistrationType::Response(client_id) => client_id,
                RegistrationType::AllPointToPoint(client_id) => client_id,
            };

            let app_name = format!("{}_{}", self.authority_name, client_id);

            // consider using a worker pool for these, otherwise this will block
            let (tx, rx) = oneshot::channel();
            trace!(
                "{}:{} - Sending TransportCommand for InitializeNewApp",
                UP_CLIENT_VSOMEIP_TAG,
                UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER
            );
            let _tx_res = self
                .tx_to_event_loop
                .send(TransportCommand::InitializeNewApp(client_id, app_name, tx))
                .await;
            let app_created_res =
                await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL, rx)
                    .await?;
        }

        let client_id = match message_type {
            RegistrationType::Publish(client_id) => client_id,
            RegistrationType::Request(client_id) => client_id,
            RegistrationType::Response(client_id) => client_id,
            RegistrationType::AllPointToPoint(client_id) => client_id,
        };

        let app_name = format!("{}_{}", self.authority_name, client_id);

        let (tx, rx) = oneshot::channel();
        // consider using a worker pool for these, otherwise this will block
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::Send(message, app_name.to_string(), tx))
            .await;
        await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, rx).await
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

        trace!("registration_type: {registration_type:?}");

        if registration_type == RegistrationType::AllPointToPoint(0xFFFF) {
            let Some(config_path) = &self.config_path else {
                let err_msg = "No path to a vsomeip config file was provided";
                error!("{err_msg}");
                return Err(UStatus::fail_with_code(UCode::NOT_FOUND, err_msg));
            };

            let application_configs = extract_applications(config_path)?;
            trace!("Got vsomeip application_configs: {application_configs:?}");

            let mut point_to_point_listener = self.point_to_point_listener.lock().await;
            if point_to_point_listener.is_some() {
                return Err(UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS,
                    "We already have a point-to-point UListener registered",
                ));
            }
            *point_to_point_listener = Some(listener.clone());
            trace!("We found a point-to-point listener and set it");

            for app_config in &application_configs {
                let (tx, rx) = oneshot::channel();
                let tx_res = self
                    .tx_to_event_loop
                    .send(TransportCommand::InitializeNewApp(
                        app_config.id,
                        app_config.name.clone(),
                        tx,
                    ))
                    .await;
                await_internal_function(
                    "Initializing point-to-point listener apps. ApplicationConfig: {app_config:?}",
                    rx,
                )
                .await?;
            }

            // TODO: Read the config file & then for each instance of application go ahead and
            //  start a new vsomeip application by sending an InitializeNewApp command
            // TODO: Goal is to check whether we can "get away with" setting up just the application
            //  array atm and confirm that the client_id set for the application matches the id
            //  we set in the vsomeip config file
            // let _tx_res = self
            //     .tx_to_event_loop
            //     .send(TransportCommand::InitializeNewApp(client_id, app_name, tx))
            //     .await;

            return Ok(());

            // let mut point_to_point_listeners = POINT_TO_POINT_LISTENERS.lock().unwrap();
            // point_to_point_listeners.insert(listener_id);
        }

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

        let comp_listener = ComparableListener::new(Arc::clone(&listener));
        let key = (source_filter.clone(), sink_filter.cloned(), comp_listener);

        {
            let mut id_map = LISTENER_ID_MAP.lock().unwrap();
            id_map.insert(key, listener_id);
        }

        trace!("Inserted into LISTENER_ID_MAP");

        // TODO: Need to do some verification on returned Option<>
        LISTENER_REGISTRY
            .lock()
            .unwrap()
            .insert(listener_id, listener);

        // TODO: Ask for initialization of app if not initialized yet
        let app_name = {
            let listener_client_id_mapping = LISTENER_CLIENT_ID_MAPPING.lock().unwrap();
            if let Some(client_id) = listener_client_id_mapping.get(&listener_id) {
                let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
                if let Some(app_name) = client_id_app_mapping.get(client_id) {
                    Ok(app_name.clone())
                } else {
                    Err(UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!(
                            "There was no app_name found for listener_id: {} and client_id: {}",
                            listener_id, client_id
                        ),
                    ))
                }
            } else {
                Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!(
                        "There was no client_id found for listener_id: {}",
                        listener_id
                    ),
                ))
            }
        };

        let extern_fn = get_extern_fn(listener_id);
        let msg_handler = MessageHandlerFnPtr(extern_fn);
        let src = source_filter.clone();
        let sink = sink_filter.cloned();

        trace!("Obtained extern_fn");

        if app_name.is_err() {
            let client_id = match registration_type {
                RegistrationType::Publish(client_id) => client_id,
                RegistrationType::Request(client_id) => client_id,
                RegistrationType::Response(client_id) => client_id,
                RegistrationType::AllPointToPoint(client_id) => client_id,
            };

            let app_name = format!("{}_{}", self.authority_name, client_id);

            // consider using a worker pool for these, otherwise this will block
            let (tx, rx) = oneshot::channel();
            trace!(
                "{}:{} - Sending TransportCommand for InitializeNewApp",
                UP_CLIENT_VSOMEIP_TAG,
                UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER
            );
            let _tx_res = self
                .tx_to_event_loop
                .send(TransportCommand::InitializeNewApp(client_id, app_name, tx))
                .await;
            let app_created_res =
                await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL, rx)
                    .await;
            match app_created_res {
                Ok(_) => {
                    let mut listener_client_id_mapping = LISTENER_CLIENT_ID_MAPPING.lock().unwrap();
                    listener_client_id_mapping.insert(listener_id, client_id);
                }
                Err(_) => {
                    // TODO: Add logging
                }
            }
        }

        let client_id = match registration_type {
            RegistrationType::Publish(client_id) => client_id,
            RegistrationType::Request(client_id) => client_id,
            RegistrationType::Response(client_id) => client_id,
            RegistrationType::AllPointToPoint(client_id) => client_id,
        };

        let app_name = format!("{}_{}", self.authority_name, client_id);

        // consider using a worker pool for these, otherwise this will block
        let (tx, rx) = oneshot::channel();
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::RegisterListener(
                src,
                sink,
                msg_handler,
                app_name,
                tx,
            ))
            .await;
        await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL, rx).await
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

        let client_id = match registration_type {
            RegistrationType::Publish(client_id) => client_id,
            RegistrationType::Request(client_id) => client_id,
            RegistrationType::Response(client_id) => client_id,
            RegistrationType::AllPointToPoint(client_id) => client_id,
        };

        let app_name = {
            let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
            if let Some(app_name) = client_id_app_mapping.get(&client_id) {
                Ok(app_name.clone())
            } else {
                Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("There was no app_name found for client_id: {}", client_id),
                ))
            }
        }?;

        let comp_listener = ComparableListener::new(listener);

        // consider using a worker pool for these, otherwise this will block
        let (tx, rx) = oneshot::channel();
        let _tx_res = self
            .tx_to_event_loop
            .send(TransportCommand::UnregisterListener(
                src,
                sink,
                app_name.to_string(),
                tx,
            ))
            .await;
        await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL, rx).await?;

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

        {
            let mut client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
            client_id_app_mapping.remove(&client_id);
        }

        {
            let mut listener_client_id_mapping = LISTENER_CLIENT_ID_MAPPING.lock().unwrap();
            listener_client_id_mapping.remove(&listener_id);
        }

        if registration_type == RegistrationType::AllPointToPoint(0xFFFF) {
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
