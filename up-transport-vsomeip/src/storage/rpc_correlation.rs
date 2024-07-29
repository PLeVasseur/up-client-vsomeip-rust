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

use crate::{ClientId, SessionId, SomeIpRequestId, UProtocolReqId};
use log::trace;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::RwLock;
use up_rust::{UCode, UStatus};

// TODO: Should attach the received Request in full so that when we're shutting down
//  the transport we can emit messages back to clients noting the error
type UeRequestCorrelation = HashMap<SomeIpRequestId, UProtocolReqId>;
type MeRequestCorrelation = HashMap<UProtocolReqId, SomeIpRequestId>;
type ClientIdSessionIdTracking = HashMap<ClientId, SessionId>;

/// Request, Response correlation and associated functions
pub struct RpcCorrelation {
    ue_request_correlation: RwLock<UeRequestCorrelation>,
    me_request_correlation: RwLock<MeRequestCorrelation>,
    client_id_session_id_tracking: RwLock<ClientIdSessionIdTracking>,
}

impl RpcCorrelation {
    /// Create a new [RpcCorrelation]
    pub fn new() -> Self {
        Self {
            ue_request_correlation: RwLock::new(HashMap::new()),
            me_request_correlation: RwLock::new(HashMap::new()),
            client_id_session_id_tracking: RwLock::new(HashMap::new()),
        }
    }

    /// Get a current [SessionId] based on a [ClientId]
    pub fn retrieve_session_id(&self, client_id: ClientId) -> SessionId {
        let mut client_id_session_id_tracking = self.client_id_session_id_tracking.write().unwrap();

        let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(1);
        let returned_session_id = *current_sesion_id;
        *current_sesion_id += 1;
        returned_session_id
    }

    /// Insert an mE [SomeIpRequestId] and uE [UProtocolReqId] for later correlation
    pub fn insert_ue_request_correlation(
        &self,
        app_request_id: SomeIpRequestId,
        req_id: &UProtocolReqId,
    ) -> Result<(), UStatus> {
        let mut ue_request_correlation = self.ue_request_correlation.write().unwrap();
        match ue_request_correlation.entry(app_request_id) {
            Entry::Occupied(occ) => Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "UE_REQUEST_CORRELATION: Already exists therefore rejecting, occupied: {occ:?}"
                ),
            )),
            Entry::Vacant(vac) => {
                trace!("(app_request_id, req_id)  inserted for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                    app_request_id, req_id.to_hyphenated_string(),
                );
                vac.insert(req_id.clone());
                Ok(())
            }
        }
    }

    /// Remove a uE [UProtocolReqId] based on an mE [SomeIpRequestId] for correlation
    pub fn remove_ue_request_correlation(
        &self,
        request_id: SomeIpRequestId,
    ) -> Result<UProtocolReqId, UStatus> {
        let mut ue_request_correlation = self.ue_request_correlation.write().unwrap();

        let Some(req_id) = ue_request_correlation.remove(&request_id) else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "Corresponding reqid not found for this SOME/IP RESPONSE: {}",
                    request_id
                ),
            ));
        };

        Ok(req_id)
    }

    /// Insert a uE [UProtocolReqId] and mE [SomeIpRequestId] for later correlation
    pub fn insert_me_request_correlation(
        &self,
        req_id: UProtocolReqId,
        request_id: SomeIpRequestId,
    ) -> Result<(), UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().unwrap();
        match me_request_correlation.entry(req_id.clone()) {
            Entry::Occupied(occ) => Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "ME_REQUEST_CORRELATION: Already exists therefore rejecting, occupied: {occ:?}"
                ),
            )),
            Entry::Vacant(vac) => {
                trace!("(req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                    req_id.to_hyphenated_string(), request_id
                );
                vac.insert(request_id);
                Ok(())
            }
        }
    }

    /// Remove an mE [SomeIpRequestId] based on a uE [UProtocolReqId] for correlation
    pub fn remove_me_request_correlation(
        &self,
        req_id: &UProtocolReqId,
    ) -> Result<SomeIpRequestId, UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().unwrap();

        let Some(request_id) = me_request_correlation.remove(req_id) else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "Corresponding SOME/IP Request ID not found for this Request UMessage's reqid: {}",
                    req_id.to_hyphenated_string()
                ),
            ));
        };

        Ok(request_id)
    }
}

// TODO: Add unit tests
