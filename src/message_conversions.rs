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

use crate::transport::{AUTHORITY_NAME, ME_REQUEST_CORRELATION, UE_REQUEST_CORRELATION};
use crate::{
    create_request_id, retrieve_session_id, split_u32_to_u16, split_u32_to_u8, ME_AUTHORITY,
};
use cxx::UniquePtr;
use log::trace;
use protobuf::Enum;
use up_rust::{UCode, UMessage, UMessageBuilder, UMessageType, UPayloadFormat, UStatus, UUri};
use vsomeip_sys::glue::{
    make_message_wrapper, make_payload_wrapper, ApplicationWrapper, MessageWrapper, RuntimeWrapper,
};
use vsomeip_sys::safe_glue::{
    get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base,
    get_pinned_payload, get_pinned_runtime, set_data_safe, set_message_payload,
};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::message_type_e;

const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG: &str = "convert_umsg_to_vsomeip_msg";
const UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG: &str = "convert_vsomeip_msg_to_umsg";

pub fn convert_umsg_to_vsomeip_msg(
    umsg: &UMessage,
    application_wrapper: &UniquePtr<ApplicationWrapper>,
    runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UniquePtr<MessageWrapper>, UStatus> {
    let Some(source) = umsg.attributes.source.as_ref() else {
        return Err(UStatus::fail_with_code(
            UCode::INVALID_ARGUMENT,
            "Message has no source UUri",
        ));
    };

    match umsg
        .attributes
        .type_
        .enum_value_or(UMessageType::UMESSAGE_TYPE_UNSPECIFIED)
    {
        UMessageType::UMESSAGE_TYPE_PUBLISH => {
            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_notification(true));
            let (_instance_id, service_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            let (_, method_id) = split_u32_to_u16(source.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            // let client_id = 0; // manually setting this to 0 as according to spec
            // get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            // let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major);
            // get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);
            // let session_id = retrieve_session_id(client_id);
            // get_pinned_message_base(&vsomeip_msg).set_session(session_id);
            // get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);

            trace!(
                "Immediately prior to request_service: service_id: {} instance_id: {}",
                service_id,
                instance_id
            );

            get_pinned_application(application_wrapper).request_service(
                service_id,
                instance_id,
                vsomeip::ANY_MAJOR,
                // interface_version,
                vsomeip::ANY_MINOR,
            );

            // get_pinned_application(_application_wrapper).offer_service(
            //     service_id,
            //     1,
            //     interface_version,
            //     vsomeip::ANY_MINOR,
            // );

            trace!("Immediately after request_service");

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_REQUEST => {
            let Some(sink) = umsg.attributes.sink.as_ref() else {
                return Err(UStatus::fail_with_code(
                    UCode::INVALID_ARGUMENT,
                    "Message has no sink UUri",
                ));
            };

            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_request(true));
            let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id);
            trace!(
                "{} - sink.ue_id: {} source.ue_id: {} _instance_id: {} service_id:{}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                sink.ue_id,
                source.ue_id,
                _instance_id,
                service_id
            );
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            let (_, method_id) = split_u32_to_u16(sink.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.id.as_ref().unwrap();
            let app_client_id = get_pinned_application(application_wrapper).get_client();
            let app_session_id = retrieve_session_id(app_client_id); // only rewritten by vsomeip for REQUESTs
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
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            if ue_request_correlation.get(&app_request_id).is_none() {
                ue_request_correlation.insert(app_request_id, req_id.clone());
                trace!("{} - (app_request_id, req_id)  inserted for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                    app_request_id, req_id.to_hyphenated_string(),
                );
            } else {
                // TODO: What do we do if we have a duplicate, already-existing pair?
                //  Eject the previous one? Fail on this one?
            }
            // get_pinned_message_base(&vsomeip_msg).set_session(session_id); // doesn't matter at all, rewritten by send()
            get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);

            let request_id = get_pinned_message_base(&vsomeip_msg).get_request();
            let service_id = get_pinned_message_base(&vsomeip_msg).get_service();
            let client_id = get_pinned_message_base(&vsomeip_msg).get_client();
            let session_id = get_pinned_message_base(&vsomeip_msg).get_session();
            let method_id = get_pinned_message_base(&vsomeip_msg).get_method();
            let instance_id = get_pinned_message_base(&vsomeip_msg).get_instance();
            let interface_version = get_pinned_message_base(&vsomeip_msg).get_interface_version();

            // let payload = get_message_payload(&vsomeip_msg);
            // let payload_bytes = get_data_safe(&payload);

            trace!("{} - : request_id: {} client_id: {} session_id: {} service_id: {} instance_id: {} method_id: {} interface_version: {} app_client_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                request_id, client_id, session_id, service_id, instance_id, method_id, interface_version, app_client_id
            );

            Ok(vsomeip_msg)
        }
        UMessageType::UMESSAGE_TYPE_RESPONSE => {
            trace!(
                "{} - Attempting to send Response",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
            );

            // Implementation goes here
            let mut vsomeip_msg =
                make_message_wrapper(get_pinned_runtime(runtime_wrapper).create_message(true));

            // TODO: Because these must be source to allow the response to be seen, we need to give
            //  this a tune-up, since now no longer does the service_id match what's expected on
            //  the other end.
            //
            // TODO: Will need to consider what changes to make
            // let (_instance_id, service_id) = split_u32_to_u16(sink.ue_id); // Should this be source?
            let (_instance_id, service_id) = split_u32_to_u16(source.ue_id);
            get_pinned_message_base(&vsomeip_msg).set_service(service_id);
            let instance_id = 1; // TODO: Setting to 1 manually for now
            get_pinned_message_base(&vsomeip_msg).set_instance(instance_id);
            // let (_, method_id) = split_u32_to_u16(sink.resource_id); // Should this be source?
            let (_, method_id) = split_u32_to_u16(source.resource_id);
            get_pinned_message_base(&vsomeip_msg).set_method(method_id);
            // let (_, _, _, interface_version) = split_u32_to_u8(sink.ue_version_major); // Should this be source?
            let (_, _, _, interface_version) = split_u32_to_u8(source.ue_version_major);
            get_pinned_message_base(&vsomeip_msg).set_interface_version(interface_version);

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.reqid.as_ref().unwrap();
            trace!(
                "{} - Looking up req_id from UMessage in ME_REQUEST_CORRELATION, req_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                req_id.to_hyphenated_string()
            );
            let mut me_request_correlation = ME_REQUEST_CORRELATION.lock().unwrap();
            let Some(request_id) = me_request_correlation.remove(req_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!("Corresponding SOME/IP Request ID not found for this Request UMessage's reqid: {}",
                            req_id.to_hyphenated_string()),
                ));
            };
            trace!(
                "{} - Found correlated request_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                request_id
            );
            let (client_id, session_id) = split_u32_to_u16(request_id);
            get_pinned_message_base(&vsomeip_msg).set_client(client_id);
            get_pinned_message_base(&vsomeip_msg).set_session(session_id);
            trace!(
                "{} - request_id: {} client_id: {} session_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                request_id,
                client_id,
                session_id
            );
            let ok = {
                if let Some(commstatus) = umsg.attributes.commstatus {
                    let commstatus = commstatus.enum_value_or(UCode::UNIMPLEMENTED);
                    commstatus == UCode::OK
                } else {
                    false
                }
            };
            let payload = {
                if let Some(bytes) = umsg.payload.clone() {
                    bytes.to_vec()
                } else {
                    Vec::new()
                }
            };
            let mut vsomeip_payload =
                make_payload_wrapper(get_pinned_runtime(runtime_wrapper).create_payload());
            set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
            set_message_payload(&mut vsomeip_msg, &mut vsomeip_payload);
            if ok {
                get_pinned_message_base(&vsomeip_msg).set_return_code(vsomeip::return_code_e::E_OK);
                get_pinned_message_base(&vsomeip_msg).set_message_type(message_type_e::MT_RESPONSE);
            } else {
                // TODO: Perform mapping from uProtocol UCode contained in commstatus into vsomeip::return_code_e
                get_pinned_message_base(&vsomeip_msg)
                    .set_return_code(vsomeip::return_code_e::E_NOT_OK);
                get_pinned_message_base(&vsomeip_msg).set_message_type(message_type_e::MT_ERROR);
            }

            trace!(
                "{} - Response: Finished building vsomeip message: service_id: {} instance_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_UMSG_TO_VSOMEIP_MSG,
                service_id,
                instance_id
            );

            Ok(vsomeip_msg)
        }
        _ => Err(UStatus::fail_with_code(
            UCode::INTERNAL,
            "Trying to convert an unspecified or notification message type.",
        )),
    }
}

pub fn convert_vsomeip_msg_to_umsg(
    vsomeip_message: &mut UniquePtr<MessageWrapper>,
    _application_wrapper: &UniquePtr<ApplicationWrapper>,
    _runtime_wrapper: &UniquePtr<RuntimeWrapper>,
) -> Result<UMessage, UStatus> {
    trace!("top of convert_vsomeip_msg_to_umsg");
    let msg_type = get_pinned_message_base(vsomeip_message).get_message_type();

    let request_id = get_pinned_message_base(vsomeip_message).get_request();
    let service_id = get_pinned_message_base(vsomeip_message).get_service();
    let client_id = get_pinned_message_base(vsomeip_message).get_client();
    let session_id = get_pinned_message_base(vsomeip_message).get_session();
    let method_id = get_pinned_message_base(vsomeip_message).get_method();
    let instance_id = get_pinned_message_base(vsomeip_message).get_instance();
    let interface_version = get_pinned_message_base(vsomeip_message).get_interface_version();
    let payload = get_message_payload(vsomeip_message);
    let payload_bytes = get_data_safe(&payload);

    trace!("{} - : request_id: {} client_id: {} session_id: {} service_id: {} instance_id: {} method_id: {} interface_version: {}",
        UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
        request_id, client_id, session_id, service_id, instance_id, method_id, interface_version
    );

    let authority_name = { AUTHORITY_NAME.lock().unwrap().clone() };

    trace!("unloaded all relevant info from vsomeip message");

    match msg_type {
        message_type_e::MT_REQUEST => {
            trace!("MT_REQUEST type");
            let sink = UUri {
                authority_name,
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            // TODO: Not sure where to get this
            //  Steven said Ivan posted something to a Slack thread; need to check
            //  Hmm, didn't find this. Asked Steven for help
            //  He pointed me to something about SOME/IP-SD, but not Request AFAICT
            let ttl = 1000;

            trace!("Prior to building Request");

            let umsg_res = UMessageBuilder::request(sink, source, ttl)
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            trace!("After building Request");

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            // TODO: Remove .unwrap()
            let req_id = umsg.attributes.id.as_ref().unwrap();
            trace!("{} - (req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                req_id.to_hyphenated_string(), request_id
            );
            let mut me_request_correlation = ME_REQUEST_CORRELATION.lock().unwrap();
            if me_request_correlation.get(req_id).is_none() {
                trace!("{} - (req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                    UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                    req_id.to_hyphenated_string(), request_id
                );
                me_request_correlation.insert(req_id.clone(), request_id);
            } else {
                // TODO: What do we do if we have a duplicate, already-existing pair?
                //  Eject the previous one? Fail on this one?
            }

            Ok(umsg)
        }
        message_type_e::MT_NOTIFICATION => {
            trace!("MT_NOTIFICATION type");
            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            let umsg_res = UMessageBuilder::publish(source)
                .with_comm_status(UCode::OK.value())
                .build();

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        message_type_e::MT_RESPONSE => {
            trace!("MT_RESPONSE type");
            let sink = UUri {
                authority_name,
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            trace!(
                "{} - request_id to look up to correlate to req_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                request_id
            );
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            let Some(req_id) = ue_request_correlation.remove(&request_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!(
                        "Corresponding reqid not found for this SOME/IP RESPONSE: {}",
                        request_id
                    ),
                ));
            };

            let umsg_res = UMessageBuilder::response(sink, req_id, source)
                .with_comm_status(UCode::OK.value())
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        message_type_e::MT_ERROR => {
            trace!("MT_ERROR type");
            let sink = UUri {
                authority_name,
                ue_id: client_id as u32,
                ue_version_major: 1, // TODO: I don't see a way to get this
                resource_id: 0,      // set to 0 as this is the resource_id of "me"
                ..Default::default()
            };

            let source = UUri {
                authority_name: ME_AUTHORITY.to_string(), // TODO: Should we set this to anything specific?
                ue_id: service_id as u32,
                ue_version_major: interface_version as u32,
                resource_id: method_id as u32,
                ..Default::default()
            };

            // TODO: Need to update to use RequestId instead of ClientId
            trace!(
                "{} - request_id to look up to correlate to req_id: {}",
                UP_CLIENT_VSOMEIP_FN_TAG_CONVERT_VSOMEIP_MSG_TO_UMSG,
                request_id
            );
            let mut ue_request_correlation = UE_REQUEST_CORRELATION.lock().unwrap();
            let Some(req_id) = ue_request_correlation.remove(&request_id) else {
                return Err(UStatus::fail_with_code(
                    UCode::NOT_FOUND,
                    format!(
                        "Corresponding reqid not found for this SOME/IP RESPONSE: {}",
                        request_id
                    ),
                ));
            };

            let umsg_res = UMessageBuilder::response(sink, req_id, source)
                .with_comm_status(UCode::INTERNAL.value())
                .build_with_payload(payload_bytes, UPayloadFormat::UPAYLOAD_FORMAT_UNSPECIFIED);

            let Ok(umsg) = umsg_res else {
                return Err(UStatus::fail_with_code(
                    UCode::INTERNAL,
                    "Unable to build UMessage from vsomeip message",
                ));
            };

            Ok(umsg)
        }
        _ => Err(UStatus::fail_with_code(
            UCode::OUT_OF_RANGE,
            format!(
                "Not one of the handled message types from SOME/IP: {:?}",
                msg_type
            ),
        )),
    }
}
