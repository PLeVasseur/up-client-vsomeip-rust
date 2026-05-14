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

use crate::{SomeIpRequestId, UProtocolReqId};
use log::trace;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::RwLock;
use up_rust::{UCode, UStatus};

type MeRequestCorrelation = HashMap<UProtocolReqId, SomeIpRequestId>;

/// Request, Response correlation and associated functions
pub trait RpcCorrelationRegistry: Send + Sync {
    /// Insert a uE [UProtocolReqId] and mE [SomeIpRequestId] for later correlation
    fn insert_me_request_correlation(
        &self,
        uprotocol_req_id: UProtocolReqId,
        someip_request_id: SomeIpRequestId,
    ) -> Result<(), UStatus>;

    /// Remove an mE [SomeIpRequestId] based on a uE [UProtocolReqId] for correlation
    fn remove_me_request_correlation(
        &self,
        uprotocol_req_id: &UProtocolReqId,
    ) -> Result<SomeIpRequestId, UStatus>;
}

/// Request, Response correlation and associated functions
pub struct InMemoryRpcCorrelationRegistry {
    me_request_correlation: RwLock<MeRequestCorrelation>,
}

impl InMemoryRpcCorrelationRegistry {
    /// Create a new [InMemoryRpcCorrelationRegistry]
    pub fn new() -> Self {
        Self {
            me_request_correlation: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a uE [UProtocolReqId] and mE [SomeIpRequestId] for later correlation
    pub fn insert_me_request_correlation(
        &self,
        uprotocol_req_id: UProtocolReqId,
        someip_request_id: SomeIpRequestId,
    ) -> Result<(), UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().unwrap();
        match me_request_correlation.entry(uprotocol_req_id.clone()) {
            Entry::Occupied(occ) => Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "ME_REQUEST_CORRELATION: Already exists therefore rejecting, occupied: {occ:?}"
                ),
            )),
            Entry::Vacant(vac) => {
                trace!("(req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                    uprotocol_req_id.to_hyphenated_string(), someip_request_id
                );
                vac.insert(someip_request_id);
                Ok(())
            }
        }
    }

    /// Remove an mE [SomeIpRequestId] based on a uE [UProtocolReqId] for correlation
    pub fn remove_me_request_correlation(
        &self,
        uprotocol_req_id: &UProtocolReqId,
    ) -> Result<SomeIpRequestId, UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().unwrap();

        let Some(someip_request_id) = me_request_correlation.remove(uprotocol_req_id) else {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!(
                    "Corresponding SOME/IP Request ID not found for this native request_id: {}",
                    uprotocol_req_id.to_hyphenated_string()
                ),
            ));
        };

        Ok(someip_request_id)
    }
}

// TODO: Add unit tests
