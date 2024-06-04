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

use cxx::{let_cxx_string, UniquePtr};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;

use log::{error, info, trace};

use up_rust::{UCode, UMessage, UMessageType, UStatus, UUri, UUID};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{
    make_application_wrapper, make_runtime_wrapper, ApplicationWrapper, RuntimeWrapper,
};
use vsomeip_sys::safe_glue::{
    get_pinned_application, get_pinned_message_base, get_pinned_runtime,
    register_message_handler_fn_ptr_safe,
};
use vsomeip_sys::vsomeip;

pub mod transport;

use transport::{AUTHORITY_NAME, CLIENT_ID_APP_MAPPING};

mod message_conversions;

use message_conversions::convert_umsg_to_vsomeip_msg;
mod determinations;

use determinations::{
    create_request_id, determine_registration_type, retrieve_session_id, split_u32_to_u16,
    split_u32_to_u8,
};

const UP_CLIENT_VSOMEIP_TAG: &str = "UPClientVsomeip";
const UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP: &str = "app_event_loop";
const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL: &str = "register_listener_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL: &str = "unregister_listener_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL: &str = "send_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL: &str = "initialize_new_app_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_START_APP: &str = "start_app";

const ME_AUTHORITY: &str = "me_authority";

enum TransportCommand {
    // Primary purpose of a UTransport
    RegisterListener(
        UUri,
        Option<UUri>,
        MessageHandlerFnPtr,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    UnregisterListener(
        UUri,
        Option<UUri>,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    Send(
        UMessage,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    // Additional helpful commands
    InitializeNewApp(
        ClientId,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
}

type ApplicationName = String;
type AuthorityName = String;
type UeId = u16;
type ClientId = u16;
type ReqId = UUID;
type SessionId = u16;
type RequestId = u32;

#[derive(PartialEq)]
enum RegistrationType {
    Publish(ClientId),
    Request(ClientId),
    Response(ClientId),
    AllPointToPoint(ClientId),
}

pub struct UPClientVsomeip {
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    authority_name: AuthorityName,
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    ue_id: UeId,
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    config_path: Option<PathBuf>,
    tx_to_event_loop: Sender<TransportCommand>,
}

impl UPClientVsomeip {
    pub fn new_with_config(
        authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: &Path,
    ) -> Result<Self, UStatus> {
        if !config_path.exists() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Configuration file not found at: {:?}", config_path),
            ));
        }

        Self::new_internal(authority_name, ue_id, Some(config_path))
    }

    pub fn new(authority_name: &AuthorityName, ue_id: UeId) -> Result<Self, UStatus> {
        Self::new_internal(authority_name, ue_id, None)
    }

    fn new_internal(
        authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: Option<&Path>,
    ) -> Result<Self, UStatus> {
        let (tx, rx) = channel(10000);

        let mut authority_name_static = AUTHORITY_NAME.lock().unwrap();
        *authority_name_static = authority_name.to_string();

        Self::app_event_loop(rx, config_path);

        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        Ok(Self {
            authority_name: authority_name.to_string(),
            ue_id,
            tx_to_event_loop: tx,
            config_path,
        })
    }

    fn start_app(app_name: &ApplicationName, config_path: Option<&Path>) {
        let app_name = app_name.to_string();
        let config_path = config_path.map(|p| p.to_path_buf());

        thread::spawn(move || {
            trace!(
                "{}:{} - Within start_app, spawned dedicated thread to park app",
                UP_CLIENT_VSOMEIP_TAG,
                UP_CLIENT_VSOMEIP_FN_TAG_START_APP
            );
            let config_path = config_path.map(|p| p.to_path_buf());
            let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());
            let_cxx_string!(app_name_cxx = app_name);
            // TODO: Add some additional checks to ensure we succeeded, e.g. check not null pointer
            //  This is probably best handled at a lower level in vsomeip-sys
            //  and surfacing a new API
            let application_wrapper = {
                if let Some(config_path) = config_path {
                    let config_path_str = config_path.display().to_string();
                    let_cxx_string!(config_path_cxx_str = config_path_str);
                    make_application_wrapper(
                        get_pinned_runtime(&runtime_wrapper)
                            .create_application1(&app_name_cxx, &config_path_cxx_str),
                    )
                } else {
                    make_application_wrapper(
                        get_pinned_runtime(&runtime_wrapper).create_application(&app_name_cxx),
                    )
                }
            };

            get_pinned_application(&application_wrapper).init();
            // thread is blocked by vsomeip here
            get_pinned_application(&application_wrapper).start();
        });

        trace!(
            "{}:{} - Made it past creating app and starting it on dedicated thread",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_START_APP
        );

        // TODO: Should be removed in favor of a signal-based strategy
        thread::sleep(Duration::from_millis(500));
    }

    fn app_event_loop(
        mut rx_to_event_loop: Receiver<TransportCommand>,
        config_path: Option<&Path>,
    ) {
        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        thread::spawn(move || {
            trace!("On dedicated thread");

            // Create a new single-threaded runtime
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime");

            runtime.block_on(async move {
                // let mut client_application_mapping: HashMap<ClientId, UniquePtr<ApplicationWrapper>> = HashMap::new();

                trace!("Within blocked runtime");

                let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());

                trace!("Entering command loop");
                while let Some(cmd) = rx_to_event_loop.recv().await {
                    trace!("Received TransportCommand");

                    match cmd {
                        TransportCommand::RegisterListener(
                            src,
                            sink,
                            msg_handler,
                            application_name,
                            return_channel,
                        ) => {
                            trace!(
                                "{}:{} - Attempting to register listener",
                                UP_CLIENT_VSOMEIP_TAG,
                                UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP
                            );

                            let registration_type = determine_registration_type(&src, &sink);

                            match registration_type {
                                Ok(registration_type) => {

                                    let client_id = match registration_type {
                                        RegistrationType::Publish(client_id) => {client_id}
                                        RegistrationType::Request(client_id) => {client_id}
                                        RegistrationType::Response(client_id) => {client_id}
                                        RegistrationType::AllPointToPoint(client_id) => {client_id}
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

                                    let Ok(app_name) = app_name else {
                                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel).await;
                                        continue;
                                    };

                                    let_cxx_string!(app_name_cxx = app_name);

                                    let mut application_wrapper = make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx));

                                    let res = Self::register_listener_internal(
                                        &application_name,
                                        src,
                                        sink,
                                        msg_handler,
                                        &mut application_wrapper,
                                        &runtime_wrapper,
                                    )
                                    .await;
                                    trace!("Sending back results of registration");
                                    Self::return_oneshot_result(res, return_channel).await;
                                    trace!("Sent back results of registration");
                                    continue;
                                }
                                Err(e) => {
                                    Self::return_oneshot_result(Err(e), return_channel).await;
                                    continue;
                                }
                            }
                        }
                        TransportCommand::UnregisterListener(src, sink, application_name, return_channel) => {

                            // TODO: Extract ApplicationWrapper from hashmap

                            let registration_type = determine_registration_type(&src, &sink);

                            match registration_type {
                                Ok(registration_type) => {

                                    let client_id = match registration_type {
                                        RegistrationType::Publish(client_id) => {client_id}
                                        RegistrationType::Request(client_id) => {client_id}
                                        RegistrationType::Response(client_id) => {client_id}
                                        RegistrationType::AllPointToPoint(client_id) => {client_id}
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

                                    let Ok(app_name) = app_name else {
                                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel).await;
                                        continue;
                                    };

                                    let_cxx_string!(app_name_cxx = app_name);

                                    let application_wrapper = make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx));

                                    Self::unregister_listener_internal(
                                        &application_name,
                                        src,
                                        sink,
                                        return_channel,
                                        &application_wrapper,
                                        &runtime_wrapper,
                                    )
                                    .await
                                }
                                Err(e) => {
                                    Self::return_oneshot_result(Err(e), return_channel).await;
                                    continue;
                                }
                            }
                        }
                        TransportCommand::Send(umsg, application_name, return_channel) => {
                            trace!(
                                "{}:{} - Attempting to send UMessage: {:?}",
                                UP_CLIENT_VSOMEIP_TAG,
                                UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                                umsg
                            );

                            let Some(source_filter) = umsg.attributes.source.as_ref() else {
                                Self::return_oneshot_result(Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "UMessage provided with no source")), return_channel).await;
                                continue;
                            };

                            let sink_filter = umsg.attributes.sink.as_ref();

                            let message_type = determine_registration_type(source_filter, &sink_filter.cloned());

                            match message_type {
                                Ok(registration_type) => {

                                    let client_id = match registration_type {
                                        RegistrationType::Publish(client_id) => {client_id}
                                        RegistrationType::Request(client_id) => {client_id}
                                        RegistrationType::Response(client_id) => {client_id}
                                        RegistrationType::AllPointToPoint(client_id) => {client_id}
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

                                    let Ok(app_name) = app_name else {
                                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel).await;
                                        continue;
                                    };

                                    let_cxx_string!(app_name_cxx = app_name);

                                    let application_wrapper = make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx));

                                    Self::send_internal(
                                        &application_name,
                                        umsg,
                                        return_channel,
                                        &application_wrapper,
                                        &runtime_wrapper,
                                    )
                                    .await
                                }
                                Err(e) => {
                                    Self::return_oneshot_result(Err(e), return_channel).await;
                                    continue;
                                }
                            }
                        }
                        TransportCommand::InitializeNewApp(client_id, app_name, return_channel) => {
                            trace!("{}:{} - Attempting to initialize new app for client_id: {} app_name: {}",
                                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                                client_id, app_name
                            );
                            let new_app_res = Self::initialize_new_app_internal(client_id, app_name.clone(), config_path.clone(), &runtime_wrapper).await;
                            trace!("{}:{} - Created new app for client_id: {} app_name: {}",
                                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                                client_id, app_name
                            );

                            match new_app_res {
                                Ok(_app) => {
                                    let mut client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
                                    if !client_id_app_mapping.contains_key(&client_id) {
                                        if client_id_app_mapping.insert(client_id, app_name.clone()).is_some() {
                                            let err_msg = format!("Insertion showed key existsed. Somehow we already had an application running for client_id: {}", client_id);
                                            let err = UStatus::fail_with_code(UCode::ALREADY_EXISTS, err_msg);
                                            Self::return_oneshot_result(Err(err), return_channel).await;
                                            continue;
                                        } else {
                                            trace!("Inserted client_id: {} and app_name: {} into CLIENT_ID_APP_MAPPING", client_id, app_name);
                                            Self::return_oneshot_result(Ok(()), return_channel).await;
                                            continue;
                                        }
                                    } else {
                                        let err_msg = format!("Already had key. Somehow we already had an application running for client_id: {}", client_id);
                                        let err = UStatus::fail_with_code(UCode::ALREADY_EXISTS, err_msg);
                                        Self::return_oneshot_result(Err(err), return_channel).await;
                                        continue;
                                    }


                                    // if !client_application_mapping.contains_key(&client_id) {
                                    //     if client_application_mapping.insert(client_id, app).is_some() {
                                    //         let err_msg = format!("Somehow we already had an application running for client_id: {}", client_id);
                                    //         let err = UStatus::fail_with_code(UCode::ALREADY_EXISTS, err_msg);
                                    //         Self::return_oneshot_result(Err(err), return_channel).await;
                                    //         continue;
                                    //     } else {
                                    //         trace!("Inserted client_id: {} and application into event loop hashmap", client_id);
                                    //         Self::return_oneshot_result(Ok(()), return_channel).await;
                                    //         continue;
                                    //     }
                                    // } else {
                                    //     let err_msg = format!("There already exists an application for client_id: {}", client_id);
                                    //     let err = UStatus::fail_with_code(UCode::ALREADY_EXISTS, err_msg);
                                    //     Self::return_oneshot_result(Err(err), return_channel).await;
                                    //     continue;
                                    //     // TODO: Need to decide on behavior when there was already
                                    //     //  an application running with that client ID
                                    // }
                                }
                                Err(err) => {
                                    error!("Unable to create new app: {:?}", err);
                                    Self::return_oneshot_result(Err(err), return_channel).await;
                                    continue;
                                }
                            }
                        }
                    }
                    trace!("Hit bottom of event loop");
                }
                info!("Broke out of loop! You probably dropped the UPClientVsomeip");
            });
            trace!("Parking dedicated thread");
            thread::park();
            trace!("Made past dedicated thread park");
        });
        trace!("Past thread spawn");
    }

    async fn register_listener_internal(
        _app_name: &str,
        _source_filter: UUri,
        _sink_filter: Option<UUri>,
        _msg_handler: MessageHandlerFnPtr,
        _application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<(), UStatus> {
        // TODO: May want to add additional validation up here on the UUri filters

        trace!(
            "{}:{} - Attempting to register: source_filter: {:?} & sink_filter: {:?}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
            _source_filter,
            _sink_filter
        );

        let registration_type_res = determine_registration_type(&_source_filter, &_sink_filter);
        let Ok(registration_type) = registration_type_res else {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "Unable to map source and sink filters to uProtocol message type",
            ));
        };
        match registration_type {
            RegistrationType::Publish(_) => {
                trace!(
                    "{}:{} - Registering for Publish style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let (_, service_id) = split_u32_to_u16(_source_filter.ue_id);
                // let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let instance_id = 1;
                let (_, method_id) = split_u32_to_u16(_source_filter.resource_id);

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    method_id
                );

                get_pinned_application(_application_wrapper).offer_service(
                    service_id,
                    instance_id,
                    vsomeip::ANY_MAJOR,
                    vsomeip::ANY_MINOR,
                );

                register_message_handler_fn_ptr_safe(
                    _application_wrapper,
                    service_id,
                    instance_id,
                    method_id,
                    _msg_handler,
                );

                trace!(
                    "{}:{} - Registered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::Request(_) => {
                trace!(
                    "{}:{} - Registering for Request style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let Some(sink_filter) = _sink_filter else {
                    return Err(UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        "Unable to map source and sink filters to uProtocol message type",
                    ));
                };

                let (_, service_id) = split_u32_to_u16(sink_filter.ue_id);
                let instance_id = 1; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(sink_filter.resource_id);

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    method_id
                );

                get_pinned_application(_application_wrapper).offer_service(
                    service_id,
                    instance_id,
                    vsomeip::ANY_MAJOR,
                    vsomeip::ANY_MINOR,
                );

                register_message_handler_fn_ptr_safe(
                    _application_wrapper,
                    service_id,
                    vsomeip::ANY_INSTANCE,
                    method_id,
                    _msg_handler,
                );

                trace!(
                    "{}:{} - Registered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::Response(_) => {
                trace!(
                    "{}:{} - Registering for Response style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                // TODO: According to the vsomeip in 10 minutes example, don't we need to actually
                //  set this to the source?
                let (_, service_id) = split_u32_to_u16(_source_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(_source_filter.resource_id);

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    method_id
                );

                register_message_handler_fn_ptr_safe(
                    _application_wrapper,
                    service_id,
                    instance_id,
                    method_id,
                    _msg_handler,
                );

                trace!(
                    "{}:{} - Registered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::AllPointToPoint(_) => {
                // Note that there's another layer of indirection needed here, as when we register for
                //  "all" we then need to ensure that what we have received is strictly one of:
                //    * REQUEST
                //    * RESPONSE
                //    * ERROR
                //  Or in other words, we want to avoid calling the callback we were given with
                //   a SOME/IP NOTIFICATION
                // This indirection is handled within the code generated by proc macro

                trace!(
                    "{}:{} - Registering for all point-to-point style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                let service_id = vsomeip::ANY_SERVICE;
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let method_id = vsomeip::ANY_METHOD;

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    method_id
                );

                register_message_handler_fn_ptr_safe(
                    _application_wrapper,
                    service_id,
                    instance_id,
                    method_id,
                    _msg_handler,
                );

                trace!(
                    "{}:{} - Registered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
        }
    }

    async fn unregister_listener_internal(
        _app_name: &str,
        _source_filter: UUri,
        _sink_filter: Option<UUri>,
        _return_channel: oneshot::Sender<Result<(), UStatus>>,
        _application_wrapper: &UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) {
        // Implementation goes here
        trace!(
            "{}:{} - Attempting to unregister: source_filter: {:?} & sink_filter: {:?}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
            _source_filter,
            _sink_filter
        );

        let registration_type_res = determine_registration_type(&_source_filter, &_sink_filter);
        let Ok(registration_type) = registration_type_res else {
            Self::return_oneshot_result(
                Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Unable to map source and sink filters to uProtocol message type",
                )),
                _return_channel,
            )
            .await;
            return;
        };

        match registration_type {
            RegistrationType::Publish(_) => {
                trace!(
                    "{}:{} - Unregistering for Publish style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let (_, service_id) = split_u32_to_u16(_source_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(_source_filter.resource_id);

                get_pinned_application(_application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
            RegistrationType::Request(_) => {
                trace!(
                    "{}:{} - Unregistering for Request style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let Some(sink_filter) = _sink_filter else {
                    Self::return_oneshot_result(
                        Err(UStatus::fail_with_code(
                            UCode::INVALID_ARGUMENT,
                            "Request doesn't contain sink",
                        )),
                        _return_channel,
                    )
                    .await;
                    return;
                };

                let (_, service_id) = split_u32_to_u16(sink_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(sink_filter.resource_id);

                get_pinned_application(_application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
            RegistrationType::Response(_) => {
                trace!(
                    "{}:{} - Unregistering for Response style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let Some(sink_filter) = _sink_filter else {
                    Self::return_oneshot_result(
                        Err(UStatus::fail_with_code(
                            UCode::INVALID_ARGUMENT,
                            "Request doesn't contain sink",
                        )),
                        _return_channel,
                    )
                    .await;
                    return;
                };

                let (_, service_id) = split_u32_to_u16(sink_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(sink_filter.resource_id);

                get_pinned_application(_application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
            RegistrationType::AllPointToPoint(_) => {
                // Note that there's another layer of indirection needed here, as when we register for
                //  "all" we then need to ensure that what we have received is strictly one of:
                //    * REQUEST
                //    * RESPONSE
                //    * ERROR
                //  Or in other words, we want to avoid calling the callback we were given with
                //   a SOME/IP NOTIFICATION
                // This indirection is handled within the code generated by proc macro

                trace!(
                    "{}:{} - Unregistering for all point-to-point style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                let service_id = vsomeip::ANY_SERVICE;
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let method_id = vsomeip::ANY_METHOD;

                get_pinned_application(_application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
        }
    }

    async fn send_internal(
        _app_name: &str,
        umsg: UMessage,
        _return_channel: oneshot::Sender<Result<(), UStatus>>,
        _application_wrapper: &UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) {
        match umsg
            .attributes
            .type_
            .enum_value_or(UMessageType::UMESSAGE_TYPE_UNSPECIFIED)
        {
            UMessageType::UMESSAGE_TYPE_UNSPECIFIED => {
                Self::return_oneshot_result(
                    Err(UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        "Unspecified message type not supported",
                    )),
                    _return_channel,
                )
                .await;
                return;
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                Self::return_oneshot_result(
                    Err(UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        "Notification is not supported",
                    )),
                    _return_channel,
                )
                .await;
                return;
            }
            UMessageType::UMESSAGE_TYPE_PUBLISH
            | UMessageType::UMESSAGE_TYPE_REQUEST
            | UMessageType::UMESSAGE_TYPE_RESPONSE => {
                // TODO: Add logging that we succeeded

                let vsomeip_msg_res =
                    convert_umsg_to_vsomeip_msg(&umsg, _application_wrapper, _runtime_wrapper);

                match vsomeip_msg_res {
                    Ok(vsomeip_msg) => {
                        let service_id = get_pinned_message_base(&vsomeip_msg).get_service();
                        let instance_id = get_pinned_message_base(&vsomeip_msg).get_instance();
                        let method_id = get_pinned_message_base(&vsomeip_msg).get_method();
                        let _interface_version =
                            get_pinned_message_base(&vsomeip_msg).get_interface_version();

                        // trace!("Immediately prior to offer_service");

                        // TODO: May want to do this the one time when we first send over this
                        //  service and understand we already offered the service, so no need
                        //  to repeatedly do it
                        // get_pinned_application(&local_app_wrapper).offer_service(
                        //     service_id,
                        //     instance_id,
                        //     interface_version,
                        //     vsomeip::ANY_MINOR,
                        // );

                        // trace!("Immediately after offer_service");

                        tokio::time::sleep(Duration::from_millis(100)).await;

                        trace!("{}:{} Sending SOME/IP message with service: {} instance: {} method: {}",
                            UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                            service_id, instance_id, method_id
                        );

                        // TODO: Add logging here that we succeeded
                        let shared_ptr_message = vsomeip_msg.as_ref().unwrap().get_shared_ptr();
                        get_pinned_application(_application_wrapper).send(shared_ptr_message);
                    }
                    Err(err) => {
                        error!(
                            "{}:{} Converting UMessage to vsomeip message failed: {:?}",
                            UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, err
                        );
                    }
                }
            }
        }

        Self::return_oneshot_result(Ok(()), _return_channel).await;
    }

    async fn return_oneshot_result(
        result: Result<(), UStatus>,
        tx: oneshot::Sender<Result<(), UStatus>>,
    ) {
        if let Err(_err) = tx.send(result) {
            // TODO: Add logging here that we couldn't return a result
        }
    }

    async fn initialize_new_app_internal(
        client_id: ClientId,
        app_name: ApplicationName,
        config_path: Option<PathBuf>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<UniquePtr<ApplicationWrapper>, UStatus> {
        trace!(
            "{}:{} - Attempting to initialize new app for client_id: {} app_name: {}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
            client_id,
            app_name
        );
        let existing_app = {
            let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
            client_id_app_mapping.contains_key(&client_id)
        };

        if existing_app {
            let err = UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "Application already exists for client_id: {} app_name: {}",
                    client_id, app_name
                ),
            );
            return Err(err);
        }

        // TODO: Should this be fallible?
        Self::start_app(&app_name, config_path.as_deref());
        trace!(
            "{}:{} - After starting app for client_id: {} app_name: {}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
            client_id,
            app_name
        );

        let_cxx_string!(app_name_cxx = app_name);

        let app_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
        );
        Ok(app_wrapper)
    }
}

// TODO: We need to ensure that we properly cleanup / unregister all message handlers
//  and then remote the application
// impl Drop for UPClientVsomeip {
//     fn drop(&mut self) {
//         todo!()
//     }
// }
