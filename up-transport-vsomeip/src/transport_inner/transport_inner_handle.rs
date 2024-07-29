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

use crate::determine_message_type::{
    determine_registration_type, determine_send_type, RegistrationType,
};
use crate::storage::message_handler_registry::{ClientUsage, GetMessageHandlerError};
use crate::storage::{UPTransportVsomeipInnerHandleStorage, UPTransportVsomeipStorage};
use crate::transport_inner::transport_inner_engine::{
    TransportCommand, UPTransportVsomeipInnerEngine,
};
use crate::transport_inner::{
    INTERNAL_FUNCTION_TIMEOUT, UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, UP_CLIENT_VSOMEIP_TAG,
};
use crate::utils::{any_uuri, any_uuri_fixed_authority_id};
use crate::vsomeip_config::extract_applications;
use crate::{ApplicationName, AuthorityName, ClientId, UeId};
use lazy_static::lazy_static;
use log::{error, info, trace, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tokio::task;
use tokio::time::timeout;
use up_rust::{
    ComparableListener, UAttributesValidators, UCode, UListener, UMessage, UStatus, UUri,
};

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

pub(crate) struct UPTransportVsomeipInnerHandle {
    storage: Arc<dyn UPTransportVsomeipStorage>,
    engine: UPTransportVsomeipInnerEngine,
    point_to_point_listener: RwLock<Option<Arc<dyn UListener>>>,
    config_path: Option<PathBuf>,
}

impl UPTransportVsomeipInnerHandle {
    pub fn new(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
    ) -> Result<Self, UStatus> {
        trace!("Starting UPTransportVsomeipInnerHandle, new, ue_id: {ue_id}");

        let storage = Arc::new(UPTransportVsomeipInnerHandleStorage::new(
            local_authority_name.clone(),
            remote_authority_name.clone(),
            ue_id,
        ));

        let engine = UPTransportVsomeipInnerEngine::new(ue_id, None);
        let point_to_point_listener = RwLock::new(None);
        let config_path = None;

        Ok(Self {
            engine,
            storage,
            point_to_point_listener,
            config_path,
        })
    }

    pub fn new_with_config(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: &Path,
    ) -> Result<Self, UStatus> {
        trace!("Starting UPTransportVsomeipInnerHandle, new_with_config, ue_id: {ue_id}");

        let storage = Arc::new(UPTransportVsomeipInnerHandleStorage::new(
            local_authority_name.clone(),
            remote_authority_name.clone(),
            ue_id,
        ));

        let engine = UPTransportVsomeipInnerEngine::new(ue_id, Some(config_path));
        let point_to_point_listener = RwLock::new(None);
        let config_path = Some(config_path.to_path_buf());

        Ok(Self {
            engine,
            storage,
            point_to_point_listener,
            config_path,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_supply_storage(
        storage: Arc<dyn UPTransportVsomeipStorage>,
    ) -> Result<Self, UStatus> {
        let engine = UPTransportVsomeipInnerEngine::new(storage.get_ue_id(), None);
        let point_to_point_listener = RwLock::new(None);
        let config_path = None;

        Ok(Self {
            engine,
            storage,
            point_to_point_listener,
            config_path,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_with_config_supply_storage(
        config_path: &Path,
        storage: Arc<dyn UPTransportVsomeipStorage>,
    ) -> Result<Self, UStatus> {
        let engine = UPTransportVsomeipInnerEngine::new(storage.get_ue_id(), Some(config_path));
        let point_to_point_listener = RwLock::new(None);
        let config_path = Some(config_path.to_path_buf());

        Ok(Self {
            engine,
            storage,
            point_to_point_listener,
            config_path,
        })
    }

    async fn await_engine(
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

    async fn send_to_engine_with_status(
        tx: &Sender<TransportCommand>,
        transport_command: TransportCommand,
    ) -> Result<(), UStatus> {
        tx.send(transport_command).await.map_err(|e| {
            UStatus::fail_with_code(
                UCode::INTERNAL,
                format!(
                    "Unable to transmit request to internal vsomeip application handler, err: {:?}",
                    e
                ),
            )
        })
    }

    fn get_storage(&self) -> Arc<dyn UPTransportVsomeipStorage> {
        self.storage.clone()
    }

    pub(crate) async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let registration_type = determine_registration_type(
            source_filter,
            &sink_filter.cloned(),
            self.get_storage().get_ue_id(),
        )?;

        trace!("registration_type: {registration_type:?}");

        if registration_type == RegistrationType::AllPointToPoint(0xFFFF) {
            return self.register_point_to_point_listener(&listener).await;
        }

        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_application_registry()
                .get_app_name_for_client_id(registration_type.client_id())
            {
                Ok(app_name)
            } else {
                let app_name = format!("{}", registration_type.client_id());
                let client_id = registration_type.client_id();
                self.initialize_vsomeip_app(client_id, app_name).await
            }
        };

        let Ok(app_name) = app_name_res else {
            return Err(app_name_res.err().unwrap());
        };

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);
        let Ok(msg_handler) = self
            .get_storage()
            .get_message_handler_registry()
            .get_message_handler(
                registration_type.client_id(),
                self.get_storage(),
                listener_config,
            )
        else {
            return Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Unable to get message handler for register_listener",
            ));
        };

        let (tx, rx) = oneshot::channel();
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::RegisterListener(
                source_filter.clone(),
                sink_filter.cloned(),
                registration_type,
                msg_handler,
                app_name,
                self.get_storage(),
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        Self::await_engine("register", rx).await
    }

    pub(crate) fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let src = source_filter.clone();
        let sink = sink_filter.cloned();

        let registration_type_res = determine_registration_type(
            source_filter,
            &sink_filter.cloned(),
            self.get_storage().get_ue_id(),
        );
        let Ok(registration_type) = registration_type_res else {
            return Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "Invalid source and sink filters for registerable types: Publish, Request, Response, AllPointToPoint"));
        };

        if registration_type == RegistrationType::AllPointToPoint(0xFFFF) {
            return self.unregister_point_to_point_listener();
        }

        let app_name_res = self
            .get_storage()
            .get_application_registry()
            .get_app_name_for_client_id(registration_type.client_id());

        let Some(app_name) = app_name_res else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "No application found for client_id: {}",
                    registration_type.client_id()
                ),
            ));
        };

        let (tx, rx) = oneshot::channel();
        trace!("attempting to block_on");

        // Using block_in_place to perform async operation in sync context
        let send_to_engine_res = task::block_in_place(|| {
            CB_RUNTIME.block_on(Self::send_to_engine_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::UnregisterListener(
                    src,
                    sink,
                    registration_type.clone(),
                    app_name,
                    tx,
                ),
            ))
        });

        trace!("after attempting to block_on");
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        let await_engine_res =
            task::block_in_place(|| CB_RUNTIME.block_on(Self::await_engine("unregister", rx)));
        if let Err(warn) = await_engine_res {
            warn!("{warn}");
        }

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);
        let client_usage_res = self
            .get_storage()
            .get_message_handler_registry()
            .release_message_handler(listener_config);

        // TODO: We should probably also remove entries from:
        //  * rpc_correlation -> send an error
        //  * vsomep_offered_requested -> unoffer / unrequest

        let Ok(client_usage) = client_usage_res else {
            warn!("{}", client_usage_res.err().unwrap());
            return Ok(());
        };

        match client_usage {
            ClientUsage::ClientIdInUse => {}
            ClientUsage::ClientIdNotInUse(client_id) => {
                task::block_in_place(|| CB_RUNTIME.block_on(self.shutdown_vsomeip_app(client_id)))?;
            }
        }

        Ok(())
    }

    pub(crate) async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        let attributes = message.attributes.as_ref().ok_or(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Missing uAttributes",
        ))?;

        // Validate UAttributes before conversion.
        UAttributesValidators::get_validator_for_attributes(attributes)
            .validate(attributes)
            .map_err(|e| {
                UStatus::fail_with_code(UCode::INTERNAL, format!("Invalid uAttributes, err: {e:?}"))
            })?;

        trace!("Sending message with attributes: {:?}", attributes);

        let Some(source_filter) = message.attributes.source.as_ref() else {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "UMessage provided with no source",
            ));
        };

        let sink_filter = message.attributes.sink.as_ref();
        let message_type = determine_send_type(source_filter, &sink_filter.cloned())?;
        trace!("inside send(), message_type: {message_type:?}");
        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_application_registry()
                .get_app_name_for_client_id(message_type.client_id())
            {
                Ok(app_name)
            } else {
                let client_id = message_type.client_id();
                let app_name = format!("{}", message_type.client_id());
                self.initialize_vsomeip_app(client_id, app_name).await
            }
        };

        let Ok(app_name) = app_name_res else {
            return Err(app_name_res.err().unwrap());
        };

        self.register_for_returning_response_if_point_to_point_listener_and_sending_request(
            source_filter,
            sink_filter,
            message_type.clone(),
        )
        .await?;

        let (tx, rx) = oneshot::channel();
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::Send(message, message_type, app_name, self.get_storage(), tx),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, rx).await
    }

    async fn register_for_returning_response_if_point_to_point_listener_and_sending_request(
        &self,
        msg_src: &UUri,
        msg_sink: Option<&UUri>,
        message_type: RegistrationType,
    ) -> Result<bool, UStatus> {
        let maybe_point_to_point_listener = {
            let point_to_point_listener = self.point_to_point_listener.read().unwrap();
            (*point_to_point_listener).as_ref().cloned()
        };

        let Some(ref point_to_point_listener) = maybe_point_to_point_listener else {
            return Ok(false);
        };

        if message_type != RegistrationType::Request(message_type.client_id()) {
            trace!("Sending non-Request when we have a point-to-point listener established");
            return Ok(true);
        }
        trace!("Sending a Request and we have a point-to-point listener");

        let Some(msg_sink) = msg_sink else {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "Missing sink for message",
            ));
        };

        // swap source and sink here since this is nominally representing a message and not a source
        // and sink filter
        let source_filter = msg_sink.clone();
        let sink_filter = Some(msg_src.clone());

        let listener = point_to_point_listener.clone();
        let comp_listener = ComparableListener::new(Arc::clone(&listener));
        let listener_config = (source_filter.clone(), sink_filter.clone(), comp_listener);
        let message_type = RegistrationType::Response(message_type.client_id());
        let msg_handler_res = self
            .get_storage()
            .get_message_handler_registry()
            .get_message_handler(
                message_type.client_id(),
                self.get_storage(),
                listener_config,
            );

        let msg_handler = {
            match msg_handler_res {
                Ok(msg_handler) => msg_handler,
                Err(e) => match e {
                    GetMessageHandlerError::ListenerConfigAlreadyExists(_msg_handler) => {
                        return Ok(true);
                    }
                    GetMessageHandlerError::ListenerIdAlreadyExists(listener_id) => {
                        return Err(UStatus::fail_with_code(
                            UCode::INTERNAL,
                            format!("listener_id already exists: {listener_id}"),
                        ));
                    }
                    GetMessageHandlerError::OtherError(s) => {
                        return Err(UStatus::fail_with_code(UCode::INTERNAL, s));
                    }
                },
            }
        };

        let Some(app_name) = self
            .get_storage()
            .get_application_registry()
            .get_app_name_for_client_id(message_type.client_id())
        else {
            panic!("vsomeip app for point_to_point_listener vsomeip app should already have been started under client_id: {}", message_type.client_id());
        };

        let (tx, rx) = oneshot::channel();
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::RegisterListener(
                source_filter.clone(),
                sink_filter.clone(),
                message_type,
                msg_handler,
                app_name,
                self.get_storage(),
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! err: {err}");
        }
        let await_res = Self::await_engine("register", rx).await;
        if let Err(err) = await_res {
            panic!("Unable to register: {err:?}");
        }

        trace!("Registered returning response listener for source_filter: {source_filter:?} sink_filter: {sink_filter:?}");

        Ok(true)
    }

    async fn register_point_to_point_listener(
        &self,
        listener: &Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let Some(config_path) = &self.config_path else {
            let err_msg = "No path to a vsomeip config file was provided";
            error!("{err_msg}");
            return Err(UStatus::fail_with_code(UCode::NOT_FOUND, err_msg));
        };

        let application_configs = extract_applications(config_path)?;
        trace!("Got vsomeip application_configs: {application_configs:?}");

        {
            let mut point_to_point_listener = self.point_to_point_listener.write().unwrap();
            if point_to_point_listener.is_some() {
                return Err(UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS,
                    "We already have a point-to-point UListener registered",
                ));
            }
            *point_to_point_listener = Some(listener.clone());
            trace!("We found a point-to-point listener and set it");
        }

        for app_config in application_configs {
            let registration_type = RegistrationType::Request(app_config.id);
            let app_config = Arc::new(app_config);

            let app_init_res = self
                .initialize_vsomeip_app(app_config.id, app_config.name.clone())
                .await;
            if let Err(err) = app_init_res {
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }

            let comp_listener = ComparableListener::new(listener.clone());
            let source_filter = any_uuri();
            let sink_filter = any_uuri_fixed_authority_id(
                &self.get_storage().get_local_authority(),
                app_config.id,
            );
            let listener_config = (
                source_filter.clone(),
                Some(sink_filter.clone()),
                comp_listener,
            );
            let Ok(msg_handler) = self
                .get_storage()
                .get_message_handler_registry()
                .get_message_handler(
                    registration_type.client_id(),
                    self.get_storage(),
                    listener_config,
                )
            else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to get message handler for register_point_to_point_listener",
                ));
            };

            let (tx, rx) = oneshot::channel();
            let send_to_engine_res = Self::send_to_engine_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::RegisterListener(
                    source_filter.clone(),
                    Some(sink_filter.clone()),
                    registration_type.clone(),
                    msg_handler,
                    app_config.name.clone(),
                    self.get_storage(),
                    tx,
                ),
            )
            .await;

            if let Err(err) = send_to_engine_res {
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            let internal_res =
                Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL, rx).await;
            if let Err(err) = internal_res {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    // TODO: Once we update to up-rust on crates.io Debug will be impl'ed on ComparableListener
                    // format!("Unable to register point to point listener: {listener_config:?} err: {err:?}"),
                    format!("Unable to register point to point listener: err: {err:?}"),
                ));
            }
        }

        Ok(())
    }

    fn unregister_point_to_point_listener(&self) -> Result<(), UStatus> {
        let ptp_comp_listener = {
            let point_to_point_listener = self.point_to_point_listener.read().unwrap();
            let Some(ref point_to_point_listener) = *point_to_point_listener else {
                return Err(UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS,
                    "No point-to-point listener found, we can't unregister it",
                ));
            };
            ComparableListener::new(point_to_point_listener.clone())
        };

        let Some(config_path) = &self.config_path else {
            let err_msg = "No path to a vsomeip config file was provided";
            error!("{err_msg}");
            return Err(UStatus::fail_with_code(UCode::NOT_FOUND, err_msg));
        };

        let application_configs = extract_applications(config_path)?;
        trace!("Got vsomeip application_configs: {application_configs:?}");

        for app_config in &application_configs {
            let source_filter = any_uuri();
            let sink_filter = any_uuri_fixed_authority_id(
                &self.get_storage().get_local_authority(),
                app_config.id,
            );

            let registration_type = {
                let reg_type_res = determine_registration_type(
                    &source_filter.clone(),
                    &Some(sink_filter.clone()),
                    self.get_storage().get_ue_id(),
                );
                match reg_type_res {
                    Ok(registration_type) => registration_type,
                    Err(warn) => {
                        // we must still attempt to unregister the rest
                        warn!("{warn}");
                        continue;
                    }
                }
            };

            let app_name = {
                match self
                    .get_storage()
                    .get_application_registry()
                    .get_app_name_for_client_id(registration_type.client_id())
                {
                    None => {
                        info!("vsomeip app for point_to_point_listener vsomeip app should already have been started under client_id: {}", registration_type.client_id());
                        return Ok(());
                    }
                    Some(app_name) => app_name,
                }
            };

            let (tx, rx) = oneshot::channel();

            let send_to_engine_res = task::block_in_place(|| {
                CB_RUNTIME.block_on(Self::send_to_engine_with_status(
                    &self.engine.transport_command_sender,
                    TransportCommand::UnregisterListener(
                        source_filter.clone(),
                        Some(sink_filter.clone()),
                        registration_type,
                        app_name,
                        tx,
                    ),
                ))
            });
            if let Err(err) = send_to_engine_res {
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            let await_engine_res =
                task::block_in_place(|| CB_RUNTIME.block_on(Self::await_engine("unregister", rx)));
            if let Err(warn) = await_engine_res {
                warn!("{warn}");
                continue;
            }

            // TODO: We should probably also remove entries from:
            //  * rpc_correlation -> send an error
            //  * vsomep_offered_requested -> unoffer / unrequest

            let listener_config = (source_filter, Some(sink_filter), ptp_comp_listener.clone());
            let client_usage_res = self
                .get_storage()
                .get_message_handler_registry()
                .release_message_handler(listener_config);

            // TODO: We should probably also remove entries from:
            //  * rpc_correlation -> send an error
            //  * vsomep_offered_requested -> unoffer / unrequest

            let Ok(client_usage) = client_usage_res else {
                warn!("{}", client_usage_res.err().unwrap());
                continue;
            };
            match client_usage {
                ClientUsage::ClientIdInUse => {}
                ClientUsage::ClientIdNotInUse(client_id) => {
                    let shutdown_res = task::block_in_place(|| {
                        CB_RUNTIME.block_on(self.shutdown_vsomeip_app(client_id))
                    });
                    if let Err(warn) = shutdown_res {
                        warn!("{warn}");
                        continue;
                    }
                }
            }
        }

        Ok(())
    }

    async fn initialize_vsomeip_app(
        &self,
        client_id: ClientId,
        app_name: ApplicationName,
    ) -> Result<ApplicationName, UStatus> {
        let (tx, rx) = oneshot::channel();
        trace!(
            "{}:{} - Sending TransportCommand for InitializeNewApp",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
        );
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::StartVsomeipApp(client_id, app_name.clone(), self.get_storage(), tx),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        let internal_res =
            Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL, rx).await;
        if let Err(err) = internal_res {
            Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Unable to start app for app_name: {app_name}, err: {err:?}"),
            ))
        } else {
            self.get_storage()
                .get_application_registry()
                .insert_client_and_app_name(client_id, app_name.clone())?;

            let check_app_res = self
                .get_storage()
                .get_application_registry()
                .get_app_name_for_client_id(client_id);
            match check_app_res {
                None => {
                    error!("Unable to find app_name for client_id: {client_id}");
                }
                Some(app_name) => {
                    trace!("Able to find app_name: {app_name} for client_id: {client_id}");
                }
            }

            Ok(app_name)
        }
    }

    async fn shutdown_vsomeip_app(&self, client_id: ClientId) -> Result<(), UStatus> {
        let Some(app_name) = self
            .get_storage()
            .get_application_registry()
            .remove_app_name_for_client_id(client_id)
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Unable to find app_name for client_id: {client_id}"),
            ));
        };
        trace!("No more remaining listeners for client_id: {client_id} app_name: {app_name}");

        let (tx, rx) = oneshot::channel();
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::StopVsomeipApp(client_id, app_name, tx),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await
    }
}

impl Drop for UPTransportVsomeipInnerHandle {
    fn drop(&mut self) {
        let ue_id = self.get_storage().get_ue_id();
        trace!("Running Drop for UPTransportVsomeipInnerHandle, ue_id: {ue_id}");

        let storage = self.get_storage().clone();
        let all_listener_configs = storage
            .get_message_handler_registry()
            .get_all_listener_configs();
        for listener_config in all_listener_configs {
            let (src_filter, sink_filter, comp_listener) = listener_config;
            let listener = comp_listener.into_inner();
            trace!(
                "attempting to unregister: src_filter: {src_filter:?} sink_filter: {sink_filter:?}"
            );
            let unreg_res = self.unregister_listener(&src_filter, sink_filter.as_ref(), listener);
            if let Err(warn) = unreg_res {
                warn!("{warn}");
            }
        }

        trace!("Finished running Drop for UPTransportVsomeipInnerHandle, ue_id: {ue_id}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_new_supply_storage() {
        env_logger::init();

        let mockable_storage =
            UPTransportVsomeipInnerHandleStorage::new("".to_string(), "".to_string(), 10);

        let _ = UPTransportVsomeipInnerHandle::new_supply_storage(Arc::new(mockable_storage));
    }

    #[tokio::test]
    async fn test_new_with_config_supply_storage() {
        let mockable_storage =
            UPTransportVsomeipInnerHandleStorage::new("".to_string(), "".to_string(), 10);

        let _ = UPTransportVsomeipInnerHandle::new_with_config_supply_storage(
            Path::new("/does/not/exist"),
            Arc::new(mockable_storage),
        );
    }
}
