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

use crate::transport::CLIENT_ID_SESSION_ID_TRACKING;
use crate::{ClientId, RegistrationType, RequestId, SessionId, ME_AUTHORITY};
use cxx::UniquePtr;
use log::trace;
use up_rust::{UStatus, UUri};
use vsomeip_sys::glue::MessageWrapper;
use vsomeip_sys::safe_glue::get_pinned_message_base;
use vsomeip_sys::vsomeip::message_type_e;

pub fn split_u32_to_u16(value: u32) -> (u16, u16) {
    let most_significant_bits = (value >> 16) as u16;
    let least_significant_bits = (value & 0xFFFF) as u16;
    (most_significant_bits, least_significant_bits)
}

pub fn split_u32_to_u8(value: u32) -> (u8, u8, u8, u8) {
    let byte1 = (value >> 24) as u8;
    let byte2 = (value >> 16 & 0xFF) as u8;
    let byte3 = (value >> 8 & 0xFF) as u8;
    let byte4 = (value & 0xFF) as u8;
    (byte1, byte2, byte3, byte4)
}

pub fn retrieve_session_id(client_id: ClientId) -> SessionId {
    let mut client_id_session_id_tracking = CLIENT_ID_SESSION_ID_TRACKING.lock().unwrap();

    trace!("retrieve_session_id: client_id: {}", client_id);
    let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(1);
    trace!(
        "retrieve_session_id: current_session_id: {}",
        current_sesion_id
    );
    let returned_session_id = *current_sesion_id;
    trace!(
        "retrieve_session_id: returned_session_id: {}",
        returned_session_id
    );
    *current_sesion_id += 1;
    trace!(
        "retrieve_session_id: newly updated current_session_id: {}",
        current_sesion_id
    );
    returned_session_id
}

pub fn create_request_id(client_id: ClientId, session_id: SessionId) -> RequestId {
    ((client_id as u32) << 16) | (session_id as u32)
}

// infer the type of message desired based on the filters provided
pub fn determine_registration_type(
    source_filter: &UUri,
    sink_filter: &Option<UUri>,
) -> Result<RegistrationType, UStatus> {
    if let Some(sink_filter) = &sink_filter {
        // determine if we're in the uStreamer use-case of capturing all point-to-point messages
        let streamer_use_case = {
            source_filter.authority_name != "*" // TODO: Is this good enough? Maybe have configurable in UPClientVsomeip?
                && source_filter.ue_id == 0x0000_FFFF
                && source_filter.ue_version_major == 0xFF
                && source_filter.resource_id == 0xFFFF
                && sink_filter.authority_name == "*"
                && sink_filter.ue_id == 0x0000_FFFF
                && sink_filter.ue_version_major == 0xFF
                && sink_filter.resource_id == 0xFFFF
        };

        if streamer_use_case {
            return Ok(RegistrationType::AllPointToPoint(0xFFFF));
        }

        if sink_filter.resource_id == 0 {
            Ok(RegistrationType::Response(source_filter.ue_id as ClientId))
        } else {
            Ok(RegistrationType::Request(sink_filter.ue_id as ClientId))
        }
    } else {
        // TODO: Have to consider how to handle the publish case, I suppose this is not ClientId,
        //  but instead ServiceId
        //  In any case, it should probably have its own application spun up
        Ok(RegistrationType::Publish(source_filter.ue_id as ClientId))
    }
}

pub fn is_point_to_point_message(_vsomeip_message: &mut UniquePtr<MessageWrapper>) -> bool {
    let msg_type = get_pinned_message_base(_vsomeip_message).get_message_type();
    matches!(
        msg_type,
        message_type_e::MT_REQUEST | message_type_e::MT_RESPONSE | message_type_e::MT_ERROR
    )
}
