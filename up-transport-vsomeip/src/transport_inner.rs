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
use crate::extern_fn_registry::{ExternFnRegistry, MockableExternFnRegistry, Registry};
use crate::listener_registry::ListenerRegistry;
use crate::message_conversions::convert_umsg_to_vsomeip_msg_and_send;
use crate::rpc_correlation::RpcCorrelation2;
use crate::vsomeip_offered_requested::{VsomeipOfferedRequested, VsomeipOfferedRequested2};
use crate::{
    split_u32_to_u16, ApplicationName, AuthorityName, ClientId, MockableUPTransportVsomeipInner,
    UPTransportVsomeipStorage, UeId,
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
use tokio::sync::{oneshot, RwLock as TokioRwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::time::timeout;
use up_rust::{
    ComparableListener, UAttributesValidators, UCode, UListener, UMessage, UMessageType, UStatus,
    UUri,
};
use uuid::Uuid;
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
pub const UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL: &str =
    "unregister_listener_internal";
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
    extern_fn_registry: TokioRwLock<Arc<dyn MockableExternFnRegistry>>,
    listener_registry: TokioRwLock<ListenerRegistry>,
    rpc_correlation: TokioRwLock<RpcCorrelation2>,
    vsomeip_offered_requested: TokioRwLock<VsomeipOfferedRequested2>,
}

impl UPTransportVsomeipInnerHandleStorage {
    pub async fn new(
        local_authority: AuthorityName,
        remote_authority: AuthorityName,
        ue_id: UeId,
    ) -> Self {
        let extern_fn_registry: Arc<dyn MockableExternFnRegistry> = Arc::new(ExternFnRegistry);

        Self {
            ue_id,
            local_authority,
            remote_authority,
            extern_fn_registry: TokioRwLock::new(extern_fn_registry),
            listener_registry: TokioRwLock::new(ListenerRegistry::new()),
            rpc_correlation: TokioRwLock::new(RpcCorrelation2::new()),
            vsomeip_offered_requested: TokioRwLock::new(VsomeipOfferedRequested2::new()),
        }
    }
}

pub(crate) struct UPTransportVsomeipInnerHandle {
    storage: Arc<dyn UPTransportVsomeipStorage>,
    engine: UPTransportVsomeipInnerEngine,
}

impl UPTransportVsomeipInnerHandle {
    pub async fn new(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
    ) -> Result<Self, UStatus> {
        let storage = Arc::new(
            UPTransportVsomeipInnerHandleStorage::new(
                local_authority_name.clone(),
                remote_authority_name.clone(),
                ue_id,
            )
            .await,
        );

        let engine = UPTransportVsomeipInnerEngine::new(None);

        Ok(Self { engine, storage })
    }

    pub async fn new_with_config(
        local_authority_name: &AuthorityName,
        remote_authority_name: &AuthorityName,
        ue_id: UeId,
        config_path: &Path,
    ) -> Result<Self, UStatus> {
        let storage = Arc::new(
            UPTransportVsomeipInnerHandleStorage::new(
                local_authority_name.clone(),
                remote_authority_name.clone(),
                ue_id,
            )
            .await,
        );

        let engine = UPTransportVsomeipInnerEngine::new(Some(config_path));

        Ok(Self { engine, storage })
    }

    pub(crate) async fn new_supply_storage(
        storage: Arc<dyn UPTransportVsomeipStorage>,
    ) -> Result<Self, UStatus> {
        let engine = UPTransportVsomeipInnerEngine::new(None);

        Ok(Self { engine, storage })
    }

    pub(crate) async fn new_with_config_supply_storage(
        config_path: &Path,
        storage: Arc<dyn UPTransportVsomeipStorage>,
    ) -> Result<Self, UStatus> {
        let engine = UPTransportVsomeipInnerEngine::new(Some(config_path));

        Ok(Self { engine, storage })
    }

    fn start_vsomeip_app(
        &self,
        transport_instance_id: uuid::Uuid,
        client_id: ClientId,
        application_name: ApplicationName,
    ) -> Result<(), UStatus> {
        todo!()
    }

    fn stop_vsomeip_app(
        &self,
        client_id: ClientId,
        application_name: ApplicationName,
    ) -> Result<(), UStatus> {
        todo!()
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
        message: &UMessage,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        message_type: RegistrationType,
    ) -> Result<bool, UStatus> {
        todo!()
    }

    async fn register_point_to_point_listener(
        &self,
        listener: &Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        todo!()
    }

    async fn unregister_point_to_point_listener(
        &self,
        listener: &Arc<dyn UListener>,
        _registration_type: &RegistrationType,
    ) -> Result<(), UStatus> {
        todo!()
    }

    async fn initialize_vsomeip_app(
        &self,
        registration_type: &RegistrationType,
    ) -> Result<ApplicationName, UStatus> {
        todo!()
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
        todo!()
    }

    async fn get_registry_read(&self) -> RwLockReadGuard<'_, ListenerRegistry> {
        self.listener_registry.read().await
    }

    async fn get_registry_write(&self) -> RwLockWriteGuard<'_, ListenerRegistry> {
        self.listener_registry.write().await
    }

    async fn get_extern_fn_registry_read(
        &self,
    ) -> RwLockReadGuard<'_, Arc<dyn MockableExternFnRegistry>> {
        self.extern_fn_registry.read().await
    }

    async fn get_extern_fn_registry_write(
        &self,
    ) -> RwLockWriteGuard<'_, Arc<dyn MockableExternFnRegistry>> {
        self.extern_fn_registry.write().await
    }

    async fn get_rpc_correlation_read(&self) -> RwLockReadGuard<'_, RpcCorrelation2> {
        self.rpc_correlation.read().await
    }

    async fn get_rpc_correlation_write(&self) -> RwLockWriteGuard<'_, RpcCorrelation2> {
        self.rpc_correlation.write().await
    }

    async fn get_vsomeip_offered_requested_read(
        &self,
    ) -> RwLockReadGuard<'_, VsomeipOfferedRequested2> {
        self.vsomeip_offered_requested.read().await
    }

    async fn get_vsomeip_offered_requested_write(
        &self,
    ) -> RwLockWriteGuard<'_, VsomeipOfferedRequested2> {
        self.vsomeip_offered_requested.write().await
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
            .get_extern_fn_registry_write()
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
            .get_extern_fn_registry_write()
            .await
            .insert_listener_id_transport(listener_id, self.get_storage())
            .await;
        if let Err(err) = insert_res {
            let _ = self
                .get_storage()
                .get_extern_fn_registry_write()
                .await
                .free_listener_id(listener_id)
                .await;

            return Err(err);
        }

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (
            source_filter.clone(),
            sink_filter.cloned(),
            comp_listener.clone(),
        );

        let insert_res = self
            .get_storage()
            .get_registry_write()
            .await
            .set_listener_id_and_listener_config(listener_id, listener_config);
        if let Err(err) = insert_res {
            let _ = self
                .get_storage()
                .get_extern_fn_registry_write()
                .await
                .free_listener_id(listener_id)
                .await;

            return Err(err);
        }

        let app_name_res = {
            if let Some(app_name) = self
                .get_storage()
                .get_registry_read()
                .await
                .get_app_name_for_client_id(registration_type.client_id())
            {
                Ok(app_name)
            } else {
                self.initialize_vsomeip_app(&registration_type).await
            }
        };

        let Ok(app_name) = app_name_res else {
            // we failed to start the vsomeip application
            let _ = self
                .get_storage()
                .get_extern_fn_registry_write()
                .await
                .free_listener_id(listener_id)
                .await;

            let _ = self
                .get_storage()
                .get_registry_write()
                .await
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id);

            return Err(app_name_res.err().unwrap());
        };

        let insert_res = self
            .get_storage()
            .get_registry_write()
            .await
            .set_client_and_app_name(registration_type.client_id(), app_name);
        if let Err(err) = insert_res {
            let _ = self
                .get_storage()
                .get_extern_fn_registry_write()
                .await
                .free_listener_id(listener_id)
                .await;

            let _ = self
                .get_storage()
                .get_registry_write()
                .await
                .remove_listener_id_and_listener_config_based_on_listener_id(listener_id);

            let Some(app_name) = self
                .get_storage()
                .get_registry_write()
                .await
                .remove_app_name_for_client_id(registration_type.client_id())
            else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Unable to find app_name for listener_id: {listener_id}"),
                ));
            };
            trace!(
                "No more remaining listeners for client_id: {} app_name: {app_name}",
                registration_type.client_id()
            );

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
            Self::await_internal_function(UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP, rx).await?;
        }

        let (tx, rx) = oneshot::channel();
        let extern_fn = self
            .get_storage()
            .get_extern_fn_registry_read()
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
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_inner_res {
            // TODO: Consider if we'd like to restart engine or if just indeterminate state and should panic
            panic!("engine has stopped! unable to proceed!");
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

        self.get_storage()
            .get_registry_read()
            .await
            .get_app_name_for_client_id(registration_type.client_id())
            .ok_or(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "No application found for client_id: {}",
                    registration_type.client_id()
                ),
            ))?;

        let (tx, rx) = oneshot::channel();
        let send_to_inner_res = Self::send_to_inner_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::UnregisterListener(src, sink, registration_type.clone(), tx),
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
            .get_registry_read()
            .await
            .get_listener_id_for_listener_config(listener_config)
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                "Unable to find listener_id for listener_config",
            ));
        };

        let Some(client_id) = self
            .get_storage()
            .get_registry_write()
            .await
            .remove_client_id_based_on_listener_id(listener_id)
        else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("Unable to find client_id for listener_id: {listener_id}"),
            ));
        };

        if let Err(err) = self
            .get_storage()
            .get_extern_fn_registry_write()
            .await
            .free_listener_id(listener_id)
            .await
        {
            warn!("Unable to free listener_id: {listener_id} with err: {err:?}");
        }

        if let Err(err) = self
            .get_storage()
            .get_extern_fn_registry_write()
            .await
            .remove_listener_id_transport(listener_id)
            .await
        {
            warn!("Unable to remove storage for listener_id: {listener_id} with err: {err:?}");
        }

        if self
            .get_storage()
            .get_registry_read()
            .await
            .listener_count_for_client_id(client_id)
            == 0
        {
            let Some(app_name) = self
                .get_storage()
                .get_registry_write()
                .await
                .remove_app_name_for_client_id(client_id)
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
                .get_registry_read()
                .await
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
            &message,
            source_filter,
            sink_filter,
            message_type.clone(),
        )
        .await?;

        let (tx, rx) = oneshot::channel();
        let send_to_inner_res = Self::send_to_inner_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::Send(message, message_type, tx),
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
        oneshot::Sender<Result<(), UStatus>>,
    ),
    UnregisterListener(
        UUri,
        Option<UUri>,
        RegistrationType,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    Send(
        UMessage,
        RegistrationType,
        oneshot::Sender<Result<(), UStatus>>,
    ),
    // Additional helpful commands
    StartVsomeipApp(
        uuid::Uuid,
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
                    return_channel,
                ) => {
                    trace!(
                        "{}:{} - Attempting to register listener: src: {src:?} sink: {sink:?}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                    );

                    trace!("registration_type: {registration_type:?}");

                    let app_name = Registry::find_app_name(registration_type.client_id()).await;

                    let Ok(app_name) = app_name else {
                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel)
                            .await;
                        continue;
                    };

                    let_cxx_string!(app_name_cxx = app_name);

                    let mut application_wrapper = make_application_wrapper(
                        get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                    );

                    let res = Self::register_listener_internal(
                        src,
                        sink,
                        registration_type,
                        msg_handler,
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
                    return_channel,
                ) => {
                    let app_name = Registry::find_app_name(registration_type.client_id()).await;

                    let Ok(app_name) = app_name else {
                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel)
                            .await;
                        continue;
                    };

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
                TransportCommand::Send(umsg, message_type, return_channel) => {
                    trace!(
                        "{}:{} - Attempting to send UMessage: {:?}",
                        UP_CLIENT_VSOMEIP_TAG,
                        UP_CLIENT_VSOMEIP_FN_TAG_APP_EVENT_LOOP,
                        umsg
                    );

                    trace!(
                        "inside TransportCommand::Send dispatch, message_type: {message_type:?}"
                    );

                    let app_name = Registry::find_app_name(message_type.client_id()).await;

                    let Ok(app_name) = app_name else {
                        Self::return_oneshot_result(Err(app_name.err().unwrap()), return_channel)
                            .await;
                        continue;
                    };

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

                    let res =
                        Self::send_internal(umsg, &mut application_wrapper, &runtime_wrapper).await;
                    Self::return_oneshot_result(res, return_channel).await;
                }
                TransportCommand::StartVsomeipApp(
                    transport_instance_id,
                    client_id,
                    app_name,
                    return_channel,
                ) => {
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

                    let add_res = Registry::add_client_id_app_name(client_id, &app_name).await;
                    let insert_res =
                        Registry::insert_instance_client_id(transport_instance_id, client_id).await;

                    error!("attempt to insert result into instance to client_ids mapping: instance_id: {} : {insert_res:?}", transport_instance_id.hyphenated().to_string());

                    let instance_id_client_ids =
                        Registry::get_instance_client_ids(transport_instance_id).await;
                    error!("after insertion into instance_client_ids: {instance_id_client_ids:?}");

                    Self::return_oneshot_result(add_res, return_channel).await;
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

                if !VsomeipOfferedRequested::is_event_requested(service_id, instance_id, event_id)
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
                    VsomeipOfferedRequested::insert_event_requested(
                        service_id,
                        instance_id,
                        event_id,
                    )
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

                if !VsomeipOfferedRequested::is_service_offered(service_id, instance_id, method_id)
                    .await
                {
                    get_pinned_application(application_wrapper).offer_service(
                        service_id,
                        instance_id,
                        ANY_MAJOR,
                        ANY_MINOR,
                    );
                    VsomeipOfferedRequested::insert_service_offered(
                        service_id,
                        instance_id,
                        method_id,
                    )
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

                if !VsomeipOfferedRequested::is_service_requested(
                    service_id,
                    instance_id,
                    method_id,
                )
                .await
                {
                    get_pinned_application(application_wrapper).request_service(
                        service_id,
                        instance_id,
                        ANY_MAJOR,
                        ANY_MINOR,
                    );
                    VsomeipOfferedRequested::insert_service_requested(
                        service_id,
                        instance_id,
                        method_id,
                    )
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
                    application_wrapper,
                    runtime_wrapper,
                )
                .await;
            }
            UMessageType::UMESSAGE_TYPE_REQUEST | UMessageType::UMESSAGE_TYPE_RESPONSE => {
                let _vsomeip_msg_res = convert_umsg_to_vsomeip_msg_and_send(
                    &umsg,
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
        let found_app_res = Registry::find_app_name(client_id).await;

        if found_app_res.is_ok() {
            let err = UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "Application already exists for client_id: {} app_name: {}",
                    client_id, app_name
                ),
            );
            return Err(err);
        }

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

        let mut app_wrapper = make_application_wrapper(
            get_pinned_runtime(runtime_wrapper).get_application(&app_name_cxx),
        );

        get_pinned_application(&mut app_wrapper).stop();

        Ok(())
    }
}

// TODO: Add unit tests
