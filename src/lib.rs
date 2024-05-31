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
use lazy_static::lazy_static;
use protobuf::Enum;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::thread;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;

use up_rust::{
    UCode, UMessage, UMessageBuilder, UMessageType, UPayloadFormat, UStatus, UUIDBuilder, UUri,
    UUID,
};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{
    make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
    ApplicationWrapper, MessageWrapper, RuntimeWrapper,
};
use vsomeip_sys::safe_glue::{
    get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base,
    get_pinned_payload, get_pinned_runtime, set_data_safe, set_message_payload,
};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::message_type_e;

pub mod transport;

const ME_AUTHORITY: &str = "me_authority";

enum TransportCommand {
    RegisterListener(
        UUri,
        Option<UUri>,
        MessageHandlerFnPtr,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    UnregisterListener(UUri, Option<UUri>, oneshot::Sender<Result<(), UStatus>>),
    Send(UMessage, oneshot::Sender<Result<(), UStatus>>),
}

type ClientId = u16;
type ReqId = UUID;
type SessionId = u16;

lazy_static! {
    static ref APP_NAME: OnceLock<String> = OnceLock::new();
    static ref AUTHORITY_NAME: OnceLock<String> = OnceLock::new();
    static ref UE_ID: OnceLock<u16> = OnceLock::new();
    static ref UE_REQUEST_CORRELATION: Mutex<HashMap<ClientId, ReqId>> = Mutex::new(HashMap::new());
    static ref ME_REQUEST_CORRELATION: Mutex<HashMap<ReqId, ClientId>> = Mutex::new(HashMap::new());
    static ref CLIENT_ID_SESSION_ID_TRACKING: Mutex<HashMap<ClientId, SessionId>> =
        Mutex::new(HashMap::new());
}

pub struct UPClientVsomeip {
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    app_name: String,
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    authority_name: String,
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    ue_id: u16,
    tx_to_event_loop: Sender<TransportCommand>,
}

impl UPClientVsomeip {
    pub fn new_with_config(
        app_name: &str,
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

        Self::new_internal(app_name, authority_name, ue_id, Some(config_path))
    }

    pub fn new(app_name: &str, authority_name: &str, ue_id: u16) -> Result<Self, UStatus> {
        Self::new_internal(app_name, authority_name, ue_id, None)
    }

    fn new_internal(
        app_name: &str,
        authority_name: &str,
        ue_id: u16,
        config_path: Option<&Path>,
    ) -> Result<Self, UStatus> {
        let (tx, rx) = channel(10000);

        APP_NAME
            .set(app_name.to_string())
            .expect("APP_NAME was already set!");

        AUTHORITY_NAME
            .set(authority_name.to_string())
            .expect("AUTHORITY_NAME was already set!");

        UE_ID.set(ue_id).expect("UE_ID was already set!");

        Self::app_event_loop(app_name, rx, config_path);

        Ok(Self {
            app_name: app_name.parse().unwrap(),
            authority_name: authority_name.to_string(),
            ue_id,
            tx_to_event_loop: tx,
        })
    }

    fn get_app_name() -> &'static String {
        APP_NAME.get().expect("APP_NAME has not been initialized")
    }

    fn get_authority_name() -> &'static String {
        AUTHORITY_NAME
            .get()
            .expect("APP_NAME has not been initialized")
    }

    fn app_event_loop(
        app_name: &str,
        mut rx_to_event_loop: Receiver<TransportCommand>,
        config_path: Option<&Path>,
    ) {
        let config_path = config_path.map(|p| p.to_path_buf());
        let app_name = app_name.to_string();
        thread::spawn(move || {
            // Create a new single-threaded runtime
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime");

            runtime.block_on(async move {
                let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());
                let_cxx_string!(app_name_cxx = app_name);
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
                get_pinned_application(&application_wrapper).start();

                while let Some(cmd) = rx_to_event_loop.recv().await {
                    match cmd {
                        TransportCommand::RegisterListener(
                            src,
                            sink,
                            msg_handler,
                            return_channel,
                        ) => Self::register_listener_internal(
                            src,
                            sink,
                            msg_handler,
                            return_channel,
                            &application_wrapper,
                            &runtime_wrapper,
                        ),
                        TransportCommand::UnregisterListener(src, sink, return_channel) => {
                            Self::unregister_listener_internal(
                                src,
                                sink,
                                return_channel,
                                &application_wrapper,
                                &runtime_wrapper,
                            )
                        }
                        TransportCommand::Send(umsg, return_channel) => {
                            Self::send_internal(
                                umsg,
                                return_channel,
                                &application_wrapper,
                                &runtime_wrapper,
                            )
                            .await
                        }
                    }
                }
            });
        });
    }

    fn register_listener_internal(
        _source_filter: UUri,
        _sink_filter: Option<UUri>,
        _msg_handler: MessageHandlerFnPtr,
        _return_channel: oneshot::Sender<Result<(), UStatus>>,
        _application_wrapper: &UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) {
        // Implementation goes here
        todo!()
    }

    fn unregister_listener_internal(
        _source_filter: UUri,
        _sink_filter: Option<UUri>,
        _return_channel: oneshot::Sender<Result<(), UStatus>>,
        _application_wrapper: &UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) {
        // Implementation goes here
        todo!()
    }

    async fn send_internal(
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
                Self::return_oneshot_value(
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
                Self::return_oneshot_value(
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
                // TODO: Add logging that we succeeded and proceed...
            }
        }

        // Implementation goes here
        let _vsomeip_msg =
            convert_umsg_to_vsomeip_msg(&umsg, _application_wrapper, _runtime_wrapper);

        let msg_to_send = _vsomeip_msg.as_ref().unwrap().get_shared_ptr();
        get_pinned_application(_application_wrapper).send(msg_to_send);

        Self::return_oneshot_value(Ok(()), _return_channel).await;
    }

    async fn return_oneshot_value(
        result: Result<(), UStatus>,
        tx: oneshot::Sender<Result<(), UStatus>>,
    ) {
        if let Err(_err) = tx.send(result) {
            // TODO: Add logging here that we couldn't return a result
        }
    }
}

fn convert_vsomeip_msg_to_umsg(
    _vsomeip_message: &mut UniquePtr<MessageWrapper>,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UMessage, UStatus> {
    let msg_type = get_pinned_message_base(_vsomeip_message).get_message_type();

    let service_id = get_pinned_message_base(_vsomeip_message).get_service();
    let client_id = get_pinned_message_base(_vsomeip_message).get_client();
    let method_id = get_pinned_message_base(_vsomeip_message).get_method();
    let interface_version = get_pinned_message_base(_vsomeip_message).get_interface_version();
    let payload = get_message_payload(_vsomeip_message);
    let payload_bytes = get_data_safe(&payload);

    match msg_type {
        message_type_e::MT_REQUEST => {
            let sink = UUri {
                authority_name: UPClientVsomeip::get_authority_name().to_string(),
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
            let ttl = 10;

            let umsg_res = UMessageBuilder::request(sink, source, ttl)
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
        message_type_e::MT_NOTIFICATION => {
            // TODO: Implement the logic here from the table
            let sink = UUri {
                authority_name: UPClientVsomeip::get_authority_name().to_string(),
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

            // TODO: Need to perform correlation to get this, this is just stand-in
            let req_id = UUIDBuilder::build();

            let umsg_res = UMessageBuilder::response(sink, req_id, source)
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
            let sink = UUri {
                authority_name: UPClientVsomeip::get_authority_name().to_string(),
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

            // TODO: Need to perform correlation to get this, this is just stand-in
            let req_id = UUIDBuilder::build();

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
            let sink = UUri {
                authority_name: UPClientVsomeip::get_authority_name().to_string(),
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

            // TODO: Need to perform correlation to get this, this is just stand-in
            let req_id = UUIDBuilder::build();

            // TODO: Check with Steven on if we should be passing along the payload in the error cases for Response
            // TODO: Need to do proper mapping from error code into commstatus, for now just set INTERNAL
            let umsg_res = UMessageBuilder::response(sink, req_id, source)
                .with_comm_status(UCode::INTERNAL.value())
                .build();

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

fn convert_umsg_to_vsomeip_msg(
    umsg: &UMessage,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UniquePtr<MessageWrapper>, UStatus> {
    let Some(sink) = umsg.attributes.sink.as_ref() else {
        return Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Message has no sink UUri",
        ));
    };

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
            let (_, method_id) = split_u32_to_u16(source.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            let (_, _client_id) = split_u32_to_u16(sink.ue_id);
            let client_id = 0; // manually setting this to 0 as according to spec
            get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            let session_id = retrieve_session_id(client_id);
            get_pinned_message_base(&vsomeip_msg).set_session(session_id);
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

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_REQUEST => {
            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_request(true));
            let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let (_, method_id) = split_u32_to_u16(sink.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            let (_, client_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            // TODO: When the SOME/IP RESPONSE is crafted within the mE, does it increment the session ID?
            let session_id = retrieve_session_id(client_id);
            get_pinned_message_base(&vsomeip_msg).set_session(session_id);
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

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_RESPONSE => {
            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_message(true));

            let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let (_, method_id) = split_u32_to_u16(sink.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            let (_, client_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            let session_id = retrieve_session_id(client_id);
            get_pinned_message_base(&vsomeip_msg).set_session(session_id);
            let ok = {
                if let Some(commstatus) = umsg.attributes.commstatus {
                    let commstatus = commstatus.enum_value_or(UCode::UNIMPLEMENTED);
                    commstatus == UCode::OK
                } else {
                    false
                }
            };
            if ok {
                get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
                get_pinned_message_base(&vsomeip_msg).set_message_type(message_type_e::MT_RESPONSE);

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
            } else {
                // TODO: Perform mapping from uProtocol UCode contained in commstatus into vsomeip::return_code_e
                get_pinned_message_base(&vsomeip_msg)
                    .set_return_code(vsomeip::return_code_e::E_NOT_OK);
                get_pinned_message_base(&vsomeip_msg)
                    .set_message_type(vsomeip::message_type_e::MT_ERROR);
            }

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

    let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(0);
    let returned_session_id = *current_sesion_id;
    *current_sesion_id += 1;
    returned_session_id
}
