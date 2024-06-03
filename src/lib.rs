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

use std::collections::HashMap;
use cxx::{let_cxx_string, UniquePtr};
use protobuf::Enum;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;

use log::{error, trace};

use up_rust::{
    UCode, UMessage, UMessageBuilder, UMessageType, UPayloadFormat, UStatus, UUri, UUID,
};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{
    make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
    ApplicationWrapper, MessageWrapper, RuntimeWrapper,
};
use vsomeip_sys::safe_glue::{
    get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base,
    get_pinned_payload, get_pinned_runtime, register_message_handler_fn_ptr_safe, set_data_safe,
    set_message_payload,
};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::message_type_e;

pub mod transport;

use transport::{
    CLIENT_ID_SESSION_ID_TRACKING, ME_REQUEST_CORRELATION,
    UE_REQUEST_CORRELATION, AUTHORITY_NAME, CLIENT_ID_APP_MAPPING
};

const UP_CLIENT_VSOMEIP_TAG: &str = "UPClientVsomeip";
const UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP: &str = "app_event_loop";
const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL: &str = "register_listener_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL: &str = "send_internal";
const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG: &str = "convert_vsomeip_msg_to_umsg";

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
    UnregisterListener(UUri, Option<UUri>, ApplicationName, oneshot::Sender<Result<(), UStatus>>),
    Send(UMessage, ApplicationName, oneshot::Sender<Result<(), UStatus>>),
    // Additional helpful commands
    InitializeNewApp(ClientId, ApplicationName, oneshot::Sender<Result<(), UStatus>>)
}

type ApplicationName = String;
type AuthorityName = String;
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

struct VsomeipAppName {
    authority_name: AuthorityName,
    client_id: ClientId,
    app_name: String
}

impl VsomeipAppName {
    pub fn new(authority_name: &AuthorityName, client_id: ClientId) -> Self {
        Self {
            authority_name: authority_name.clone(),
            client_id,
            app_name: format!("{}_{}", authority_name, client_id)
        }
    }

    pub fn app_name(&self) -> String {
        self.app_name.clone()
    }
}

pub struct UPClientVsomeip {
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    authority_name: String,
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    ue_id: u16,
    config_path: Option<PathBuf>,
    tx_to_event_loop: Sender<TransportCommand>,
}

impl UPClientVsomeip {
    pub fn new_with_config(
        authority_name: &str,
        ue_id: u16,
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

    pub fn new(authority_name: &str, ue_id: u16) -> Result<Self, UStatus> {
        Self::new_internal(authority_name, ue_id, None)
    }

    fn new_internal(
        authority_name: &str,
        ue_id: u16,
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
            config_path
        })
    }

    fn start_app(app_name: &str, config_path: Option<&Path>) {
        let app_name = app_name.to_string();
        let config_path = config_path.map(|p| p.to_path_buf());

        thread::spawn(move || {
            trace!("Within start_app, spawned dedicated thread to park app");
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

        thread::sleep(Duration::from_millis(500));
    }

    fn app_event_loop(mut rx_to_event_loop: Receiver<TransportCommand>, config_path: Option<&Path>) {
        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        thread::spawn(move || {
            trace!("On dedicated thread");

            // Create a new single-threaded runtime
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime");

            runtime.block_on(async move {
                let mut client_application_mapping: HashMap<ClientId, UniquePtr<ApplicationWrapper>> = HashMap::new();

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

                                    let application_wrapper = client_application_mapping.get_mut(&client_id);

                                    let Some(app_wrapper) = application_wrapper else {
                                        Self::return_oneshot_result(Err(UStatus::fail_with_code(UCode::NOT_FOUND, "Unable to find application")), return_channel).await;
                                        continue;
                                    };

                                    Self::register_listener_internal(
                                        &application_name,
                                        src,
                                        sink,
                                        msg_handler,
                                        return_channel,
                                        app_wrapper,
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

                                    let application_wrapper = client_application_mapping.get(&client_id);

                                    let Some(app_wrapper) = application_wrapper else {
                                        Self::return_oneshot_result(Err(UStatus::fail_with_code(UCode::NOT_FOUND, "Unable to find application")), return_channel).await;
                                        continue;
                                    };

                                    Self::unregister_listener_internal(
                                        &application_name,
                                        src,
                                        sink,
                                        return_channel,
                                        app_wrapper,
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

                                    let application_wrapper = client_application_mapping.get(&client_id);

                                    let Some(app_wrapper) = application_wrapper else {
                                        Self::return_oneshot_result(Err(UStatus::fail_with_code(UCode::NOT_FOUND, "Unable to find application")), return_channel).await;
                                        continue;
                                    };

                                    Self::send_internal(
                                        &application_name,
                                        umsg,
                                        return_channel,
                                        app_wrapper,
                                        &runtime_wrapper,
                                    )
                                    .await
                                }
                                Err(e) => {
                                    Self::return_oneshot_result(Err(e), return_channel).await;
                                    continue;
                                }
                            }

                            // TODO: Extract ApplicationWrapper from hashmap


                        }
                        TransportCommand::InitializeNewApp(client_id, app_name, return_channel) => {
                            let new_app_res = Self::initialize_new_app_internal(client_id, app_name, config_path.clone(), &runtime_wrapper, return_channel).await;

                            match new_app_res {
                                Ok(app) => {
                                    if !client_application_mapping.contains_key(&client_id) {
                                        if client_application_mapping.insert(client_id, app).is_some() {
                                            error!("Somehow we already had an application running for client_id: {}", client_id);
                                        }
                                    } else {
                                        // TODO: Need to decide on behavior when there was already
                                        //  an application running with that client ID
                                    }
                                }
                                Err(err) => {
                                    error!("Unable to create new app: {:?}", err);
                                }
                            }
                        }
                    }
                }
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
        _return_channel: oneshot::Sender<Result<(), UStatus>>,
        _application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) {
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

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
            RegistrationType::Request(_) => {
                trace!(
                    "{}:{} - Registering for Request style messages.",
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

                Self::return_oneshot_result(Ok(()), _return_channel).await;
            }
            RegistrationType::Response(_) => {
                trace!(
                    "{}:{} - Registering for Response style messages.",
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

                Self::return_oneshot_result(Ok(()), _return_channel).await;
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
                        let interface_version =
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
                        error!("{}:{} Converting UMessage to vsomeip message failed: {:?}",
                            UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                            err
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
        return_channel: oneshot::Sender<Result<(), UStatus>>
    ) -> Result<UniquePtr<ApplicationWrapper>, UStatus> {

        let existing_app = {
            let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
            client_id_app_mapping.contains_key(&client_id)
        };

        if existing_app {
            let err = UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("Application already exists for client_id: {} app_name: {}", client_id, app_name));
            Self::return_oneshot_result(Err(err.clone()), return_channel).await;
            return Err(err);
        }

        // TODO: Should this be fallible?
        Self::start_app(&app_name, config_path.as_deref());

        let_cxx_string!(app_name_cxx = app_name);

        let app_wrapper = make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx));
        Ok(app_wrapper)
    }
}

fn convert_vsomeip_msg_to_umsg(
    _vsomeip_message: &mut UniquePtr<MessageWrapper>,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UMessage, UStatus> {
    trace!("top of convert_vsomeip_msg_to_umsg");
    let msg_type = get_pinned_message_base(_vsomeip_message).get_message_type();

    let request_id = get_pinned_message_base(_vsomeip_message).get_request();
    let service_id = get_pinned_message_base(_vsomeip_message).get_service();
    let client_id = get_pinned_message_base(_vsomeip_message).get_client();
    let session_id = get_pinned_message_base(_vsomeip_message).get_session();
    let method_id = get_pinned_message_base(_vsomeip_message).get_method();
    let instance_id = get_pinned_message_base(_vsomeip_message).get_instance();
    let interface_version = get_pinned_message_base(_vsomeip_message).get_interface_version();
    let payload = get_message_payload(_vsomeip_message);
    let payload_bytes = get_data_safe(&payload);

    trace!("{}:{} - : request_id: {} client_id: {} session_id: {} service_id: {} instance_id: {} method_id: {} interface_version: {}",
        UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
        request_id, client_id, session_id, service_id, instance_id, method_id, interface_version
    );

    let authority_name = { AUTHORITY_NAME.lock().unwrap().clone() };

    trace!("unloaded all relevant info from vsomeip message");

    match msg_type {
        message_type_e::MT_REQUEST => {
            trace!("MT_REQUEST type");
            let sink = UUri {
                authority_name,
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            // TODO: Not sure where to get this
            //  Steven said Ivan posted something to a Slack thread; need to check
            //  Hmm, didn't find this. Asked Steven for help
            //  He pointed me to something about SOME/IP-SD, but not Request AFAICT
            let ttl = 1000;

            trace!("Prior to building Request");

            let umsg_res = UMessageBuilder::request(sink, source, ttl)
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            trace!("After building Request");

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.id.as_ref().unwrap();
            trace!("{}:{} - (req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                req_id.to_hyphenated_string(), request_id
            );
            let mut me_request_correlation = ME_REQUEST_CORRELATION.lock().unwrap();
            if me_request_correlation.get(req_id).is_none() {
                trace!("{}:{} - (req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                    UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                    req_id.to_hyphenated_string(), request_id
                );
                me_request_correlation.insert(req_id.clone(), request_id);
            } else {
                // TODO: What do we do if we have a duplicate, already-existing pair?
                //  Eject the previous one? Fail on this one?
            }

            Ok(umsg)
        }
        message_type_e::MT_NOTIFICATION => {
            trace!("MT_NOTIFICATION type");
            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            let umsg_res = UMessageBuilder::publish(source)
                .with_comm_status(UCode::OK.value())
                .build();

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        message_type_e::MT_RESPONSE => {
            trace!("MT_RESPONSE type");
            let sink = UUri {
                authority_name,
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };


            trace!("{}:{} - request_id to look up to correlate to req_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                request_id
            );
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            let Some(req_id) = ue_request_correlation.remove(&request_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Corresponding reqid not found for this SOME/IP RESPONSE: {}", request_id),
                ));
            };

            let umsg_res = UMessageBuilder::response(sink, req_id, source)
                .with_comm_status(UCode::OK.value())
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        message_type_e::MT_ERROR => {
            trace!("MT_ERROR type");
            let sink = UUri {
                authority_name,
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            // TODO: Need to update to use RequestId instead of ClientId
            trace!("{}:{} - request_id to look up to correlate to req_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                request_id
            );
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            let Some(req_id) = ue_request_correlation.remove(&request_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Corresponding reqid not found for this SOME/IP RESPONSE: {}", request_id),
                ));
            };

            let umsg_res = UMessageBuilder::response(sink, req_id, source)
                .with_comm_status(UCode::INTERNAL.value())
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        _ => Err(UStatus::fail_with_code(
            UCode::OUT_OF_RANGE,
            format!(
                "Not one of the handled message types from SOME/IP: {:?}",
                msg_type
            ),
        )),
    }
}

// TODO: We need to ensure that we properly cleanup / unregister all message handlers
//  and then remote the application
// impl Drop for UPClientVsomeip {
//     fn drop(&mut self) {
//         todo!()
//     }
// }

fn convert_umsg_to_vsomeip_msg(
    umsg: &UMessage,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UniquePtr<MessageWrapper>, UStatus> {
    let Some(source) = umsg.attributes.source.as_ref() else {
        return Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Message has no source UUri",
        ));
    };

    match umsg
        .attributes
        .type_
        .enum_value_or(UMessageType::UMESSAGE_TYPE_UNSPECIFIED)
    {
        UMessageType::UMESSAGE_TYPE_PUBLISH => {
            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_notification(true));
            let (_instance_id, service_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            let (_, method_id) = split_u32_to_u16(source.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            // let client_id = 0; // manually setting this to 0 as according to spec
            // get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            // let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major);
            // get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            // let session_id = retrieve_session_id(client_id);
            // get_pinned_message_base(&vsomeip_msg).set_session(session_id);
            // get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);

            trace!("Immediately prior to request_service: service_id: {} instance_id: {}", service_id, instance_id);

            get_pinned_application(_application_wrapper).request_service(
                service_id,
                instance_id,
                vsomeip::ANY_MAJOR,
                // interface_version,
                vsomeip::ANY_MINOR,
            );

            // get_pinned_application(_application_wrapper).offer_service(
            //     service_id,
            //     1,
            //     interface_version,
            //     vsomeip::ANY_MINOR,
            // );

            trace!("Immediately after request_service");

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_REQUEST => {
            let Some(sink) = umsg.attributes.sink.as_ref() else {
                return Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Message has no sink UUri",
                ));
            };

            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_request(true));
            let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id);
            trace!("{}:{} - sink.ue_id: {} source.ue_id: {} _instance_id: {} service_id:{}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                sink.ue_id, source.ue_id, _instance_id, service_id
            );
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            let (_, method_id) = split_u32_to_u16(sink.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            let (_, client_id) = split_u32_to_u16(source.ue_id);
            // get_pinned_message_base(&vsomeip_msg).set_client(client_id); // doesn't matter at all; rewritten by send()

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.id.as_ref().unwrap();
            let session_id = retrieve_session_id(client_id);
            let request_id = create_request_id(client_id, session_id);
            let app_client_id = get_pinned_application(_application_wrapper).get_client();
            let app_session_id = get_request_session_id();
            trace!("{}:{} - client_id: {} session_id: {} request_id: {} service_id: {} app_client_id: {} app_session_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                client_id, session_id, request_id, service_id, app_client_id, app_session_id
            );
            let app_request_id = create_request_id(app_client_id, app_session_id);
            trace!("{}:{} - (app_request_id, req_id) to store for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                app_request_id, req_id.to_hyphenated_string(),
            );
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            if ue_request_correlation.get(&app_request_id).is_none() {
                ue_request_correlation.insert(app_request_id, req_id.clone());
                trace!("{}:{} - (app_request_id, req_id)  inserted for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                    UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                    app_request_id, req_id.to_hyphenated_string(),
                );
            } else {
                // TODO: What do we do if we have a duplicate, already-existing pair?
                //  Eject the previous one? Fail on this one?
            }
            // get_pinned_message_base(&vsomeip_msg).set_session(session_id); // doesn't matter at all, rewritten by send()
            get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);

            let request_id = get_pinned_message_base(&vsomeip_msg).get_request();
            let service_id = get_pinned_message_base(&vsomeip_msg).get_service();
            let client_id = get_pinned_message_base(&vsomeip_msg).get_client();
            let session_id = get_pinned_message_base(&vsomeip_msg).get_session();
            let method_id = get_pinned_message_base(&vsomeip_msg).get_method();
            let instance_id = get_pinned_message_base(&vsomeip_msg).get_instance();
            let interface_version = get_pinned_message_base(&vsomeip_msg).get_interface_version();

            // let payload = get_message_payload(&vsomeip_msg);
            // let payload_bytes = get_data_safe(&payload);

            trace!("{}:{} - : request_id: {} client_id: {} session_id: {} service_id: {} instance_id: {} method_id: {} interface_version: {} app_client_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                request_id, client_id, session_id, service_id, instance_id, method_id, interface_version, app_client_id
            );

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_RESPONSE => {
            trace!("{}:{} - Attempting to send Response",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL
            );

            let Some(sink) = umsg.attributes.sink.as_ref() else {
                return Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Message has no sink UUri",
                ));
            };

            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_message(true));

            // TODO: Because these must be source to allow the response to be seen, we need to give
            //  this a tune-up, since now no longer does the service_id match what's expected on
            //  the other end.
            //
            // TODO: Will need to consider what changes to make
            // let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id); // Should this be source?
            let (_instance_id, service_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            // let (_, method_id) = split_u32_to_u16(sink.resource_id); // Should this be source?
            let (_, method_id) = split_u32_to_u16(source.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            // let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major); // Should this be source?
            let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.reqid.as_ref().unwrap();
            trace!("{}:{} - Looking up req_id from UMessage in ME_REQUEST_CORRELATION, req_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                req_id.to_hyphenated_string()
            );
            let mut me_request_correlation = ME_REQUEST_CORRELATION.lock().unwrap();
            let Some(request_id) = me_request_correlation.remove(req_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Corresponding SOME/IP Request ID not found for this Request UMessage's reqid: {}",
                            req_id.to_hyphenated_string()),
                ));
            };
            trace!("{}:{} - Found correlated request_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, request_id
            );
            let (client_id, session_id) = split_u32_to_u16(request_id);
            get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            get_pinned_message_base(&vsomeip_msg).set_session(session_id);
            trace!("{}:{} - request_id: {} client_id: {} session_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                request_id, client_id, session_id
            );
            let ok = {
                if let Some(commstatus) = umsg.attributes.commstatus {
                    let commstatus = commstatus.enum_value_or(UCode::UNIMPLEMENTED);
                    commstatus == UCode::OK
                } else {
                    false
                }
            };
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);
            if ok {
                get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
                get_pinned_message_base(&vsomeip_msg).set_message_type(message_type_e::MT_RESPONSE);
            } else {
                // TODO: Perform mapping from uProtocol UCode contained in commstatus into vsomeip::return_code_e
                get_pinned_message_base(&vsomeip_msg)
                    .set_return_code(vsomeip::return_code_e::E_NOT_OK);
                get_pinned_message_base(&vsomeip_msg).set_message_type(message_type_e::MT_ERROR);
            }

            trace!("{}:{} - Response: Finished building vsomeip message: service_id: {} instance_id: {}",
                UP_CLIENT_VSOMEIP_TAG, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
                service_id, instance_id);

            Ok(vsomeip_msg)
        }
        _ => Err(UStatus::fail_with_code(
            UCode::INTERNAL,
            "Trying to convert an unspecified or notification message type.",
        )),
    }
}

fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

fn split_u32_to_u8(value: u32) -> (u8, u8, u8, u8) {
    let byte1 = (value >> 24) as u8;
    let byte2 = (value >> 16 & 0xFF) as u8;
    let byte3 = (value >> 8 & 0xFF) as u8;
    let byte4 = (value & 0xFF) as u8;
    (byte1, byte2, byte3, byte4)
}

fn retrieve_session_id(client_id: ClientId) -> SessionId {
    let mut client_id_session_id_tracking = CLIENT_ID_SESSION_ID_TRACKING.lock().unwrap();

    trace!("retrieve_session_id: client_id: {}", client_id);
    let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(0);
    trace!("retrieve_session_id: current_session_id: {}", current_sesion_id);
    let returned_session_id = *current_sesion_id;
    trace!("retrieve_session_id: returned_session_id: {}", returned_session_id);
    *current_sesion_id += 1;
    trace!("retrieve_session_id: newly updated current_session_id: {}", current_sesion_id);
    returned_session_id
}

fn create_request_id(client_id: ClientId, session_id: SessionId) -> RequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}

// infer the type of message desired based on the filters provided
fn determine_registration_type(
    source_filter: &UUri,
    sink_filter: &Option<UUri>,
) -> Result<RegistrationType, UStatus> {
    if let Some(sink_filter) = &sink_filter {
        // determine if we're in the uStreamer use-case of capturing all point-to-point messages
        let streamer_use_case = {
            source_filter.authority_name == ME_AUTHORITY // TODO: Is this good enough? Maybe have configurable in UPClientVsomeip?
            && source_filter.ue_id == 0x0000_FFFF
            && source_filter.ue_version_major == 0xFF
            && source_filter.resource_id == 0xFFFF
            && sink_filter.authority_name == "*"
            && sink_filter.ue_id == 0x0000_FFFF
            && sink_filter.ue_version_major == 0xFF
            && sink_filter.resource_id == 0xFFFF
        };

        if streamer_use_case {
            return Ok(RegistrationType::AllPointToPoint(0xFFFF));
        }

        if sink_filter.resource_id == 0 {
            Ok(RegistrationType::Response(source_filter.ue_id as ClientId))
        } else {
            Ok(RegistrationType::Request(sink_filter.ue_id as ClientId))
        }
    } else {
        Ok(RegistrationType::Publish(0))
    }
}

fn is_point_to_point_message(_vsomeip_message: &mut UniquePtr<MessageWrapper>) -> bool {
    let msg_type = get_pinned_message_base(_vsomeip_message).get_message_type();
    matches!(
        msg_type,
        message_type_e::MT_REQUEST | message_type_e::MT_RESPONSE | message_type_e::MT_ERROR
    )
}

static SESSION_COUNTER: AtomicU16 = AtomicU16::new(1);

fn get_request_session_id() -> u16 {
    SESSION_COUNTER.fetch_add(1, Ordering::SeqCst)
}
