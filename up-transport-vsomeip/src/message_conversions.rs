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

use crate::frame_wire::decode_frame_payload;
use crate::storage::rpc_correlation::RpcCorrelationRegistry;
use crate::storage::vsomeip_offered_requested::VsomeipOfferedRequestedRegistry;
use crate::utils::{split_u32_to_u16, split_u32_to_u8};
use crate::{AuthorityName, EventId, InstanceId, ServiceId};
use cxx::UniquePtr;
use log::trace;
use std::sync::Arc;
use std::time::Duration;
use up_rust::{UCode, UOwnedFrame, UStatus, UUri};
use vsomeip_sys::glue::{make_message_wrapper, ApplicationWrapper, MessageWrapper, RuntimeWrapper};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::{message_type_e, ANY_MAJOR};

const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_FRAME_TO_VSOMEIP_MSG: &str = "convert_frame_to_vsomeip_msg";
pub struct UFrameToVsomeipMessage;

impl UFrameToVsomeipMessage {
    pub async fn frame_publish_to_vsomeip_notification(
        frame: &UOwnedFrame,
        vsomeip_offered_requested_registry: Arc<dyn VsomeipOfferedRequestedRegistry>,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    ) -> Result<(ServiceId, InstanceId, EventId), UStatus> {
        let source = frame.metadata().attributes().source();

        let (_instance_id, service_id) = split_u32_to_u16(source.ue_id());
        let instance_id = 1;
        let (_, event_id) = split_u32_to_u16(source.resource_id_raw());
        let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major());
        trace!("uProtocol Publish frame's interface_version: {interface_version}");

        if !vsomeip_offered_requested_registry.is_event_offered(service_id, instance_id, event_id) {
            application_wrapper.get_pinned().offer_service(
                service_id,
                instance_id,
                ANY_MAJOR,
                vsomeip::ANY_MINOR,
            );
            (*application_wrapper).offer_single_event_safe(
                service_id,
                instance_id,
                event_id,
                event_id,
            );
            tokio::time::sleep(Duration::from_nanos(5)).await;
            vsomeip_offered_requested_registry.insert_event_offered(
                service_id,
                instance_id,
                event_id,
            );
        }

        Ok((service_id, instance_id, event_id))
    }

    pub async fn frame_request_to_vsomeip_message(
        frame: &UOwnedFrame,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<UniquePtr<MessageWrapper>, UStatus> {
        let source = frame.metadata().attributes().source();
        let sink = frame.metadata().attributes().sink().ok_or_else(|| {
            UStatus::fail_with_code(UCode::INVALID_ARGUMENT, "Request frame has no sink UUri")
        })?;

        let vsomeip_msg = make_message_wrapper(runtime_wrapper.get_pinned().create_request(true));
        let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id());
        trace!(
            "{} - sink.ue_id: {} source.ue_id: {} _instance_id: {} service_id:{}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_FRAME_TO_VSOMEIP_MSG,
            sink.ue_id(),
            source.ue_id(),
            _instance_id,
            service_id
        );
        vsomeip_msg
            .get_message_base_pinned()
            .set_service(service_id);
        let instance_id = 1;
        vsomeip_msg
            .get_message_base_pinned()
            .set_instance(instance_id);
        let (_, method_id) = split_u32_to_u16(sink.resource_id_raw());
        vsomeip_msg.get_message_base_pinned().set_method(method_id);
        let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major());
        vsomeip_msg
            .get_message_base_pinned()
            .set_interface_version(interface_version);

        let app_client_id = application_wrapper.get_pinned().get_client();
        trace!(
            "{} - app_client_id for Request frame: {} request_uuid: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_FRAME_TO_VSOMEIP_MSG,
            app_client_id,
            frame.metadata().attributes().id().to_hyphenated_string(),
        );

        vsomeip_msg
            .get_message_base_pinned()
            .set_return_code(vsomeip::return_code_e::E_OK);

        Ok(vsomeip_msg)
    }

    pub async fn frame_response_to_vsomeip_message(
        frame: &UOwnedFrame,
        rpc_correlation_registry: Arc<dyn RpcCorrelationRegistry>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<UniquePtr<MessageWrapper>, UStatus> {
        let source = frame.metadata().attributes().source();

        let vsomeip_msg = make_message_wrapper(runtime_wrapper.get_pinned().create_message(true));
        let (_instance_id, service_id) = split_u32_to_u16(source.ue_id());
        vsomeip_msg
            .get_message_base_pinned()
            .set_service(service_id);
        let instance_id = 1;
        vsomeip_msg
            .get_message_base_pinned()
            .set_instance(instance_id);
        let (_, method_id) = split_u32_to_u16(source.resource_id_raw());
        vsomeip_msg.get_message_base_pinned().set_method(method_id);
        let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major());
        vsomeip_msg
            .get_message_base_pinned()
            .set_interface_version(interface_version);

        let request_uuid = frame.metadata().attributes().request_id().ok_or_else(|| {
            UStatus::fail_with_code(
                UCode::INVALID_ARGUMENT,
                "Response frame has no request_id for SOME/IP request correlation",
            )
        })?;
        let request_id = rpc_correlation_registry.remove_me_request_correlation(request_uuid)?;

        let (client_id, session_id) = split_u32_to_u16(request_id);
        vsomeip_msg.get_message_base_pinned().set_client(client_id);
        vsomeip_msg
            .get_message_base_pinned()
            .set_session(session_id);

        let (return_code, vsomeip_msg_type) = match frame.metadata().attributes().commstatus() {
            Some(UCode::OK) | None => (vsomeip::return_code_e::E_OK, message_type_e::MT_RESPONSE),
            Some(commstatus) => (
                Self::ucode_to_vsomeip_err_code(commstatus),
                message_type_e::MT_ERROR,
            ),
        };
        vsomeip_msg
            .get_message_base_pinned()
            .set_return_code(return_code);
        vsomeip_msg
            .get_message_base_pinned()
            .set_message_type(vsomeip_msg_type);

        Ok(vsomeip_msg)
    }

    fn ucode_to_vsomeip_err_code(ucode: UCode) -> vsomeip::return_code_e {
        match ucode {
            UCode::OK => vsomeip::return_code_e::E_OK,
            UCode::INVALID_ARGUMENT => vsomeip::return_code_e::E_WRONG_MESSAGE_TYPE,
            UCode::DEADLINE_EXCEEDED => vsomeip::return_code_e::E_TIMEOUT,
            UCode::NOT_FOUND => vsomeip::return_code_e::E_UNKNOWN_SERVICE,
            UCode::UNAVAILABLE => vsomeip::return_code_e::E_UNKNOWN_SERVICE,
            UCode::DATA_LOSS => vsomeip::return_code_e::E_MALFORMED_MESSAGE,
            UCode::INTERNAL => vsomeip::return_code_e::E_NOT_REACHABLE,
            UCode::UNKNOWN => vsomeip::return_code_e::E_NOT_OK,
            UCode::FAILED_PRECONDITION => vsomeip::return_code_e::E_WRONG_PROTOCOL_VERSION,
            _ => vsomeip::return_code_e::E_UNKNOWN,
        }
    }
}

pub struct VsomeipMessageToUFrame;

impl VsomeipMessageToUFrame {
    pub async fn convert_vsomeip_msg_to_frame(
        _authority_name: &AuthorityName,
        _self_uuri: &UUri,
        _mechatronics_authority_name: &AuthorityName,
        rpc_correlation_registry: Arc<dyn RpcCorrelationRegistry>,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
    ) -> Result<UOwnedFrame, UStatus> {
        let msg_type = vsomeip_message.get_message_base_pinned().get_message_type();
        let payload_bytes = {
            let Some(payload) = (*vsomeip_message).get_message_payload() else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to extract PayloadWrapper from MessageWrapper",
                ));
            };
            payload.get_data_safe()
        };

        let frame = decode_frame_payload(payload_bytes)?;
        match msg_type {
            message_type_e::MT_REQUEST => {
                let request_id = vsomeip_message.get_message_base_pinned().get_request();
                rpc_correlation_registry.insert_me_request_correlation(
                    frame.metadata().attributes().id().clone(),
                    request_id,
                )?;
            }
            message_type_e::MT_NOTIFICATION
            | message_type_e::MT_RESPONSE
            | message_type_e::MT_ERROR => {}
            _ => {
                return Err(UStatus::fail_with_code(
                    UCode::OUT_OF_RANGE,
                    format!("Not one of the handled message types from SOME/IP: {msg_type:?}"),
                ));
            }
        }

        Ok(frame)
    }
}
