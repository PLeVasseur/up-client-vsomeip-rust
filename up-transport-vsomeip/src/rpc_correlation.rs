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

use crate::{ClientId, ReqId, RequestId, SessionId};
use lazy_static::lazy_static;
use log::trace;
use std::collections::HashMap;
use tokio::sync::RwLock as TokioRwLock;
use up_rust::{UCode, UStatus};

pub(crate) struct RpcCorrelation2 {
    ue_request_correlation: TokioRwLock<HashMap<RequestId, ReqId>>,
    me_request_correlation: TokioRwLock<HashMap<ReqId, RequestId>>,
    client_id_session_id_tracking: TokioRwLock<HashMap<ClientId, SessionId>>,
}

impl RpcCorrelation2 {
    pub fn new() -> Self {
        Self {
            ue_request_correlation: TokioRwLock::new(HashMap::new()),
            me_request_correlation: TokioRwLock::new(HashMap::new()),
            client_id_session_id_tracking: TokioRwLock::new(HashMap::new()),
        }
    }

    pub(crate) async fn retrieve_session_id(&self, client_id: ClientId) -> SessionId {
        let mut client_id_session_id_tracking = self.client_id_session_id_tracking.write().await;

        let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(1);
        let returned_session_id = *current_sesion_id;
        *current_sesion_id += 1;
        returned_session_id
    }

    pub(crate) async fn insert_ue_request_correlation(
        &self,
        app_request_id: RequestId,
        req_id: &ReqId,
    ) -> Result<(), UStatus> {
        let mut ue_request_correlation = self.ue_request_correlation.write().await;

        if ue_request_correlation.get(&app_request_id).is_none() {
            ue_request_correlation.insert(app_request_id, req_id.clone());
            trace!("(app_request_id, req_id)  inserted for later correlation in UE_REQUEST_CORRELATION: ({}, {})",
                    app_request_id, req_id.to_hyphenated_string(),
                );
            Ok(())
        } else {
            Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!(
                    "Already exists same request with id: {app_request_id}, therefore rejecting"
                ),
            ))
        }
    }

    pub(crate) async fn remove_ue_request_correlation(
        &self,
        request_id: RequestId,
    ) -> Result<ReqId, UStatus> {
        let mut ue_request_correlation = self.ue_request_correlation.write().await;

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

    pub(crate) async fn insert_me_request_correlation(
        &self,
        req_id: ReqId,
        request_id: RequestId,
    ) -> Result<(), UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().await;

        if me_request_correlation.get(&req_id).is_none() {
            trace!("(req_id, request_id) to store for later correlation in ME_REQUEST_CORRELATION: ({}, {})",
                    req_id.to_hyphenated_string(), request_id
                );
            me_request_correlation.insert(req_id.clone(), request_id);
            Ok(())
        } else {
            Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!("Already exists same MT_REQUEST with id: {req_id}, therefore rejecting"),
            ))
        }
    }

    pub(crate) async fn remove_me_request_correlation(
        &self,
        req_id: &ReqId,
    ) -> Result<RequestId, UStatus> {
        let mut me_request_correlation = self.me_request_correlation.write().await;

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
