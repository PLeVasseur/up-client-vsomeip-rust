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

use crate::{ClientId, UeId};
use log::trace;
use up_rust::{UCode, UStatus, UUri};

/// Registration type containing the [ClientId] of the [vsomeip_sys::vsomeip::application]
/// which should be used for this message
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RegistrationType {
    Publish,
    Request,
    Response,
    AllPointToPoint,
}

/// Determines [RegistrationType] of a source and sink filter [UUri]
pub fn determine_type(
    source_filter: &UUri,
    sink_filter: &Option<UUri>,
) -> Result<RegistrationType, UStatus> {
    trace!(
        "determine RegistrationType for source_filter: {:?} sink_filter: {:?}",
        source_filter,
        sink_filter
    );

    if let Some(sink_filter) = &sink_filter {
        let sink_auth_name_condition_met = sink_filter.authority_name != "*";
        let sink_ue_id_condition_met = sink_filter.ue_id == 0x0000_FFFF;
        let sink_ue_version_major_condition_met = sink_filter.ue_version_major == 0xFF;
        let sink_resource_id_condition_met = sink_filter.resource_id == 0xFFFF;

        let source_auth_name_condition_met = source_filter.authority_name == "*";
        let source_ue_id_condition_met = source_filter.ue_id == 0x0000_FFFF;
        let source_ue_version_major_condition_met = source_filter.ue_version_major == 0xFF;
        let source_resource_id_condition_met = source_filter.resource_id == 0xFFFF;

        if log::log_enabled!(log::Level::Trace) {
            trace!(
                "sink_auth_name_condition_met: {sink_auth_name_condition_met:?}
                   sink_ue_id_condition_met: {sink_ue_id_condition_met:?}
                   sink_ue_version_major_condition_met: {sink_ue_version_major_condition_met:?}
                   sink_resource_id_condition_met: {sink_resource_id_condition_met:?}
                   source_auth_name_condition_met: {source_auth_name_condition_met:?}
                   source_ue_id_condition_met: {source_ue_id_condition_met:?}
                   source_ue_version_major_condition_met: {source_ue_version_major_condition_met:?}
                   source_resource_id_condition_met: {source_resource_id_condition_met:?}"
            );
        }
        // determine if we're in the uStreamer use-case of capturing all point-to-point messages
        let streamer_use_case = {
            sink_auth_name_condition_met
                && sink_ue_id_condition_met
                && sink_ue_version_major_condition_met
                && sink_resource_id_condition_met
                && source_auth_name_condition_met
                && source_ue_id_condition_met
                && source_ue_version_major_condition_met
                && source_resource_id_condition_met
        };

        if streamer_use_case {
            return Ok(RegistrationType::AllPointToPoint);
        }

        if sink_filter.resource_id == 0 {
            Ok(RegistrationType::Response)
        } else {
            Ok(RegistrationType::Request)
        }
    } else {
        Ok(RegistrationType::Publish)
    }
}

// TODO: Add unit tests
