/********************************************************************************
 * Copyright (c) 2026 Contributors to the Eclipse Foundation
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

use crossbeam_channel::{Receiver, Sender};
use lazy_static::lazy_static;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Once, RwLock};
use up_rust::{UCode, UStatus};
use vsomeip_proc_macro::generate_subscription_handler_extern_c_fns;
use vsomeip_sys::glue::SubscriptionHandlerFnPtr;
use vsomeip_sys::vsomeip;

generate_subscription_handler_extern_c_fns!(1000);

pub trait SubscriptionHandlerRegistry: Send + Sync {
    fn allocate_subscription_handler(
        &self,
    ) -> Result<(usize, SubscriptionHandlerFnPtr, Receiver<()>), UStatus>;

    fn free_subscription_handler(&self, handler_id: usize);
}

pub struct InMemorySubscriptionHandlerRegistry;

impl InMemorySubscriptionHandlerRegistry {
    pub fn new() -> Arc<Self> {
        static INIT: Once = Once::new();
        INIT.call_once(subscription_handler_proc_macro::initialize_channels);
        Arc::new(Self)
    }
}

impl SubscriptionHandlerRegistry for InMemorySubscriptionHandlerRegistry {
    fn allocate_subscription_handler(
        &self,
    ) -> Result<(usize, SubscriptionHandlerFnPtr, Receiver<()>), UStatus> {
        let handler_id = {
            let mut free_ids = subscription_handler_proc_macro::FREE_SUBSCRIPTION_HANDLER_IDS
                .write()
                .unwrap();
            let handler_id = free_ids.iter().next().copied().ok_or_else(|| {
                UStatus::fail_with_code(
                    UCode::ResourceExhausted,
                    "No subscription readiness handlers available",
                )
            })?;
            free_ids.remove(&handler_id);
            handler_id
        };
        let (extern_fn, receiver) = subscription_handler_proc_macro::get_extern_fn(handler_id);
        Ok((handler_id, SubscriptionHandlerFnPtr(extern_fn), receiver))
    }

    fn free_subscription_handler(&self, handler_id: usize) {
        subscription_handler_proc_macro::FREE_SUBSCRIPTION_HANDLER_IDS
            .write()
            .unwrap()
            .insert(handler_id);
    }
}
