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
use crate::extern_fn_registry::{ExternFnRegistry, MockableExternFnRegistry};
use crate::listener_registry::ListenerRegistry;
use crate::message_conversions::convert_umsg_to_vsomeip_msg_and_send;
use crate::rpc_correlation::RpcCorrelation2;
use crate::vsomeip_config::extract_applications;
use crate::vsomeip_offered_requested::VsomeipOfferedRequested2;
use crate::TimedRwLock;
use crate::{
    any_uuri, any_uuri_fixed_authority_id, split_u32_to_u16, ApplicationName, AuthorityName,
    ClientId, MockableUPTransportVsomeipInner, UPTransportVsomeipStorage, UeId,
};
use async_trait::async_trait;
use cxx::{let_cxx_string, UniquePtr};
use log::{error, info, trace, warn};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Builder;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::time::timeout;
use up_rust::{
    ComparableListener, UAttributesValidators, UCode, UListener, UMessage, UMessageType, UStatus,
    UUri,
};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;
use vsomeip_sys::glue::{
    make_application_wrapper, make_runtime_wrapper, ApplicationWrapper, RuntimeWrapper,
};
use vsomeip_sys::safe_glue::{
    get_pinned_application, get_pinned_runtime, register_message_handler_fn_ptr_safe,
    request_single_event_safe,
};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::{ANY_MAJOR, ANY_MINOR};

pub const UP_CLIENT_VSOMEIP_TAG: &str = "UPClientVsomeipInner";
pub const UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP: &str = "app_event_loop";
pub const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL: &str = "register_listener_internal";
// TODO: Decide whether to keep
// pub const UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL: &str =
//     "unregister_listener_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL: &str = "send_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL: &str =
    "initialize_new_app_internal";
pub const UP_CLIENT_VSOMEIP_FN_TAG_START_APP: &str = "start_app";
pub const UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP: &str = "stop_app";

const INTERNAL_FUNCTION_TIMEOUT: u64 = 3;

pub(crate) struct UPTransportVsomeipInnerHandleStorage {
    ue_id: UeId,
    local_authority: AuthorityName,
    remote_authority: AuthorityName,
    extern_fn_registry: Arc<dyn MockableExternFnRegistry>,
    listener_registry: Arc<ListenerRegistry>,
    rpc_correlation: Arc<RpcCorrelation2>,
    vsomeip_offered_requested: Arc<VsomeipOfferedRequested2>,
}

impl UPTransportVsomeipInnerHandleStorage {
    pub fn new(
        local_authority: AuthorityName,
        remote_authority: AuthorityName,
        ue_id: UeId,
    ) -> Self {
        let extern_fn_registry = ExternFnRegistry::new_trait_obj();

        Self {
            ue_id,
            local_authority,
            remote_authority,
            extern_fn_registry,
            listener_registry: Arc::new(ListenerRegistry::new()),
            rpc_correlation: Arc::new(RpcCorrelation2::new()),
            vsomeip_offered_requested: Arc::new(VsomeipOfferedRequested2::new()),
        }
    }
}

pub(crate) struct UPTransportVsomeipInnerHandle {
    storage: Arc<dyn UPTransportVsomeipStorage>,
    engine: UPTransportVsomeipInnerEngine,
    point_to_point_listener: TimedRwLock<Option<Arc<dyn UListener>>>,
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
        let point_to_point_listener = TimedRwLock::new(None);
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
        let point_to_point_listener = TimedRwLock::new(None);
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
        let point_to_point_listener = TimedRwLock::new(None);
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
        let point_to_point_listener = TimedRwLock::new(None);
        let config_path = Some(config_path.to_path_buf());

        Ok(Self {
            engine,
            storage,
            point_to_point_listener,
            config_path,
        })
    }

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

    async fn send_to_inner_with_status(
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
            let point_to_point_listener = self.point_to_point_listener.read().await;
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
        let listener_config = (
            source_filter.clone(),
            sink_filter.clone(),
            comp_listener.clone(),
        );

        // TODO: We should check here first on whether this has already been registered
        if let Some(existing_listener_id) = self
            .get_storage()
            .get_registry()
            .await
            .get_listener_id_for_listener_config(listener_config)
            .await
        {
            info!("Already have registered this response handler with existing_listener_id: {existing_listener_id}");
            return Ok(true);
        }

        let Ok(listener_id) = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .find_available_listener_id()
            .await
        else {
            return Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "Exhausted all extern 'C' fns",
            ));
        };

        let insert_res = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .insert_listener_id_transport(listener_id, self.get_storage())
            .await;
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
            {
                warn!("{warn}");
            }

            return Err(err);
        }

        let listener_config = (
            source_filter.clone(),
            sink_filter.clone(),
            comp_listener.clone(),
        );

        let insert_res = self
            .get_storage()
            .get_registry()
            .await
            .insert_listener_id_and_listener_config(listener_id, listener_config)
            .await;
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            return Err(err);
        }

        let insert_res = self
            .get_storage()
            .get_registry()
            .await
            .insert_listener_id_client_id(listener_id, message_type.client_id())
            .await;
        if let Some(previous_entry) = insert_res {
            let listener_id = previous_entry.0;
            let client_id = previous_entry.1;

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
            {
                warn!("{warn}");
            }

            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("We already had used that listener_id with a client_id. listener_id: {} client_id: {}", listener_id, client_id)));
        }

        if self
            .get_storage()
            .get_registry()
            .await
            .get_app_name_for_client_id(message_type.client_id())
            .await
            .is_none()
        {
            panic!("vsomeip app for point_to_point_listener vsomeip app should already have been started under client_id: {}", message_type.client_id());
        }

        trace!(
            "listener_id mapped to client_id: listener_id: {listener_id} client_id: {}",
            message_type.client_id()
        );

        let (tx, rx) = oneshot::channel();
        let extern_fn = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .get_extern_fn(listener_id)
            .await;
        let msg_handler = MessageHandlerFnPtr(extern_fn);
        let message_type = RegistrationType::Response(message_type.client_id());

        let Some(app_name) = self
            .get_storage()
            .get_registry()
            .await
            .get_app_name_for_client_id(message_type.client_id())
            .await
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
        let send_to_inner_res = Self::send_to_inner_with_status(
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
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        let await_res = Self::await_internal_function("register", rx).await;

        if let Err(err) = await_res {
            // TODO: Roll-back all the book-keeping
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
            let mut point_to_point_listener = self.point_to_point_listener.write().await;
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
            // TODO: Reconsider if we leave this here or use the initialize_vsomeip_app function
            let send_to_inner_res = Self::send_to_inner_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::StartVsomeipApp2(app_config.id, app_config.name.clone(), tx),
            )
            .await;
            if let Err(err) = send_to_inner_res {
                // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }

            let await_res = Self::await_internal_function(
                &format!("Initializing point-to-point listener apps. ApplicationConfig: {:?} app_config.id: {} app_config.name: {}",
                         app_config, app_config.id, app_config.name),
                rx,
            )
            .await;

            if let Err(err) = await_res {
                // TODO: Roll-back the book-keeping done up till this point
                return Err(err);
            }

            let registration_type = RegistrationType::Request(app_config.id);

            let insert_res = self
                .get_storage()
                .get_registry()
                .await
                .insert_client_and_app_name(registration_type.client_id(), app_config.name.clone())
                .await;

            if let Err(err) = insert_res {
                let Some(app_name) = self
                    .get_storage()
                    .get_registry()
                    .await
                    .remove_app_name_for_client_id(registration_type.client_id())
                    .await
                else {
                    return Err(UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!("Unable to find app_name for point-to-point app: app.id: {} app.name: {}", app_config.id, app_config.name),
                    ));
                };

                let (tx, rx) = oneshot::channel();
                let send_to_inner_res = Self::send_to_inner_with_status(
                    &self.engine.transport_command_sender,
                    TransportCommand::StopVsomeipApp(registration_type.client_id(), app_name, tx),
                )
                .await;
                if let Err(err) = send_to_inner_res {
                    // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                    panic!("engine has stopped! unable to proceed! with err: {err:?}");
                }

                if let Err(warn) =
                    Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await
                {
                    warn!("{warn}");
                }

                return Err(err);
            }

            let Ok(listener_id) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .find_available_listener_id()
                .await
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
                .get_extern_fn_registry()
                .await
                .insert_listener_id_transport(listener_id, self.get_storage())
                .await;
            if let Err(err) = insert_res {
                if let Err(warn) = self
                    .get_storage()
                    .get_extern_fn_registry()
                    .await
                    .free_listener_id(listener_id)
                    .await
                {
                    warn!("{warn}");
                }

                return Err(err);
            }

            let insert_res = self
                .get_storage()
                .get_registry()
                .await
                .insert_listener_id_client_id(listener_id, registration_type.client_id())
                .await;
            if let Some(previous_entry) = insert_res {
                let listener_id = previous_entry.0;
                let client_id = previous_entry.1;

                if let Err(warn) = self
                    .get_storage()
                    .get_extern_fn_registry()
                    .await
                    .free_listener_id(listener_id)
                    .await
                {
                    warn! {"{warn}"};
                }

                if let Err(warn) = self
                    .get_storage()
                    .get_extern_fn_registry()
                    .await
                    .remove_listener_id_transport(listener_id)
                    .await
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
                .get_registry()
                .await
                .insert_listener_id_and_listener_config(listener_id, listener_config)
                .await;
            if let Err(err) = insert_res {
                if let Err(warn) = self
                    .get_storage()
                    .get_extern_fn_registry()
                    .await
                    .free_listener_id(listener_id)
                    .await
                {
                    warn!("{warn}");
                }

                if let Err(warn) = self
                    .get_storage()
                    .get_extern_fn_registry()
                    .await
                    .remove_listener_id_transport(listener_id)
                    .await
                {
                    warn!("{warn}");
                }

                if self
                    .get_storage()
                    .get_registry()
                    .await
                    .remove_client_id_based_on_listener_id(listener_id)
                    .await
                    .is_none()
                {
                    warn!("No client_id found to remove for listener_id: {listener_id}");
                }

                return Err(err);
            }

            let (tx, rx) = oneshot::channel();
            let extern_fn = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .get_extern_fn(listener_id)
                .await;
            let msg_handler = MessageHandlerFnPtr(extern_fn);

            let send_to_inner_res = Self::send_to_inner_with_status(
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
            if let Err(err) = send_to_inner_res {
                // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                panic!("engine has stopped! unable to proceed! err: {err}");
            }

            Self::await_internal_function("register", rx).await?;
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
            let point_to_point_listener = self.point_to_point_listener.read().await;
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
                .get_registry()
                .await
                .get_app_name_for_client_id(app_config.id)
                .await;
            let Some(app_name) = app_name_res else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("No application found for client_id: {}", app_config.id),
                ));
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
                    Err(err) => {
                        return Err(err);
                    }
                }
            };

            let send_to_inner_res = Self::send_to_inner_with_status(
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
            if let Err(err) = send_to_inner_res {
                // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            Self::await_internal_function("unregister", rx).await?;

            let comp_listener = ComparableListener::new(listener.clone());
            let listener_config = (
                source_filter.clone(),
                Some(sink_filter.clone()),
                comp_listener,
            );

            let Some(listener_id) = self
                .get_storage()
                .get_registry()
                .await
                .get_listener_id_for_listener_config(listener_config)
                .await
            else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    "Unable to find listener_id for listener_config",
                ));
            };

            let Some(client_id) = self
                .get_storage()
                .get_registry()
                .await
                .remove_client_id_based_on_listener_id(listener_id)
                .await
            else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Unable to find client_id for listener_id: {listener_id}"),
                ));
            };

            if let Err(err) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("Unable to free listener_id: {listener_id} with err: {err:?}");
            }

            if let Err(err) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
            {
                warn!("Unable to remove storage for listener_id: {listener_id} with err: {err:?}");
            }

            if self
                .get_storage()
                .get_registry()
                .await
                .listener_count_for_client_id(client_id)
                .await
                == 0
            {
                let Some(app_name) = self
                    .get_storage()
                    .get_registry()
                    .await
                    .remove_app_name_for_client_id(client_id)
                    .await
                else {
                    return Err(UStatus::fail_with_code(
                        UCode::NOT_FOUND,
                        format!("Unable to find app_name for listener_id: {listener_id}"),
                    ));
                };
                trace!(
                    "No more remaining listeners for client_id: {client_id} app_name: {app_name}"
                );

                let (tx, rx) = oneshot::channel();
                let send_to_inner_res = Self::send_to_inner_with_status(
                    &self.engine.transport_command_sender,
                    TransportCommand::StopVsomeipApp(client_id, app_name, tx),
                )
                .await;
                if let Err(err) = send_to_inner_res {
                    // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                    panic!("engine has stopped! unable to proceed! with err: {err:?}");
                }
                Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await?;
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
        let send_to_inner_res = Self::send_to_inner_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::StartVsomeipApp2(registration_type.client_id(), app_name.clone(), tx),
        )
        .await;
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        let internal_res =
            Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL, rx)
                .await;
        if let Err(err) = internal_res {
            Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                format!("Unable to start app for app_name: {app_name}, err: {err:?}"),
            ))
        } else {
            self.get_storage()
                .get_registry()
                .await
                .insert_client_and_app_name(registration_type.client_id(), app_name.clone())
                .await?;

            Ok(app_name)
        }
    }

    pub async fn print_rwlock_times(&self) {
        println!("point_to_point_listener");
        println!(
            "reads: {:?}",
            self.point_to_point_listener.read_durations().await
        );
        println!(
            "writes: {:?}",
            self.point_to_point_listener.write_durations().await
        );

        self.get_storage()
            .get_registry()
            .await
            .print_rwlock_times()
            .await;
        self.get_storage()
            .get_rpc_correlation()
            .await
            .print_rwlock_times()
            .await;
    }
}

#[async_trait]
impl UPTransportVsomeipStorage for UPTransportVsomeipInnerHandleStorage {
    fn get_local_authority(&self) -> AuthorityName {
        self.local_authority.clone()
    }

    fn get_remote_authority(&self) -> AuthorityName {
        self.remote_authority.clone()
    }

    fn get_ue_id(&self) -> UeId {
        self.ue_id
    }

    async fn get_registry(&self) -> Arc<ListenerRegistry> {
        self.listener_registry.clone()
    }

    async fn get_extern_fn_registry(&self) -> Arc<dyn MockableExternFnRegistry> {
        self.extern_fn_registry.clone()
    }

    async fn get_rpc_correlation(&self) -> Arc<RpcCorrelation2> {
        self.rpc_correlation.clone()
    }

    async fn get_vsomeip_offered_requested(&self) -> Arc<VsomeipOfferedRequested2> {
        self.vsomeip_offered_requested.clone()
    }
}

#[async_trait]
impl MockableUPTransportVsomeipInner for UPTransportVsomeipInnerHandle {
    fn get_storage(&self) -> Arc<dyn UPTransportVsomeipStorage> {
        self.storage.clone()
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let registration_type_res = determine_registration_type(
            source_filter,
            &sink_filter.cloned(),
            self.get_storage().get_ue_id(),
        );
        let Ok(registration_type) = registration_type_res else {
            return Err(UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "Invalid source and sink filters for registerable types: Publish, Request, Response, AllPointToPoint"));
        };

        trace!("registration_type: {registration_type:?}");

        if registration_type == RegistrationType::AllPointToPoint(0xFFFF) {
            return self.register_point_to_point_listener(&listener).await;
        }

        let Ok(listener_id) = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .find_available_listener_id()
            .await
        else {
            return Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "No more available extern fns",
            ));
        };

        let insert_res = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .insert_listener_id_transport(listener_id, self.get_storage())
            .await;
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            return Err(err);
        }

        let insert_res = self
            .get_storage()
            .get_registry()
            .await
            .insert_listener_id_client_id(listener_id, registration_type.client_id())
            .await;
        if let Some(previous_entry) = insert_res {
            let listener_id = previous_entry.0;
            let client_id = previous_entry.1;

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
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
            .get_registry()
            .await
            .insert_listener_id_and_listener_config(listener_id, listener_config)
            .await;
        if let Err(err) = insert_res {
            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if self
                .get_storage()
                .get_registry()
                .await
                .remove_client_id_based_on_listener_id(listener_id)
                .await
                .is_none()
            {
                warn!("No client_id found to remove for listener_id: {listener_id}");
            }

            return Err(err);
        }

        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_registry()
                .await
                .get_app_name_for_client_id(registration_type.client_id())
                .await
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
                .get_extern_fn_registry()
                .await
                .free_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_extern_fn_registry()
                .await
                .remove_listener_id_transport(listener_id)
                .await
            {
                warn!("{warn}");
            }

            if self
                .get_storage()
                .get_registry()
                .await
                .remove_client_id_based_on_listener_id(listener_id)
                .await
                .is_none()
            {
                warn!("No client_id found to remove for listener_id: {listener_id}");
            }

            if let Err(warn) = self
                .get_storage()
                .get_registry()
                .await
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id)
                .await
            {
                warn!("{warn}");
            }

            return Err(app_name_res.err().unwrap());
        };

        let (tx, rx) = oneshot::channel();
        let extern_fn = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .get_extern_fn(listener_id)
            .await;
        let msg_handler = MessageHandlerFnPtr(extern_fn);

        let send_to_inner_res = Self::send_to_inner_with_status(
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
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        Self::await_internal_function("register", rx).await
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
            .get_registry()
            .await
            .get_app_name_for_client_id(registration_type.client_id())
            .await;
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
        let send_to_inner_res = Self::send_to_inner_with_status(
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
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_internal_function("unregister", rx).await?;

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);

        let Some(listener_id) = self
            .get_storage()
            .get_registry()
            .await
            .get_listener_id_for_listener_config(listener_config)
            .await
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                "Unable to find listener_id for listener_config",
            ));
        };

        let Some(client_id) = self
            .get_storage()
            .get_registry()
            .await
            .remove_client_id_based_on_listener_id(listener_id)
            .await
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Unable to find client_id for listener_id: {listener_id}"),
            ));
        };

        if let Err(err) = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .free_listener_id(listener_id)
            .await
        {
            warn!("Unable to free listener_id: {listener_id} with err: {err:?}");
        }

        if let Err(err) = self
            .get_storage()
            .get_extern_fn_registry()
            .await
            .remove_listener_id_transport(listener_id)
            .await
        {
            warn!("Unable to remove storage for listener_id: {listener_id} with err: {err:?}");
        }

        if self
            .get_storage()
            .get_registry()
            .await
            .listener_count_for_client_id(client_id)
            .await
            == 0
        {
            let Some(app_name) = self
                .get_storage()
                .get_registry()
                .await
                .remove_app_name_for_client_id(client_id)
                .await
            else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Unable to find app_name for listener_id: {listener_id}"),
                ));
            };
            trace!("No more remaining listeners for client_id: {client_id} app_name: {app_name}");

            let (tx, rx) = oneshot::channel();
            let send_to_inner_res = Self::send_to_inner_with_status(
                &self.engine.transport_command_sender,
                TransportCommand::StopVsomeipApp(client_id, app_name, tx),
            )
            .await;
            if let Err(err) = send_to_inner_res {
                // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
                panic!("engine has stopped! unable to proceed! with err: {err:?}");
            }
            Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await?;
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

        trace!("Sending message: {:?}", message);

        let Some(source_filter) = message.attributes.source.as_ref() else {
            return Err(UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "UMessage provided with no source",
            ));
        };

        source_filter.verify_no_wildcards().map_err(|e| {
            UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("Invalid source: {e:?}"))
        })?;

        let sink_filter = message.attributes.sink.as_ref();

        if let Some(sink) = sink_filter {
            sink.verify_no_wildcards().map_err(|e| {
                UStatus::fail_with_code(UCode::INVALID_ARGUMENT, format!("Invalid sink: {e:?}"))
            })?;
        }

        let message_type = determine_send_type(source_filter, &sink_filter.cloned())?;
        trace!("inside send(), message_type: {message_type:?}");
        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_registry()
                .await
                .get_app_name_for_client_id(message_type.client_id())
                .await
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
        let send_to_inner_res = Self::send_to_inner_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::Send(message, message_type, app_name, self.get_storage(), tx),
        )
        .await;
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, rx).await
    }
}

pub enum TransportCommand {
    // Primary purpose of a UTransport
    RegisterListener(
        UUri,
        Option<UUri>,
        RegistrationType,
        MessageHandlerFnPtr,
        ApplicationName,
        Arc<dyn UPTransportVsomeipStorage>,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    UnregisterListener(
        UUri,
        Option<UUri>,
        RegistrationType,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    Send(
        UMessage,
        RegistrationType,
        ApplicationName,
        Arc<dyn UPTransportVsomeipStorage>,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    // Additional helpful commands
    StartVsomeipApp2(
        ClientId,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    StopVsomeipApp(
        ClientId,
        ApplicationName,
        oneshot::Sender<Result<(), UStatus>>,
    ),
}

pub(crate) struct UPTransportVsomeipInnerEngine {
    pub(crate) transport_command_sender: Sender<TransportCommand>,
}

impl UPTransportVsomeipInnerEngine {
    pub fn new(config_path: Option<&Path>) -> Self {
        let (tx, rx) = channel(10000);

        Self::start_event_loop(rx, config_path);

        Self {
            transport_command_sender: tx,
        }
    }

    fn start_event_loop(rx_to_event_loop: Receiver<TransportCommand>, config_path: Option<&Path>) {
        let config_path: Option<PathBuf> = config_path.map(|p| p.to_path_buf());

        thread::spawn(move || {
            trace!("On dedicated thread");

            // Create a new single-threaded runtime
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to create Tokio runtime");

            runtime.block_on(async move {
                trace!("Within blocked runtime");
                Self::event_loop(rx_to_event_loop, config_path).await;
                info!("Broke out of loop! You probably dropped the UPClientVsomeip");
            });
            trace!("Parking dedicated thread");
            thread::park();
            trace!("Made past dedicated thread park");
        });
        trace!("Past thread spawn");
    }

    fn create_app(app_name: &ApplicationName, config_path: Option<&Path>) -> Result<(), UStatus> {
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
            if application_wrapper.get_mut().is_null() {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to create vsomeip application",
                ));
            }

            get_pinned_application(&application_wrapper).init();
            let client_id = get_pinned_application(&application_wrapper).get_client();
            trace!("start_app: after starting app we see its client_id: {client_id}");
            // FYI: thread is blocked by vsomeip here
            get_pinned_application(&application_wrapper).start();

            Ok(())
        });

        trace!(
            "{}:{} - Made it past creating app and starting it on dedicated thread",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_START_APP
        );

        // TODO: Should be removed in favor of a signal-based strategy
        trace!("within create_app, slept for 50ms");
        thread::sleep(Duration::from_millis(50));

        Ok(())
    }

    async fn event_loop(
        mut rx_to_event_loop: Receiver<TransportCommand>,
        config_path: Option<PathBuf>,
    ) {
        let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());

        trace!("Entering command loop");
        while let Some(cmd) = rx_to_event_loop.recv().await {
            trace!("Received TransportCommand");

            match cmd {
                TransportCommand::RegisterListener(
                    src,
                    sink,
                    registration_type,
                    msg_handler,
                    app_name,
                    transport_storage,
                    return_channel,
                ) => {
                    trace!(
                        "{}:{} - Attempting to register listener: src: {src:?} sink: {sink:?}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                    );

                    trace!("registration_type: {registration_type:?}");

                    let_cxx_string!(app_name_cxx = app_name);

                    let mut application_wrapper = make_application_wrapper(
                        get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                    );

                    let res = Self::register_listener_internal(
                        src,
                        sink,
                        registration_type,
                        msg_handler,
                        transport_storage,
                        &mut application_wrapper,
                        &runtime_wrapper,
                    )
                    .await;
                    trace!("Sending back results of registration");
                    Self::return_oneshot_result(res, return_channel).await;
                    trace!("Sent back results of registration");
                }
                TransportCommand::UnregisterListener(
                    src,
                    sink,
                    registration_type,
                    app_name,
                    return_channel,
                ) => {
                    let_cxx_string!(app_name_cxx = app_name);

                    let application_wrapper = make_application_wrapper(
                        get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                    );

                    let res = Self::unregister_listener_internal(
                        src,
                        sink,
                        registration_type,
                        &application_wrapper,
                        &runtime_wrapper,
                    )
                    .await;
                    Self::return_oneshot_result(res, return_channel).await;
                }
                TransportCommand::Send(
                    umsg,
                    message_type,
                    app_name,
                    transport_storage,
                    return_channel,
                ) => {
                    trace!(
                        "{}:{} - Attempting to send UMessage: {:?}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                        umsg
                    );

                    trace!(
                        "inside TransportCommand::Send dispatch, message_type: {message_type:?}"
                    );

                    let_cxx_string!(app_name_cxx = app_name.clone());

                    let application =
                        get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx);
                    if application.is_null() {
                        let err = format!(
                            "No application exists for {app_name} under client_id: {}",
                            message_type.client_id()
                        );
                        Self::return_oneshot_result(
                            Err(UStatus::fail_with_code(UCode::INTERNAL, err)),
                            return_channel,
                        )
                        .await;
                        continue;
                    }
                    let mut application_wrapper = make_application_wrapper(application);
                    let app_client_id = get_pinned_application(&application_wrapper).get_client();
                    trace!("Application existed for {app_name}, listed under client_id: {}, with app_client_id: {app_client_id}", message_type.client_id());

                    let res = Self::send_internal(
                        umsg,
                        transport_storage,
                        &mut application_wrapper,
                        &runtime_wrapper,
                    )
                    .await;
                    Self::return_oneshot_result(res, return_channel).await;
                }
                TransportCommand::StartVsomeipApp2(client_id, app_name, return_channel) => {
                    trace!(
                        "{}:{} - Attempting to initialize new app for client_id: {} app_name: {}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                        client_id,
                        app_name
                    );
                    let new_app_res = Self::start_vsomeip_app_internal(
                        client_id,
                        app_name.clone(),
                        config_path.clone(),
                        &runtime_wrapper,
                    )
                    .await;
                    trace!(
                        "{}:{} - After attempt to create new app for client_id: {} app_name: {}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                        client_id,
                        app_name
                    );

                    if let Err(err) = new_app_res {
                        error!("Unable to create new app: {:?}", err);
                        Self::return_oneshot_result(Err(err), return_channel).await;
                        continue;
                    }

                    // TODO: Need to find a way to confirm that we brought up the vsomeip app here
                    //  and use that for the result.
                    //
                    //  For now we assume success
                    Self::return_oneshot_result(Ok(()), return_channel).await;
                }
                TransportCommand::StopVsomeipApp(_client_id, app_name, return_channel) => {
                    let stop_res =
                        Self::stop_vsomeip_app_internal(app_name, &runtime_wrapper).await;
                    Self::return_oneshot_result(stop_res, return_channel).await;
                }
            }
            trace!("Hit bottom of event loop");
        }
    }

    async fn register_listener_internal(
        source_filter: UUri,
        sink_filter: Option<UUri>,
        registration_type: RegistrationType,
        msg_handler: MessageHandlerFnPtr,
        transport_storage: Arc<dyn UPTransportVsomeipStorage>,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<(), UStatus> {
        trace!(
            "{}:{} - Attempting to register: source_filter: {:?} & sink_filter: {:?}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
            source_filter,
            sink_filter
        );

        match registration_type {
            RegistrationType::Publish(_) => {
                trace!(
                    "{}:{} - Registering for Publish style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let (_, service_id) = split_u32_to_u16(source_filter.ue_id);
                // let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let instance_id = 1;
                let (_, event_id) = split_u32_to_u16(source_filter.resource_id);

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    event_id
                );

                if !transport_storage
                    .get_vsomeip_offered_requested()
                    .await
                    .is_event_requested(service_id, instance_id, event_id)
                    .await
                {
                    get_pinned_application(application_wrapper).request_service(
                        service_id,
                        instance_id,
                        ANY_MAJOR,
                        vsomeip::ANY_MINOR,
                    );
                    request_single_event_safe(
                        application_wrapper,
                        service_id,
                        instance_id,
                        event_id,
                        event_id,
                    );
                    get_pinned_application(application_wrapper).subscribe(
                        service_id,
                        instance_id,
                        event_id,
                        ANY_MAJOR,
                        event_id,
                    );
                    transport_storage
                        .get_vsomeip_offered_requested()
                        .await
                        .insert_event_requested(service_id, instance_id, event_id)
                        .await;
                }

                register_message_handler_fn_ptr_safe(
                    application_wrapper,
                    service_id,
                    instance_id,
                    event_id,
                    msg_handler,
                );

                // Letting vsomeip settle
                trace!("within register_listener_internal, slept for 5ms");
                tokio::time::sleep(Duration::from_millis(5)).await;

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
                let Some(sink_filter) = sink_filter else {
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

                if !transport_storage
                    .get_vsomeip_offered_requested()
                    .await
                    .is_service_offered(service_id, instance_id, method_id)
                    .await
                {
                    get_pinned_application(application_wrapper).offer_service(
                        service_id,
                        instance_id,
                        ANY_MAJOR,
                        ANY_MINOR,
                    );
                    transport_storage
                        .get_vsomeip_offered_requested()
                        .await
                        .insert_service_offered(service_id, instance_id, method_id)
                        .await;
                }

                register_message_handler_fn_ptr_safe(
                    application_wrapper,
                    service_id,
                    vsomeip::ANY_INSTANCE,
                    method_id,
                    msg_handler,
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

                let (_, service_id) = split_u32_to_u16(source_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(source_filter.resource_id);

                if !transport_storage
                    .get_vsomeip_offered_requested()
                    .await
                    .is_service_requested(service_id, instance_id, method_id)
                    .await
                {
                    get_pinned_application(application_wrapper).request_service(
                        service_id,
                        instance_id,
                        ANY_MAJOR,
                        ANY_MINOR,
                    );
                    transport_storage
                        .get_vsomeip_offered_requested()
                        .await
                        .insert_service_requested(service_id, instance_id, method_id)
                        .await;
                }

                trace!(
                    "{}:{} - register_message_handler: service: {} instance: {} method: {}",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                    service_id,
                    instance_id,
                    method_id
                );

                register_message_handler_fn_ptr_safe(
                    application_wrapper,
                    service_id,
                    instance_id,
                    method_id,
                    msg_handler,
                );

                trace!(
                    "{}:{} - Registered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::AllPointToPoint(_) => Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Should be impossible to register point-to-point internally",
            )),
        }
    }

    async fn unregister_listener_internal(
        source_filter: UUri,
        sink_filter: Option<UUri>,
        registration_type: RegistrationType,
        application_wrapper: &UniquePtr<ApplicationWrapper>,
        _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<(), UStatus> {
        trace!(
            "{}:{} - Attempting to unregister: source_filter: {:?} & sink_filter: {:?}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
            source_filter,
            sink_filter
        );

        match registration_type {
            RegistrationType::Publish(_) => {
                trace!(
                    "{}:{} - Unregistering for Publish style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let (_, service_id) = split_u32_to_u16(source_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(source_filter.resource_id);

                get_pinned_application(application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                Ok(())
            }
            RegistrationType::Request(_) => {
                trace!(
                    "{}:{} - Unregistering for Request style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let Some(sink_filter) = sink_filter else {
                    return Err(UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        "Request doesn't contain sink",
                    ));
                };

                let (_, service_id) = split_u32_to_u16(sink_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(sink_filter.resource_id);

                get_pinned_application(application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::Response(_) => {
                trace!(
                    "{}:{} - Unregistering for Response style messages.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );
                let Some(sink_filter) = sink_filter else {
                    return Err(UStatus::fail_with_code(
                        UCode::INVALID_ARGUMENT,
                        "Request doesn't contain sink",
                    ));
                };

                let (_, service_id) = split_u32_to_u16(sink_filter.ue_id);
                let instance_id = vsomeip::ANY_INSTANCE; // TODO: Set this to 1? To ANY_INSTANCE?
                let (_, method_id) = split_u32_to_u16(sink_filter.resource_id);

                get_pinned_application(application_wrapper).unregister_message_handler(
                    service_id,
                    instance_id,
                    method_id,
                );

                trace!(
                    "{}:{} - Unregistered vsomeip message handler.",
                    UP_CLIENT_VSOMEIP_TAG,
                    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL,
                );

                Ok(())
            }
            RegistrationType::AllPointToPoint(_) => Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Should be impossible to unregister point-to-point internally",
            )),
        }
    }

    async fn send_internal(
        umsg: UMessage,
        transport_storage: Arc<dyn UPTransportVsomeipStorage>,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<(), UStatus> {
        match umsg
            .attributes
            .type_
            .enum_value_or(UMessageType::UMESSAGE_TYPE_UNSPECIFIED)
        {
            UMessageType::UMESSAGE_TYPE_UNSPECIFIED => {
                return Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Unspecified message type not supported",
                ));
            }
            UMessageType::UMESSAGE_TYPE_NOTIFICATION => {
                return Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Notification is not supported",
                ));
            }
            UMessageType::UMESSAGE_TYPE_PUBLISH => {
                let _vsomeip_msg_res = convert_umsg_to_vsomeip_msg_and_send(
                    &umsg,
                    transport_storage,
                    application_wrapper,
                    runtime_wrapper,
                )
                .await;
            }
            UMessageType::UMESSAGE_TYPE_REQUEST | UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let _vsomeip_msg_res = convert_umsg_to_vsomeip_msg_and_send(
                    &umsg,
                    transport_storage,
                    application_wrapper,
                    runtime_wrapper,
                )
                .await;
            }
        }
        Ok(())
    }

    async fn return_oneshot_result(
        result: Result<(), UStatus>,
        tx: oneshot::Sender<Result<(), UStatus>>,
    ) {
        if let Err(err) = tx.send(result) {
            error!("Unable to return oneshot result back to transport: {err:?}");
        }
    }

    async fn start_vsomeip_app_internal(
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

        Self::create_app(&app_name, config_path.as_deref())?;
        trace!(
            "{}:{} - After starting app for client_id: {} app_name: {}",
            UP_CLIENT_VSOMEIP_TAG,
            UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
            client_id,
            app_name
        );

        let_cxx_string!(app_name_cxx = app_name);

        let app_wrapper = make_application_wrapper(
            get_pinned_runtime(runtime_wrapper).get_application(&app_name_cxx),
        );
        Ok(app_wrapper)
    }

    async fn stop_vsomeip_app_internal(
        app_name: ApplicationName,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<(), UStatus> {
        trace!("Stopping vsomeip application for app_name: {app_name}");

        let_cxx_string!(app_name_cxx = app_name);

        let app_wrapper = make_application_wrapper(
            get_pinned_runtime(runtime_wrapper).get_application(&app_name_cxx),
        );

        get_pinned_application(&app_wrapper).stop();

        Ok(())
    }
}

// TODO: Add unit tests
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_new_supply_storage() {
        let mockable_storage =
            UPTransportVsomeipInnerHandleStorage::new("".to_string(), "".to_string(), 10);

        let _ = UPTransportVsomeipInnerHandle::new_supply_storage(Arc::new(mockable_storage));
    }

    #[test]
    fn test_new_with_config_supply_storage() {
        let mockable_storage =
            UPTransportVsomeipInnerHandleStorage::new("".to_string(), "".to_string(), 10);

        let _ = UPTransportVsomeipInnerHandle::new_with_config_supply_storage(
            Path::new("/does/not/exist"),
            Arc::new(mockable_storage),
        );
    }
}
