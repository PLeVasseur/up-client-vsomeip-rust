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

use crate::{EventId, InstanceId, MethodId, ServiceId};
use lazy_static::lazy_static;
use std::collections::HashSet;
use tokio::sync::RwLock;

pub(crate) struct VsomeipOfferedRequested2 {
    offered_services: HashSet<(ServiceId, InstanceId, MethodId)>,
    requested_services: HashSet<(ServiceId, InstanceId, MethodId)>,
    offered_events: HashSet<(ServiceId, InstanceId, EventId)>,
    requested_events: HashSet<(ServiceId, InstanceId, MethodId)>,
}

impl VsomeipOfferedRequested2 {
    pub fn new() -> Self {
        Self {
            offered_services: HashSet::new(),
            requested_services: HashSet::new(),
            offered_events: HashSet::new(),
            requested_events: HashSet::new(),
        }
    }

    pub(crate) fn is_service_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.offered_services
            .contains(&(service_id, instance_id, method_id))
    }

    pub(crate) fn insert_service_offered(
        &mut self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.offered_services
            .insert((service_id, instance_id, method_id))
    }

    pub(crate) fn is_service_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.requested_services
            .contains(&(service_id, instance_id, method_id))
    }

    pub(crate) fn insert_service_requested(
        &mut self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.requested_services
            .insert((service_id, instance_id, method_id))
    }

    pub(crate) fn is_event_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.offered_events
            .contains(&(service_id, instance_id, event_id))
    }

    pub(crate) fn insert_event_offered(
        &mut self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.offered_events
            .insert((service_id, instance_id, event_id))
    }

    pub(crate) fn is_event_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.requested_events
            .contains(&(service_id, instance_id, event_id))
    }

    pub(crate) fn insert_event_requested(
        &mut self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.requested_events
            .insert((service_id, instance_id, event_id))
    }
}

// TODO: Add unit tests
