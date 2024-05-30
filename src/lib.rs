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
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;

use up_rust::{UCode, UMessage, UMessageType, UStatus, UUri};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{make_application_wrapper, make_runtime_wrapper, ApplicationWrapper, MessageWrapper, RuntimeWrapper, make_message_wrapper};
use vsomeip_sys::safe_glue::{get_pinned_application, get_pinned_runtime};
use vsomeip_sys::vsomeip;

pub mod transport;

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

lazy_static! {
    static ref APP_NAME: OnceLock<String> = OnceLock::new();
}

pub struct UPClientVsomeip {
    // we're going to be using this for error messages, so suppress this warning for now
    #[allow(dead_code)]
    app_name: String,
    tx_to_event_loop: Sender<TransportCommand>,
}

impl UPClientVsomeip {
    pub fn new_with_config(app_name: &str, config_path: &Path) -> Result<Self, UStatus> {
        if !config_path.exists() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Configuration file not found at: {:?}", config_path),
            ));
        }

        Self::new_internal(app_name, Some(config_path))
    }

    pub fn new(app_name: &str) -> Result<Self, UStatus> {
        Self::new_internal(app_name, None)
    }

    fn new_internal(app_name: &str, config_path: Option<&Path>) -> Result<Self, UStatus> {
        let (tx, rx) = channel(10000);

        APP_NAME
            .set(app_name.to_string())
            .expect("APP_NAME was already set!");

        Self::app_event_loop(app_name, rx, config_path);

        Ok(Self {
            app_name: app_name.parse().unwrap(),
            tx_to_event_loop: tx,
        })
    }

    fn get_app_name() -> &'static String {
        APP_NAME.get().expect("APP_NAME has not been initialized")
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
        get_pinned_application(&_application_wrapper).send(msg_to_send);

        Self::return_oneshot_value(
            Ok(()),
            _return_channel,
        )
        .await;
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
    _vsomeip_message: &UniquePtr<MessageWrapper>,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> UMessage {
    // Implementation goes here
    UMessage::default()
}

fn convert_umsg_to_vsomeip_msg(
    _umsg: &UMessage,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UniquePtr<MessageWrapper>, UStatus> {

    match _umsg.attributes.type_.enum_value_or(UMessageType::UMESSAGE_TYPE_UNSPECIFIED) {
        UMessageType::UMESSAGE_TYPE_PUBLISH => {
            // Implementation goes here
            let vsomeip_msg = make_message_wrapper(get_pinned_runtime(&_runtime_wrapper).create_request(true));

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_REQUEST => {
            // Implementation goes here
            let vsomeip_msg = make_message_wrapper(get_pinned_runtime(&_runtime_wrapper).create_request(true));

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_RESPONSE => {
            // Implementation goes here
            // TODO -- this should be create_response, but that takes a &SharedPtr<message> which
            //  should be the original request message
            let vsomeip_msg = make_message_wrapper(get_pinned_runtime(&_runtime_wrapper).create_request(true));

            Ok(vsomeip_msg)
        }
        _ => {
            return Err(UStatus::fail_with_code(UCode::INTERNAL, "Trying to convert an unspecified or notification message type."));
        }
    }


}
