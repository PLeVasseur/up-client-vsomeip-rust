/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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

use crate::determine_message_type::{determine_type, RegistrationType};
use crate::storage::message_handler_registry::MessageHandlerRegistry;
use crate::transport_engine::TransportCommand;
use crate::transport_engine::UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL;
use crate::UPTransportVsomeip;
use async_trait::async_trait;
use log::trace;
use std::sync::Arc;
use tokio::sync::oneshot;
use up_rust::{
    transport::{ComparableOwnedListener, UOwnedTransportImpl, ValidatedOwnedFrame},
    LocalUriProvider, UCode, UOwnedListener, UStatus, UUri,
};

#[async_trait]
impl UOwnedTransportImpl for UPTransportVsomeip {
    async fn send_validated_owned(&self, frame: ValidatedOwnedFrame) -> Result<(), UStatus> {
        let frame = frame.into_inner();
        let attributes = frame.metadata().attributes();
        trace!("Sending native frame with attributes: {:?}", attributes);

        let source_filter = attributes.source();
        let sink_filter = attributes.sink();
        let message_type = determine_type(source_filter, &sink_filter.cloned())?;
        trace!("inside send_owned(), message_type: {message_type:?}");

        let app_name = self.storage.get_vsomeip_application_config().name;

        self.register_for_returning_response_if_point_to_point_listener_and_sending_request(
            source_filter,
            sink_filter,
            message_type.clone(),
        )
        .await?;

        let (tx, rx) = oneshot::channel();
        let send_to_engine_res = Self::send_to_engine_with_status(
            &self.engine.transport_command_sender,
            TransportCommand::Send(
                Box::new(frame),
                message_type,
                app_name,
                self.storage.clone(),
                self.storage.clone(),
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! with err: {err:?}");
        }
        Self::await_engine(UP_CLIENT_VSOMEIP_FN_TAG_SEND_INTERNAL, rx).await
    }

    async fn register_validated_owned_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UOwnedListener>,
    ) -> Result<(), UStatus> {
        let registration_type = determine_type(source_filter, &sink_filter.cloned())?;

        trace!("registration_type: {registration_type:?}");

        if registration_type == RegistrationType::AllPointToPoint {
            return self.register_point_to_point_listener(&listener).await;
        }

        let app_name = self.storage.get_vsomeip_application_config().name;

        let comp_listener = ComparableOwnedListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);
        let Ok(msg_handler) = self
            .storage
            .get_message_handler(self.storage.clone(), listener_config)
        else {
            return Err(UStatus::fail_with_code(
                UCode::INTERNAL,
                "Unable to get message handler for register_owned_listener",
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
                self.storage.clone(),
                tx,
            ),
        )
        .await;
        if let Err(err) = send_to_engine_res {
            panic!("engine has stopped! unable to proceed! err: {err}");
        }

        Self::await_engine("register", rx).await
    }

    async fn unregister_validated_owned_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UOwnedListener>,
    ) -> Result<(), UStatus> {
        self.unregister_listener(source_filter, sink_filter, listener)
    }
}

impl LocalUriProvider for UPTransportVsomeip {
    fn get_authority(&self) -> String {
        self.storage.get_uri().authority_name()
    }

    fn get_resource_uri(&self, resource_id: u16) -> UUri {
        self.storage.get_uri().with_resource_id(resource_id)
    }

    fn get_source_uri(&self) -> UUri {
        self.storage.get_uri()
    }
}
