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
    ComparableListener, LocalUriProvider, PayloadEncoding, UAttributesValidators, UCode, UListener,
    UMessage, UStatus, UTransport, UUri,
};

#[async_trait]
impl UTransport for UPTransportVsomeip {
    async fn send(&self, message: UMessage) -> Result<(), UStatus> {
        let attributes = message.attributes();

        // Validate UAttributes before conversion.
        UAttributesValidators::validator_for_attributes(attributes)
            .validate(attributes)
            .map_err(|e| {
                UStatus::fail_with_code(
                    UCode::InvalidArgument,
                    format!("Invalid uAttributes, err: {e:?}"),
                )
            })?;

        trace!("Sending message with attributes: {:?}", attributes);

        let source_filter = message.source();
        let sink_filter = message.sink();
        let message_type = determine_type(source_filter, &sink_filter.cloned())?;
        trace!("inside send(), message_type: {message_type:?}");

        validate_payload_encoding_matches_assumption(
            &message,
            self.storage.get_assumed_payload_encoding(),
        )?;

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
                message,
                message_type,
                app_name,
                self.storage.clone(),
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

    async fn register_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        let registration_type = determine_type(source_filter, &sink_filter.cloned())?;

        trace!("registration_type: {registration_type:?}");

        if registration_type == RegistrationType::AllPointToPoint {
            return self.register_point_to_point_listener(&listener).await;
        }

        let app_name = self.storage.get_vsomeip_application_config().name;

        let comp_listener = ComparableListener::new(listener);
        let listener_config = (source_filter.clone(), sink_filter.cloned(), comp_listener);
        let Ok(msg_handler) = self
            .storage
            .get_message_handler(self.storage.clone(), listener_config)
        else {
            return Err(UStatus::fail_with_code(
                UCode::Internal,
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

    async fn unregister_listener(
        &self,
        source_filter: &UUri,
        sink_filter: Option<&UUri>,
        listener: Arc<dyn UListener>,
    ) -> Result<(), UStatus> {
        self.unregister_listener(source_filter, sink_filter, listener)
    }

    async fn receive(
        &self,
        _source_filter: &UUri,
        _sink_filter: Option<&UUri>,
    ) -> Result<UMessage, UStatus> {
        Err(UStatus::fail_with_code(
            UCode::Unimplemented,
            "This method is not implemented for vsomeip. Use register_listener instead.",
        ))
    }
}

impl LocalUriProvider for UPTransportVsomeip {
    fn get_authority(&self) -> String {
        self.storage.get_uri().authority_name().to_owned()
    }
    fn get_resource_uri(&self, resource_id: u16) -> UUri {
        self.storage.get_uri().clone_with_resource_id(resource_id)
    }
    fn get_source_uri(&self) -> UUri {
        self.storage.get_uri()
    }
}

fn validate_payload_encoding_matches_assumption(
    message: &UMessage,
    assumed_payload_encoding: PayloadEncoding,
) -> Result<(), UStatus> {
    match (message.payload().is_some(), message.payload_encoding()) {
        (false, None) => Ok(()),
        (true, Some(actual)) if actual == assumed_payload_encoding => Ok(()),
        (true, Some(actual)) => Err(UStatus::fail_with_code(
            UCode::InvalidArgument,
            format!(
                "payload encoding `{actual}` does not match configured SOME/IP assumed payload encoding `{assumed_payload_encoding}`"
            ),
        )),
        (true, None) => Err(UStatus::fail_with_code(
            UCode::InvalidArgument,
            "payload-bearing SOME/IP message does not declare a payload encoding",
        )),
        (false, Some(_)) => Err(UStatus::fail_with_code(
            UCode::InvalidArgument,
            "payloadless SOME/IP message declares a payload encoding",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use up_rust::UMessageBuilder;

    fn topic() -> UUri {
        UUri::try_from_parts("authority", 0x1234, 1, 0x8001).unwrap()
    }

    #[test]
    fn matching_registered_and_private_encodings_are_accepted() {
        let encodings = [
            PayloadEncoding::PROTOBUF_WRAPPED_IN_ANY,
            PayloadEncoding::PROTOBUF,
            PayloadEncoding::JSON,
            PayloadEncoding::SOMEIP,
            PayloadEncoding::SOMEIP_TLV,
            PayloadEncoding::RAW,
            PayloadEncoding::TEXT,
            PayloadEncoding::SHM,
            PayloadEncoding::from_registry_entry(0x1000_0042),
        ];

        for encoding in encodings {
            let message = UMessageBuilder::publish(topic())
                .build_with_payload(Vec::<u8>::new(), encoding)
                .unwrap();
            validate_payload_encoding_matches_assumption(&message, encoding).unwrap();
        }
    }

    #[test]
    fn payload_encoding_mismatch_is_rejected() {
        let message = UMessageBuilder::publish(topic())
            .build_with_payload("payload", PayloadEncoding::TEXT)
            .unwrap();
        let error =
            validate_payload_encoding_matches_assumption(&message, PayloadEncoding::PROTOBUF)
                .unwrap_err();

        assert_eq!(error.code(), UCode::InvalidArgument);
        assert!(error
            .to_string()
            .contains("does not match configured SOME/IP assumed payload encoding"));
    }

    #[test]
    fn payloadless_message_remains_identity_free() {
        let message = UMessageBuilder::publish(topic()).build().unwrap();

        validate_payload_encoding_matches_assumption(&message, PayloadEncoding::TEXT).unwrap();
        assert!(message.payload().is_none());
        assert!(message.payload_encoding().is_none());
    }
}
