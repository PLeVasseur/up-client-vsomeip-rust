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
use crate::storage::{UPTransportVsomeipInnerHandleStorage, UPTransportVsomeipStorage};
use crate::transport_inner::transport_inner_engine::{
    TransportCommand, UPTransportVsomeipInnerEngine,
};
use crate::transport_inner::{
    INTERNAL_FUNCTION_TIMEOUT, UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP,
    UP_CLIENT_VSOMEIP_TAG,
};
use crate::utils::{any_uuri, any_uuri_fixed_authority_id, TimedStdRwLock};
use crate::vsomeip_config::extract_applications;
use crate::{ApplicationName, AuthorityName, UPTransportVsomeipInner, UeId};
use async_trait::async_trait;
use futures::executor;
use log::{error, info, trace, warn};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tokio::time::timeout;
use up_rust::{
    ComparableListener, UAttributesValidators, UCode, UListener, UMessage, UStatus, UUri,
};

pub(crate) struct UPTransportVsomeipInnerHandle {
    storage: Arc<dyn UPTransportVsomeipStorage>,
    engine: UPTransportVsomeipInnerEngine,
    point_to_point_listener: TimedStdRwLock<Option<Arc<dyn UListener>>>,
    config_path: Option<PathBuf>,
}

impl UPTransportVsomeipInnerHandle {
    pub fn new(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
    ) -> Result<Self, UStatus> {
        let storage = Arc::new(UPTransportVsomeipInnerHandleStorage::new(
            local_authority_name.clone(),
            remote_authority_name.clone(),
            ue_id,
        ));

        let engine = UPTransportVsomeipInnerEngine::new(None);
        let point_to_point_listener = TimedStdRwLock::new(None);
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
        let storage = Arc::new(UPTransportVsomeipInnerHandleStorage::new(
            local_authority_name.clone(),
            remote_authority_name.clone(),
            ue_id,
        ));

        let engine = UPTransportVsomeipInnerEngine::new(Some(config_path));
        let point_to_point_listener = TimedStdRwLock::new(None);
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
        let engine = UPTransportVsomeipInnerEngine::new(None);
        let point_to_point_listener = TimedStdRwLock::new(None);
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
        let engine = UPTransportVsomeipInnerEngine::new(Some(config_path));
        let point_to_point_listener = TimedStdRwLock::new(None);
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

    async fn register_for_returning_response_if_point_to_point_listener_and_sending_request(
        &self,
        msg_src: &UUri,
        msg_sink: Option<&UUri>,
        message_type: RegistrationType,
    ) -> Result<bool, UStatus> {
        let maybe_point_to_point_listener = {
            let point_to_point_listener = self.point_to_point_listener.read();
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

        let Ok(listener_id) = self
            .get_storage()
            .get_message_handler_registry()
            .find_available_listener_id()
        else {
            return Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "Exhausted all extern 'C' fns",
            ));
        };

        let insert_res = self
            .get_storage()
            .get_message_handler_registry()
            .insert_listener_id_transport(listener_id, self.get_storage());
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            info!("register_for_returning_response_if_point_to_point_listener_and_sending_request: {err}");
            return Ok(true);
        }

        let listener_config = (
            source_filter.clone(),
            sink_filter.clone(),
            comp_listener.clone(),
        );

        let insert_res = self
            .get_storage()
            .get_listener_registry()
            .insert_listener_id_and_listener_config(listener_id, listener_config);
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            info!("register_for_returning_response_if_point_to_point_listener_and_sending_request: {err}");

            return Ok(true);
        }

        let insert_res = self
            .get_storage()
            .get_listener_registry()
            .insert_listener_id_client_id(listener_id, message_type.client_id());
        if let Some(previous_entry) = insert_res {
            let listener_id = previous_entry.0;
            let client_id = previous_entry.1;

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_listener_registry()
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("We already had used that listener_id with a client_id. listener_id: {} client_id: {}", listener_id, client_id)));
        }

        if self
            .get_storage()
            .get_listener_registry()
            .get_app_name_for_client_id(message_type.client_id())
            .is_none()
        {
            panic!("vsomeip app for point_to_point_listener vsomeip app should already have been started under client_id: {}", message_type.client_id());
        }

        trace!(
            "listener_id mapped to client_id: listener_id: {listener_id} client_id: {}",
            message_type.client_id()
        );

        let (tx, rx) = oneshot::channel();
        let msg_handler = self
            .get_storage()
            .get_message_handler_registry()
            .get_message_handler(listener_id);
        let message_type = RegistrationType::Response(message_type.client_id());

        let Some(app_name) = self
            .get_storage()
            .get_listener_registry()
            .get_app_name_for_client_id(message_type.client_id())
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "No vsomeip app_name found for client_id: {}",
                    message_type.client_id()
                ),
            ));
        };

        // we set sink_filter's resource id to wildcard so that we have
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
            // TODO: Consider if we'd like to try to get back to a sane state or just panic
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        let await_res = Self::await_engine("register", rx).await;

        if let Err(err) = await_res {
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_listener_registry()
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if self
                .get_storage()
                .get_listener_registry()
                .remove_client_id_based_on_listener_id(listener_id)
                .is_none()
            {
                warn!("No client_id found to remove for listener_id: {listener_id}");
            }

            return Err(err);
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
            let mut point_to_point_listener = self.point_to_point_listener.write();
            if point_to_point_listener.is_some() {
                return Err(UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS,
                    "We already have a point-to-point UListener registered",
                ));
            }
            *point_to_point_listener = Some(listener.clone());
            trace!("We found a point-to-point listener and set it");
        }

        for app_config in &application_configs {
            let (tx, rx) = oneshot::channel();
            let send_to_engine_res = Self::send_to_engine_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::StartVsomeipApp(
                    app_config.id,
                    app_config.name.clone(),
                    self.get_storage(),
                    tx,
                ),
            )
            .await;
            if let Err(err) = send_to_engine_res {
                // TODO: Consider if we'd like to try to get back to a sane state or just panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }

            let await_res = Self::await_engine(
                &format!("Initializing point-to-point listener apps. ApplicationConfig: {:?} app_config.id: {} app_config.name: {}",
                         app_config, app_config.id, app_config.name),
                rx,
            )
                .await;

            if let Err(err) = await_res {
                panic!("Unable to start necessary vsomeip app in point-to-point mode. app_config.id: {} app_config.name: {} err: {}", app_config.id, app_config.name, err);
            }

            let registration_type = RegistrationType::Request(app_config.id);

            let insert_res = self
                .get_storage()
                .get_listener_registry()
                .insert_client_and_app_name(registration_type.client_id(), app_config.name.clone());

            if let Err(err) = insert_res {
                let Some(app_name) = self
                    .get_storage()
                    .get_listener_registry()
                    .remove_app_name_for_client_id(registration_type.client_id())
                else {
                    return Err(UStatus::fail_with_code(
                            UCode::NOT_FOUND,
                            format!("Unable to find app_name for point-to-point app: app.id: {} app.name: {}", app_config.id, app_config.name),
                        ));
                };

                let (tx, rx) = oneshot::channel();
                let send_to_engine_res = Self::send_to_engine_with_status(
                    &self.engine.transport_command_sender,
                    TransportCommand::StopVsomeipApp(registration_type.client_id(), app_name, tx),
                )
                .await;
                if let Err(err) = send_to_engine_res {
                    // TODO: Consider if we'd like to try to get back to a sane state or just panic
                    panic!("engine has stopped! unable to proceed! with err: {err:?}");
                }

                if let Err(warn) = Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await {
                    warn!("{warn}");
                }

                return Err(err);
            }

            let Ok(listener_id) = self
                .get_storage()
                .get_message_handler_registry()
                .find_available_listener_id()
            else {
                return Err(UStatus::fail_with_code(
                    UCode::RESOURCE_EXHAUSTED,
                    "Exhausted all extern 'C' fns",
                ));
            };

            let comp_listener = ComparableListener::new(listener.clone());

            let source_filter = any_uuri();
            // TODO: How to explicitly handle instance_id?
            //  I'm not sure it's possible to set within the vsomeip config file
            let sink_filter = any_uuri_fixed_authority_id(
                &self.get_storage().get_local_authority(),
                app_config.id,
            );

            let insert_res = self
                .get_storage()
                .get_message_handler_registry()
                .insert_listener_id_transport(listener_id, self.get_storage());
            if let Err(err) = insert_res {
                if let Err(warn) = self
                    .get_storage()
                    .get_message_handler_registry()
                    .free_listener_id(listener_id)
                {
                    warn!("{warn}");
                }

                return Err(err);
            }

            let insert_res = self
                .get_storage()
                .get_listener_registry()
                .insert_listener_id_client_id(listener_id, registration_type.client_id());
            if let Some(previous_entry) = insert_res {
                let listener_id = previous_entry.0;
                let client_id = previous_entry.1;

                if let Err(warn) = self
                    .get_storage()
                    .get_message_handler_registry()
                    .free_listener_id(listener_id)
                {
                    warn! {"{warn}"};
                }

                if let Err(warn) = self
                    .get_storage()
                    .get_message_handler_registry()
                    .remove_listener_id_transport(listener_id)
                {
                    warn!("{warn}");
                }

                return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("We already had used that listener_id with a client_id. listener_id: {} client_id: {}", listener_id, client_id)));
            }

            let listener_config = (
                source_filter.clone(),
                Some(sink_filter.clone()),
                comp_listener.clone(),
            );

            let insert_res = self
                .get_storage()
                .get_listener_registry()
                .insert_listener_id_and_listener_config(listener_id, listener_config);
            if let Err(err) = insert_res {
                if let Err(warn) = self
                    .get_storage()
                    .get_message_handler_registry()
                    .free_listener_id(listener_id)
                {
                    warn!("{warn}");
                }

                if let Err(warn) = self
                    .get_storage()
                    .get_message_handler_registry()
                    .remove_listener_id_transport(listener_id)
                {
                    warn!("{warn}");
                }

                if self
                    .get_storage()
                    .get_listener_registry()
                    .remove_client_id_based_on_listener_id(listener_id)
                    .is_none()
                {
                    warn!("No client_id found to remove for listener_id: {listener_id}");
                }

                return Err(err);
            }

            let (tx, rx) = oneshot::channel();
            let msg_handler = self
                .get_storage()
                .get_message_handler_registry()
                .get_message_handler(listener_id);

            let send_to_engine_res = Self::send_to_engine_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::RegisterListener(
                    source_filter.clone(),
                    Some(sink_filter.clone()),
                    registration_type,
                    msg_handler,
                    app_config.name.clone(),
                    self.get_storage(),
                    tx,
                ),
            )
            .await;
            if let Err(err) = send_to_engine_res {
                // TODO: Consider if we'd like to try to get back to a sane state or just panic
                panic!("engine has stopped! unable to proceed! err: {err}");
            }

            Self::await_engine("register", rx).await?;
        }
        Ok(())
    }

    async fn unregister_point_to_point_listener(
        &self,
        listener: &Arc<dyn UListener>,
        _registration_type: &RegistrationType,
    ) -> Result<(), UStatus> {
        // TODO: Unregister all listener instances we established for handling responses

        let Some(config_path) = &self.config_path else {
            let err_msg = "No path to a vsomeip config file was provided";
            error!("{err_msg}");
            return Err(UStatus::fail_with_code(UCode::NOT_FOUND, err_msg));
        };

        let application_configs = extract_applications(config_path)?;
        trace!("Got vsomeip application_configs: {application_configs:?}");

        let ptp_comp_listener = {
            let point_to_point_listener = self.point_to_point_listener.read();
            let Some(ref point_to_point_listener) = *point_to_point_listener else {
                return Err(UStatus::fail_with_code(
                    UCode::ALREADY_EXISTS,
                    "No point-to-point listener found, we can't unregister it",
                ));
            };
            ComparableListener::new(point_to_point_listener.clone())
        };
        let comp_listener = ComparableListener::new(listener.clone());
        if ptp_comp_listener != comp_listener {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "listener provided doesn't match registered point_to_point_listener",
            ));
        }

        for app_config in &application_configs {
            let source_filter = any_uuri();
            let sink_filter = any_uuri_fixed_authority_id(
                &self.get_storage().get_local_authority(),
                app_config.id,
            );

            trace!(
                "Searching for src: {source_filter:?} sink: {sink_filter:?} to find listener_id"
            );

            let app_name_res = self
                .get_storage()
                .get_listener_registry()
                .get_app_name_for_client_id(app_config.id);
            let Some(app_name) = app_name_res else {
                // We must attempt to let the others unregister
                let warn = UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("No application found for client_id: {}", app_config.id),
                );
                warn!("{warn}");
                continue;
            };

            let (tx, rx) = oneshot::channel();
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

            let send_to_engine_res = Self::send_to_engine_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::UnregisterListener(
                    source_filter.clone(),
                    Some(sink_filter.clone()),
                    registration_type,
                    app_name,
                    tx,
                ),
            )
            .await;
            if let Err(err) = send_to_engine_res {
                // TODO: Consider if we'd like to try to get back to a sane state or just panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            let await_engine_res = Self::await_engine("unregister", rx).await;
            if let Err(warn) = await_engine_res {
                warn!("{warn}");
                continue;
            }

            let comp_listener = ComparableListener::new(listener.clone());
            let listener_config = (
                source_filter.clone(),
                Some(sink_filter.clone()),
                comp_listener,
            );

            let Some(listener_id) = self
                .get_storage()
                .get_listener_registry()
                .get_listener_id_for_listener_config(listener_config)
            else {
                // We must attempt to let the others unregister
                let warn = UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    "Unable to find listener_id for listener_config",
                );
                warn!("{warn}");
                continue;
            };

            let Some(client_id) = self
                .get_storage()
                .get_listener_registry()
                .remove_client_id_based_on_listener_id(listener_id)
            else {
                // We must attempt to let the others unregister
                let warn = UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Unable to find client_id for listener_id: {listener_id}"),
                );
                warn!("{warn}");
                continue;
            };

            if let Err(err) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("Unable to free listener_id: {listener_id} with err: {err:?}");
            }

            if let Err(err) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("Unable to remove storage for listener_id: {listener_id} with err: {err:?}");
            }

            if self
                .get_storage()
                .get_listener_registry()
                .listener_count_for_client_id(client_id)
                == 0
            {
                let Some(app_name) = self
                    .get_storage()
                    .get_listener_registry()
                    .remove_app_name_for_client_id(client_id)
                else {
                    // We must attempt to let the others unregister
                    let warn = UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!("Unable to find client_id for listener_id: {listener_id}"),
                    );
                    warn!("{warn}");
                    continue;
                };
                trace!(
                    "No more remaining listeners for client_id: {client_id} app_name: {app_name}"
                );

                let (tx, rx) = oneshot::channel();
                let send_to_engine_res = Self::send_to_engine_with_status(
                    &self.engine.transport_command_sender,
                    TransportCommand::StopVsomeipApp(client_id, app_name, tx),
                )
                .await;
                if let Err(err) = send_to_engine_res {
                    // TODO: Consider if we'd like to try to get back to a sane state or just panic
                    panic!("engine has stopped! unable to proceed! with err: {err:?}");
                }
                Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await?;
            }
        }
        Ok(())
    }

    async fn initialize_vsomeip_app(
        &self,
        registration_type: &RegistrationType,
    ) -> Result<ApplicationName, UStatus> {
        let app_name = format!("{}", registration_type.client_id());

        let (tx, rx) = oneshot::channel();
        trace!(
            "{}:{} - Sending TransportCommand for InitializeNewApp",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
        );
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::StartVsomeipApp(
                registration_type.client_id(),
                app_name.clone(),
                self.get_storage(),
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            // TODO: Consider if we'd like to try to get back to a sane state or just panic
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
                .get_listener_registry()
                .insert_client_and_app_name(registration_type.client_id(), app_name.clone())?;

            Ok(app_name)
        }
    }
}

impl Drop for UPTransportVsomeipInnerHandle {
    fn drop(&mut self) {
        trace!("Running Drop for UPTransportVsomeipInnerHandle");

        // Create a oneshot channel to wait for task completion
        let (tx, rx) = oneshot::channel();

        // Get the handle of the current runtime
        let handle = Handle::current();

        let storage = self.get_storage().clone();
        let transport_command_sender = self.engine.transport_command_sender.clone();

        thread::spawn(move || {
            handle.block_on(async move {
                let listener_ids = storage.get_listener_registry().get_listener_ids();

                let mut stopped_app_client_ids = HashSet::new();

                for listener_id in listener_ids {
                    trace!(
                        "Removing entries from message_handler_registry(): listener_id: {listener_id}"
                    );

                    let Some(client_id) = storage
                        .get_listener_registry()
                        .remove_client_id_based_on_listener_id(listener_id)
                        else {
                            warn!("Unable to find client_id for listener_id: {listener_id}");
                            continue;
                        };

                    let Some(app_name) = storage
                        .get_listener_registry()
                        .remove_app_name_for_client_id(client_id)
                        else {
                            let base_msg = format!("No app_name found for client_id: {client_id}");

                            if stopped_app_client_ids.contains(&client_id) {
                                info!("Trying to stop already stopped app. {base_msg}");
                            } else {
                                warn!("Unable to stop app. {base_msg}");
                            }

                            continue;
                        };

                    stopped_app_client_ids.insert(client_id);

                    // TODO: Should we iterate through all offered / requested services / events and
                    //  unoffer / unrequest them?

                    let (tx, rx) = oneshot::channel();
                    let send_to_engine_res = Self::send_to_engine_with_status(
                        &transport_command_sender,
                        TransportCommand::StopVsomeipApp(client_id, app_name, tx),
                    )
                        .await;
                    if let Err(err) = send_to_engine_res {
                        // TODO: Consider if we'd like to try to get back to a sane state or just panic
                        panic!("engine has stopped! unable to proceed! with err: {err:?}");
                    }
                    let stop_res =
                        Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await;
                    if let Err(warn) = stop_res {
                        warn!("{warn}");
                    }

                    if let Err(warn) = storage
                        .get_message_handler_registry()
                        .remove_listener_id_transport(listener_id)
                    {
                        warn!("{warn}");
                    }

                    if let Err(warn) = storage
                        .get_message_handler_registry()
                        .free_listener_id(listener_id)
                    {
                        warn!("{warn}");
                    }
                }

                // Notify that the task is complete
                trace!("Completed removal of all listener_ids");
                let _ = tx.send(());
            });
        });

        trace!("Waiting till removal of listener_ids");
        let _ = executor::block_on(rx);
        trace!("Finished waiting on removal of listener_ids");
    }
}

#[async_trait]
impl UPTransportVsomeipInner for UPTransportVsomeipInnerHandle {
    fn get_storage(&self) -> Arc<dyn UPTransportVsomeipStorage> {
        self.storage.clone()
    }

    async fn register_listener(
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

        let Ok(listener_id) = self
            .get_storage()
            .get_message_handler_registry()
            .find_available_listener_id()
        else {
            return Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "No more available extern fns",
            ));
        };

        let insert_res = self
            .get_storage()
            .get_message_handler_registry()
            .insert_listener_id_transport(listener_id, self.get_storage());
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            return Err(err);
        }

        let insert_res = self
            .get_storage()
            .get_listener_registry()
            .insert_listener_id_client_id(listener_id, registration_type.client_id());
        if let Some(previous_entry) = insert_res {
            let listener_id = previous_entry.0;
            let client_id = previous_entry.1;

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("We already had used that listener_id with a client_id. listener_id: {} client_id: {}", listener_id, client_id)));
        }

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (
            source_filter.clone(),
            sink_filter.cloned(),
            comp_listener.clone(),
        );

        let insert_res = self
            .get_storage()
            .get_listener_registry()
            .insert_listener_id_and_listener_config(listener_id, listener_config);
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            if self
                .get_storage()
                .get_listener_registry()
                .remove_client_id_based_on_listener_id(listener_id)
                .is_none()
            {
                warn!("No client_id found to remove for listener_id: {listener_id}");
            }

            return Err(err);
        }

        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_listener_registry()
                .get_app_name_for_client_id(registration_type.client_id())
            {
                Ok(app_name)
            } else {
                self.initialize_vsomeip_app(&registration_type).await
            }
        };

        let Ok(app_name) = app_name_res else {
            // we failed to start the vsomeip application
            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .free_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_message_handler_registry()
                .remove_listener_id_transport(listener_id)
            {
                warn!("{warn}");
            }

            if self
                .get_storage()
                .get_listener_registry()
                .remove_client_id_based_on_listener_id(listener_id)
                .is_none()
            {
                warn!("No client_id found to remove for listener_id: {listener_id}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_listener_registry()
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id)
            {
                warn!("{warn}");
            }

            return Err(app_name_res.err().unwrap());
        };

        let (tx, rx) = oneshot::channel();
        let msg_handler = self
            .get_storage()
            .get_message_handler_registry()
            .get_message_handler(listener_id);

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
            // TODO: Consider if we'd like to try to get back to a sane state or just panic
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        Self::await_engine("register", rx).await
    }

    async fn unregister_listener(
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
            return self
                .unregister_point_to_point_listener(&listener, &registration_type)
                .await;
        }

        let app_name_res = self
            .get_storage()
            .get_listener_registry()
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
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::UnregisterListener(
                src,
                sink,
                registration_type.clone(),
                app_name,
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            // TODO: Consider if we'd like to try to get back to a sane state or just panic
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_engine("unregister", rx).await?;

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);

        let Some(listener_id) = self
            .get_storage()
            .get_listener_registry()
            .get_listener_id_for_listener_config(listener_config)
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                "Unable to find listener_id for listener_config",
            ));
        };

        let Some(client_id) = self
            .get_storage()
            .get_listener_registry()
            .remove_client_id_based_on_listener_id(listener_id)
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Unable to find client_id for listener_id: {listener_id}"),
            ));
        };

        if let Err(err) = self
            .get_storage()
            .get_message_handler_registry()
            .free_listener_id(listener_id)
        {
            warn!("Unable to free listener_id: {listener_id} with err: {err:?}");
        }

        if let Err(err) = self
            .get_storage()
            .get_message_handler_registry()
            .remove_listener_id_transport(listener_id)
        {
            warn!("Unable to remove storage for listener_id: {listener_id} with err: {err:?}");
        }

        if self
            .get_storage()
            .get_listener_registry()
            .listener_count_for_client_id(client_id)
            == 0
        {
            let Some(app_name) = self
                .get_storage()
                .get_listener_registry()
                .remove_app_name_for_client_id(client_id)
            else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Unable to find app_name for listener_id: {listener_id}"),
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
                // TODO: Consider if we'd like to try to get back to a sane state or just panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await?;
        }

        Ok(())
    }

    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
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
                .get_listener_registry()
                .get_app_name_for_client_id(message_type.client_id())
            {
                Ok(app_name)
            } else {
                self.initialize_vsomeip_app(&message_type).await
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
            // TODO: Consider if we'd like to try to get back to a sane state or just panic
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, rx).await
    }
    async fn print_rwlock_times(&self) {
        #[cfg(feature = "timing")]
        {
            println!("point_to_point_listener");
            println!("reads: {:?}", self.point_to_point_listener.read_durations());
            println!(
                "writes: {:?}",
                self.point_to_point_listener.write_durations()
            );
        }

        self.get_storage()
            .get_listener_registry()
            .print_rwlock_times()
            .await;
        self.get_storage()
            .get_rpc_correlation()
            .print_rwlock_times()
            .await;
        self.get_storage()
            .get_message_handler_registry()
            .print_rwlock_times()
            .await;
    }
}

// TODO: Add unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_new_supply_storage() {
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
