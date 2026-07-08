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

use crate::storage::rpc_correlation::RpcCorrelationRegistry;
use crate::storage::vsomeip_offered_requested::VsomeipOfferedRequestedRegistry;
use crate::utils::{
    create_request_id, create_ue_id_from_instance_service, split_u32_to_u16,
    split_ue_id_to_instance_service, uuri_ue_id,
};
use crate::{AuthorityName, EventId, InstanceId, ServiceId};
use cxx::UniquePtr;
use log::trace;
use std::sync::Arc;
use std::time::Duration;
use up_rust::{PayloadEncoding, UCode, UMessage, UMessageBuilder, UStatus, UUri};
use vsomeip_sys::glue::{make_message_wrapper, ApplicationWrapper, MessageWrapper, RuntimeWrapper};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::{message_type_e, ANY_MAJOR};

const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG: &str = "convert_umsg_to_vsomeip_msg";
const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG: &str = "convert_vsomeip_msg_to_umsg";

pub struct UMessageToVsomeipMessage;

impl UMessageToVsomeipMessage {
    // TODO: Consider passing in the Arc<RpcCorrelation> and Arc<VsomeipOfferedRequested>
    //  to minimize the amount of data we have to pass through to more clearly communicate
    //  what will be done
    pub async fn umsg_publish_to_vsomeip_notification(
        umsg: &UMessage,
        vsomeip_offered_requested_registry: Arc<dyn VsomeipOfferedRequestedRegistry>,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    ) -> Result<(ServiceId, InstanceId, EventId), UStatus> {
        let source = umsg.source();

        let (instance_id, service_id) = split_ue_id_to_instance_service(uuri_ue_id(source));
        let (_, event_id) = split_u32_to_u16(u32::from(source.resource_id()));
        let interface_version = source.uentity_major_version();
        trace!("uProtocol Publish message's interface_version: {interface_version}");

        trace!(
            "Immediately prior to request_service: service_id: {} instance_id: {}",
            service_id,
            instance_id
        );

        // TODO: We also need to add a corresponding stop_offer_event perhaps when we drop
        //  the UPClientVsomeip?
        if !vsomeip_offered_requested_registry.is_event_offered(service_id, instance_id, event_id) {
            application_wrapper.get_pinned().offer_service(
                service_id,
                instance_id,
                ANY_MAJOR,
                // interface_version,
                vsomeip::ANY_MINOR,
            );
            (*application_wrapper).offer_single_event_safe(
                service_id,
                instance_id,
                event_id,
                event_id,
            );
            trace!("doing event offered");
            // TODO: We should replace this with using a vsomeip register_availability_handler()
            //  and then we block with timeout while waiting on receiving availability
            //  change over a channel and if we exceed timeout then we can return an error
            //
            // Initial prototyping of this in vsomeip-sys doesn't look promising
            // Seems it's possible to start sending messages before this application's offered
            // service and event are "understood" by other applications
            // Leaving sleep for now till thinking of some better idea
            tokio::time::sleep(Duration::from_nanos(5)).await;
            vsomeip_offered_requested_registry.insert_event_offered(
                service_id,
                instance_id,
                event_id,
            );
        }

        trace!("Immediately after request_service");

        Ok((service_id, instance_id, event_id))
    }

    pub async fn umsg_request_to_vsomeip_message(
        umsg: &UMessage,
        rpc_correlation_registry: Arc<dyn RpcCorrelationRegistry>,
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<UniquePtr<MessageWrapper>, UStatus>
where {
        let source = umsg.source();

        let Some(sink) = umsg.sink() else {
            return Err(UStatus::fail_with_code(
                UCode::InvalidArgument,
                "Message has no sink UUri",
            ));
        };

        let vsomeip_msg = make_message_wrapper(runtime_wrapper.get_pinned().create_request(true));
        let (instance_id, service_id) = split_ue_id_to_instance_service(uuri_ue_id(sink));
        trace!(
            "{} - sink.ue_id: {} source.ue_id: {} instance_id: {} service_id:{}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            uuri_ue_id(sink),
            uuri_ue_id(source),
            instance_id,
            service_id
        );
        vsomeip_msg
            .get_message_base_pinned()
            .set_service(service_id);
        vsomeip_msg
            .get_message_base_pinned()
            .set_instance(instance_id);
        let (_, method_id) = split_u32_to_u16(u32::from(sink.resource_id()));
        vsomeip_msg.get_message_base_pinned().set_method(method_id);
        let interface_version = sink.uentity_major_version();
        vsomeip_msg
            .get_message_base_pinned()
            .set_interface_version(interface_version);

        let req_id = umsg.id();
        let app_client_id = application_wrapper.get_pinned().get_client();
        let app_session_id = rpc_correlation_registry.retrieve_session_id(app_client_id);
        let request_id = create_request_id(app_client_id, app_session_id);
        trace!("{} - client_id: {} session_id: {} request_id: {} service_id: {} app_client_id: {} app_session_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                app_client_id, app_session_id, request_id, service_id, app_client_id, app_session_id
            );
        let app_request_id = create_request_id(app_client_id, app_session_id);
        trace!("{} - (app_request_id, req_id) to store for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                app_request_id, req_id.to_hyphenated_string(),
            );
        rpc_correlation_registry.insert_ue_request_correlation(app_request_id, req_id, source)?;

        vsomeip_msg
            .get_message_base_pinned()
            .set_return_code(vsomeip::return_code_e::E_OK);

        Ok(vsomeip_msg)
    }

    pub async fn umsg_response_to_vsomeip_message(
        umsg: &UMessage,
        rpc_correlation_registry: Arc<dyn RpcCorrelationRegistry>,
        runtime_wrapper: &UniquePtr<RuntimeWrapper>,
    ) -> Result<UniquePtr<MessageWrapper>, UStatus> {
        let source = umsg.source();

        trace!(
            "{} - Attempting to send Response",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
        );

        let vsomeip_msg = make_message_wrapper(runtime_wrapper.get_pinned().create_message(true));

        let (instance_id, service_id) = split_ue_id_to_instance_service(uuri_ue_id(source));
        vsomeip_msg
            .get_message_base_pinned()
            .set_service(service_id);
        vsomeip_msg
            .get_message_base_pinned()
            .set_instance(instance_id);
        let (_, method_id) = split_u32_to_u16(u32::from(source.resource_id()));
        vsomeip_msg.get_message_base_pinned().set_method(method_id);
        let interface_version = source.uentity_major_version();
        vsomeip_msg
            .get_message_base_pinned()
            .set_interface_version(interface_version);

        let req_id = umsg.request_id().ok_or_else(|| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                "Missing id for Request message. Would be unable to correlate. Rejected.",
            )
        })?;
        trace!(
            "{} - Looking up req_id from UMessage in ME_REQUEST_CORRELATION, req_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            req_id.to_hyphenated_string()
        );

        let request_id = rpc_correlation_registry.remove_me_request_correlation(req_id)?;

        trace!(
            "{} - Found correlated request_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            request_id
        );
        let (client_id, session_id) = split_u32_to_u16(request_id);
        vsomeip_msg.get_message_base_pinned().set_client(client_id);
        vsomeip_msg
            .get_message_base_pinned()
            .set_session(session_id);
        trace!(
            "{} - request_id: {} client_id: {} session_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            request_id,
            client_id,
            session_id
        );

        let (return_code, vsomeip_msg_type) = match umsg.commstatus() {
            Some(UCode::Ok) | None => (vsomeip::return_code_e::E_OK, message_type_e::MT_RESPONSE),
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

        trace!(
            "{} - Response: Finished building vsomeip message: service_id: {} instance_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            service_id,
            instance_id
        );

        Ok(vsomeip_msg)
    }

    fn ucode_to_vsomeip_err_code(ucode: UCode) -> vsomeip::return_code_e {
        // TODO handle the one-to-many mapping. eg: INVALID_ARGUMENT <==> E_WRONG_MESSAGE_TYPE / E_UNKNOWN_METHOD
        match ucode {
            UCode::Ok => vsomeip::return_code_e::E_OK,
            UCode::InvalidArgument => vsomeip::return_code_e::E_WRONG_MESSAGE_TYPE,
            UCode::DeadlineExceeded => vsomeip::return_code_e::E_TIMEOUT,
            UCode::NotFound => vsomeip::return_code_e::E_UNKNOWN_SERVICE,
            UCode::Unavailable => vsomeip::return_code_e::E_UNKNOWN_SERVICE,
            UCode::DataLoss => vsomeip::return_code_e::E_MALFORMED_MESSAGE,
            UCode::Internal => vsomeip::return_code_e::E_NOT_REACHABLE,
            UCode::Unknown => vsomeip::return_code_e::E_NOT_OK,
            UCode::FailedPrecondition => vsomeip::return_code_e::E_WRONG_PROTOCOL_VERSION,
            _ => vsomeip::return_code_e::E_UNKNOWN,
        }
    }
}

pub struct VsomeipMessageToUMessage;

impl VsomeipMessageToUMessage {
    pub async fn convert_vsomeip_msg_to_umsg(
        authority_name: &AuthorityName,
        self_uuri: &UUri,
        mechatronics_authority_name: &AuthorityName,
        assumed_payload_encoding: PayloadEncoding,
        rpc_correlation_registry: Arc<dyn RpcCorrelationRegistry>,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
    ) -> Result<UMessage, UStatus> {
        let msg_type = vsomeip_message.get_message_base_pinned().get_message_type();

        let payload_bytes = {
            let Some(payload) = (*vsomeip_message).get_message_payload() else {
                return Err(UStatus::fail_with_code(
                    UCode::Internal,
                    "Unable to extract PayloadWrapper from MessageWrapper",
                ));
            };
            payload.get_data_safe()
        };

        match msg_type {
            message_type_e::MT_REQUEST => {
                Self::convert_vsomeip_mt_request_to_umsg(
                    authority_name,
                    self_uuri,
                    mechatronics_authority_name,
                    &rpc_correlation_registry,
                    vsomeip_message,
                    payload_bytes,
                    &assumed_payload_encoding,
                )
                .await
            }
            message_type_e::MT_NOTIFICATION => {
                Self::convert_vsomeip_mt_notification_to_umsg(
                    mechatronics_authority_name,
                    &assumed_payload_encoding,
                    vsomeip_message,
                    payload_bytes,
                )
                .await
            }
            message_type_e::MT_RESPONSE => {
                Self::convert_vsomeip_mt_response_to_umsg(
                    mechatronics_authority_name,
                    &rpc_correlation_registry,
                    vsomeip_message,
                    payload_bytes,
                    &assumed_payload_encoding,
                )
                .await
            }
            message_type_e::MT_ERROR => {
                Self::convert_vsomeip_mt_error_to_umsg(
                    mechatronics_authority_name,
                    &rpc_correlation_registry,
                    vsomeip_message,
                    payload_bytes,
                    &assumed_payload_encoding,
                )
                .await
            }
            _ => Err(UStatus::fail_with_code(
                UCode::OutOfRange,
                format!(
                    "Not one of the handled message types from SOME/IP: {:?}",
                    msg_type
                ),
            )),
        }
    }

    async fn convert_vsomeip_mt_request_to_umsg(
        authority_name: &AuthorityName,
        self_uuri: &UUri,
        mechatronics_authority_name: &AuthorityName,
        rpc_correlation_registry: &Arc<dyn RpcCorrelationRegistry>,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
        payload_bytes: Vec<u8>,
        assumed_payload_encoding: &PayloadEncoding,
    ) -> Result<UMessage, UStatus> {
        let request_id = vsomeip_message.get_message_base_pinned().get_request();
        let service_id = vsomeip_message.get_message_base_pinned().get_service();
        let method_id = vsomeip_message.get_message_base_pinned().get_method();
        let interface_version = vsomeip_message
            .get_message_base_pinned()
            .get_interface_version();

        trace!("MT_REQUEST type");
        let sink = UUri::try_from_parts(
            authority_name,
            create_ue_id_from_instance_service(
                vsomeip_message.get_message_base_pinned().get_instance(),
                service_id,
            ),
            interface_version,
            method_id,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                format!("Unable to build sink UUri for MT_REQUEST type: {e:?}"),
            )
        })?;

        let source = UUri::try_from_parts(
            mechatronics_authority_name,
            uuri_ue_id(self_uuri),
            self_uuri.uentity_major_version(),
            0, // set to 0 as this is the resource_id of "intended for me"
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                format!("Unable to build source UUri for MT_REQUEST type: {e:?}"),
            )
        })?;

        // TODO: Not sure where to get this
        let ttl = 1000;

        trace!("Prior to building Request");

        let umsg_res = UMessageBuilder::request(sink, source, ttl)
            .build_with_payload_encoding(payload_bytes, assumed_payload_encoding.clone());

        trace!("After building Request");

        let Ok(umsg) = umsg_res else {
            return Err(UStatus::fail_with_code(
                UCode::Internal,
                format!(
                    "Unable to build UMessage from vsomeip message: {:?}",
                    umsg_res.err().unwrap()
                ),
            ));
        };

        let req_id = umsg.id();
        trace!("{} - (req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                req_id.to_hyphenated_string(), request_id
            );

        rpc_correlation_registry.insert_me_request_correlation(req_id.clone(), request_id)?;

        Ok(umsg)
    }

    async fn convert_vsomeip_mt_response_to_umsg(
        mechatronics_authority_name: &AuthorityName,
        rpc_correlation_registry: &Arc<dyn RpcCorrelationRegistry>,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
        payload_bytes: Vec<u8>,
        assumed_payload_encoding: &PayloadEncoding,
    ) -> Result<UMessage, UStatus> {
        let request_id = vsomeip_message.get_message_base_pinned().get_request();
        let service_id = vsomeip_message.get_message_base_pinned().get_service();
        let method_id = vsomeip_message.get_message_base_pinned().get_method();
        let interface_version = vsomeip_message
            .get_message_base_pinned()
            .get_interface_version();

        trace!("MT_RESPONSE type");

        let source = UUri::try_from_parts(
            mechatronics_authority_name,
            create_ue_id_from_instance_service(
                vsomeip_message.get_message_base_pinned().get_instance(),
                service_id,
            ),
            interface_version,
            method_id,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                format!("Unable to build source UUri for MT_RESPONSE type: {e:?}"),
            )
        })?;

        trace!(
            "{} - request_id to look up to correlate to req_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
            request_id
        );
        let (req_id, sink) = rpc_correlation_registry.remove_ue_request_correlation(request_id)?;

        trace!("source: {source:?}; sink: {sink:?}");

        let umsg_res = UMessageBuilder::response(sink, req_id, source)
            .with_comm_status(UCode::Ok)
            .build_with_payload_encoding(payload_bytes, assumed_payload_encoding.clone());

        let Ok(umsg) = umsg_res else {
            return Err(UStatus::fail_with_code(
                UCode::Internal,
                format!(
                    "Unable to build UMessage from vsomeip message: {:?}",
                    umsg_res.err().unwrap()
                ),
            ));
        };

        Ok(umsg)
    }

    async fn convert_vsomeip_mt_error_to_umsg(
        mechatronics_authority_name: &AuthorityName,
        rpc_correlation_registry: &Arc<dyn RpcCorrelationRegistry>,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
        payload_bytes: Vec<u8>,
        assumed_payload_encoding: &PayloadEncoding,
    ) -> Result<UMessage, UStatus> {
        let request_id = vsomeip_message.get_message_base_pinned().get_request();
        let service_id = vsomeip_message.get_message_base_pinned().get_service();
        let method_id = vsomeip_message.get_message_base_pinned().get_method();
        let interface_version = vsomeip_message
            .get_message_base_pinned()
            .get_interface_version();

        trace!("MT_ERROR type");

        let source = UUri::try_from_parts(
            mechatronics_authority_name,
            create_ue_id_from_instance_service(
                vsomeip_message.get_message_base_pinned().get_instance(),
                service_id,
            ),
            interface_version,
            method_id,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                format!("Unable to build source UUri for MT_ERROR type: {e:?}"),
            )
        })?;

        trace!(
            "{} - request_id to look up to correlate to req_id: {}",
            UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
            request_id
        );
        let (req_id, sink) = rpc_correlation_registry.remove_ue_request_correlation(request_id)?;
        let comm_status = Self::vsomeip_err_code_to_ucode(
            vsomeip_message.get_message_base_pinned().get_return_code(),
        );

        trace!("source: {source:?}; sink: {sink:?}");

        let umsg_res = UMessageBuilder::response(sink, req_id, source)
            .with_comm_status(comm_status)
            .build_with_payload_encoding(payload_bytes, assumed_payload_encoding.clone());

        let Ok(umsg) = umsg_res else {
            return Err(UStatus::fail_with_code(
                UCode::Internal,
                format!(
                    "Unable to build UMessage from vsomeip message: {:?}",
                    umsg_res.err().unwrap()
                ),
            ));
        };

        Ok(umsg)
    }

    async fn convert_vsomeip_mt_notification_to_umsg(
        mechatronics_authority_name: &AuthorityName,
        assumed_payload_encoding: &PayloadEncoding,
        vsomeip_message: &mut UniquePtr<MessageWrapper>,
        payload_bytes: Vec<u8>,
    ) -> Result<UMessage, UStatus> {
        let service_id = (*vsomeip_message).get_message_base_pinned().get_service();
        let method_id = vsomeip_message.get_message_base_pinned().get_method();

        trace!("MT_NOTIFICATION type");

        // TODO: Talk with @StevenHartley. It seems like vsomeip notify doesn't let us set the
        //  interface_version... going to set this manually to 1 for now
        let interface_version = 1;
        let source = UUri::try_from_parts(
            mechatronics_authority_name, // TODO: Should we set this to anything specific?
            create_ue_id_from_instance_service(
                vsomeip_message.get_message_base_pinned().get_instance(),
                service_id,
            ),
            interface_version,
            method_id,
        )
        .map_err(|e| {
            UStatus::fail_with_code(
                UCode::InvalidArgument,
                format!("Unable to build source UUri for MT_NOTIFICATION type: {e:?}"),
            )
        })?;

        let umsg_res = UMessageBuilder::publish(source)
            .build_with_payload_encoding(payload_bytes, assumed_payload_encoding.clone());

        let Ok(umsg) = umsg_res else {
            return Err(UStatus::fail_with_code(
                UCode::Internal,
                format!(
                    "Unable to build UMessage from vsomeip message: {:?}",
                    umsg_res.err().unwrap()
                ),
            ));
        };

        Ok(umsg)
    }

    fn vsomeip_err_code_to_ucode(someip_error: vsomeip::return_code_e) -> UCode {
        match someip_error {
            vsomeip::return_code_e::E_OK => UCode::Ok,
            vsomeip::return_code_e::E_WRONG_MESSAGE_TYPE
            | vsomeip::return_code_e::E_UNKNOWN_METHOD => UCode::InvalidArgument,
            vsomeip::return_code_e::E_TIMEOUT => UCode::DeadlineExceeded,
            vsomeip::return_code_e::E_UNKNOWN_SERVICE => UCode::NotFound,
            vsomeip::return_code_e::E_NOT_READY => UCode::Unavailable,
            vsomeip::return_code_e::E_MALFORMED_MESSAGE => UCode::DataLoss,
            vsomeip::return_code_e::E_NOT_REACHABLE => UCode::Internal,
            vsomeip::return_code_e::E_NOT_OK => UCode::Unknown,
            vsomeip::return_code_e::E_WRONG_PROTOCOL_VERSION
            | vsomeip::return_code_e::E_WRONG_INTERFACE_VERSION => UCode::FailedPrecondition,
            _ => UCode::Unknown,
        }
    }
}

// TODO: Add unit tests
