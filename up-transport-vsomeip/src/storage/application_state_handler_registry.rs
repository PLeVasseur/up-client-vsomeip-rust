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

use crate::utils::TimedStdRwLock;
use async_trait::async_trait;
use crossbeam_channel::{Receiver, Sender};
use lazy_static::lazy_static;
use log::{error, trace};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Once};
use up_rust::{UCode, UStatus};
use vsomeip_proc_macro::generate_available_state_handler_extern_c_fns;
use vsomeip_sys::glue::AvailableStateHandlerFnPtr;
use vsomeip_sys::vsomeip;

generate_available_state_handler_extern_c_fns!(1000);

#[async_trait]
pub trait MockableApplicationAvailableRegistry: Send + Sync {
    fn get_state_handler(
        &self,
        state_handler_id: usize,
    ) -> (AvailableStateHandlerFnPtr, Receiver<vsomeip::state_type_e>);
    fn free_state_handler_id(&self, state_handler_id: usize) -> Result<(), UStatus>;
    fn find_available_state_handler_id(&self) -> Result<usize, UStatus>;
}

pub struct ApplicationStateAvailabilityRegistry;

impl ApplicationStateAvailabilityRegistry {
    pub fn new_trait_obj() -> Arc<dyn MockableApplicationAvailableRegistry> {
        static INIT: Once = Once::new();

        INIT.call_once(|| {
            available_state_handler_proc_macro::initialize_sender_receiver();
        });

        Arc::new(ApplicationStateAvailabilityRegistry)
    }
}

impl MockableApplicationAvailableRegistry for ApplicationStateAvailabilityRegistry {
    fn get_state_handler(
        &self,
        state_handler_id: usize,
    ) -> (AvailableStateHandlerFnPtr, Receiver<vsomeip::state_type_e>) {
        let (extern_fn, receiver) =
            available_state_handler_proc_macro::get_extern_fn(state_handler_id);
        (AvailableStateHandlerFnPtr(extern_fn), receiver)
    }

    fn free_state_handler_id(&self, state_handler_id: usize) -> Result<(), UStatus> {
        let mut free_state_handler_ids =
            available_state_handler_proc_macro::FREE_AVAILABLE_STATE_HANDLER_EXTERN_FN_IDS.write();
        free_state_handler_ids.insert(state_handler_id);

        trace!("free_state_handler_id: {state_handler_id}");

        Ok(())
    }

    fn find_available_state_handler_id(&self) -> Result<usize, UStatus> {
        let mut free_state_handler_ids =
            available_state_handler_proc_macro::FREE_AVAILABLE_STATE_HANDLER_EXTERN_FN_IDS.write();
        if let Some(&id) = free_state_handler_ids.iter().next() {
            free_state_handler_ids.remove(&id);
            trace!("find_available_listener_id: {id}");
            Ok(id)
        } else {
            Err(UStatus::fail_with_code(
                UCode::RESOURCE_EXHAUSTED,
                "No more extern C fns available",
            ))
        }
    }
}
