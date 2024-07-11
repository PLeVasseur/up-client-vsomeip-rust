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
use crate::transport_inner::{TransportCommand, UP_CLIENT_VSOMEIP_FN_TAG_STOP_APP};
use crate::transport_inner::{
    UP_CLIENT_VSOMEIP_FN_TAG_INITIALIZE_NEW_APP_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER_INTERNAL, UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL,
    UP_CLIENT_VSOMEIP_FN_TAG_UNREGISTER_LISTENER_INTERNAL, UP_CLIENT_VSOMEIP_TAG,
};
use crate::vsomeip_config::extract_applications;
use crate::{
    any_uuri, any_uuri_fixed_authority_id, ApplicationName, MockableUPTransportVsomeipInner,
};
use crate::{InstanceId, UPTransportVsomeip};
use async_trait::async_trait;
use futures::executor;
use log::{error, trace, warn};
use once_cell::sync::Lazy;
use protobuf::EnumOrUnknown;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::mpsc::Sender;
use tokio::sync::oneshot;
use tokio::time::timeout;
use up_rust::{
    ComparableListener, UAttributesValidators, UCode, UListener, UMessage, UMessageType, UStatus,
    UTransport, UUri,
};
use vsomeip_sys::extern_callback_wrappers::MessageHandlerFnPtr;

const UP_CLIENT_VSOMEIP_FN_TAG_REGISTER_LISTENER: &str = "register_listener";

const INTERNAL_FUNCTION_TIMEOUT: u64 = 3;

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
    tx: &tokio::sync::mpsc::Sender<TransportCommand>,
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

static RUNTIME: Lazy<Arc<Runtime>> =
    Lazy::new(|| Arc::new(Runtime::new().expect("Failed to create Tokio runtime")));

#[async_trait]
impl UTransport for UPTransportVsomeip {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        self.transport_inner.send(message).await
    }

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.transport_inner
            .register_listener(source_filter, sink_filter, listener)
            .await
    }

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.transport_inner
            .unregister_listener(source_filter, sink_filter, listener)
            .await
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

// TODO: Update this
impl Drop for UPTransportVsomeip {
    fn drop(&mut self) {
        // let instance_id = self.instance_id.clone();
        // error!(
        //     "dropping UPTransportVsomeip with instance_id: {}",
        //     instance_id.hyphenated().to_string()
        // );
        // let transport_command_sender = self.inner_transport.transport_command_sender.clone();
        //
        // // Create a oneshot channel to wait for task completion
        // let (tx, rx) = oneshot::channel();
        //
        // // Get the handle of the current runtime
        // let handle = Handle::current();
        //
        // std::thread::spawn(move || {
        //     handle.block_on(async move {
        //         Self::delete_registry_items_internal(instance_id, transport_command_sender).await;
        //         // Notify that the task is complete
        //         let _ = tx.send(());
        //     });
        // });
        //
        // // Wait for the task to complete
        // let _ = executor::block_on(rx);
    }
}
