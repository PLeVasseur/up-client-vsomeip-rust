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

use crate::TimedRwLock;
use crate::{EventId, InstanceId, MethodId, ServiceId};
use std::collections::HashSet;

type OfferedServices = HashSet<(ServiceId, InstanceId, MethodId)>;
type RequestedServices = HashSet<(ServiceId, InstanceId, MethodId)>;
type OfferedEvents = HashSet<(ServiceId, InstanceId, EventId)>;
type RequestedEvents = HashSet<(ServiceId, InstanceId, MethodId)>;

pub(crate) struct VsomeipOfferedRequested2 {
    offered_services: TimedRwLock<OfferedServices>,
    requested_services: TimedRwLock<RequestedServices>,
    offered_events: TimedRwLock<OfferedEvents>,
    requested_events: TimedRwLock<RequestedEvents>,
}

impl VsomeipOfferedRequested2 {
    pub fn new() -> Self {
        Self {
            offered_services: TimedRwLock::new(HashSet::new()),
            requested_services: TimedRwLock::new(HashSet::new()),
            offered_events: TimedRwLock::new(HashSet::new()),
            requested_events: TimedRwLock::new(HashSet::new()),
        }
    }

    pub(crate) async fn is_service_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.offered_services
            .read()
            .await
            .contains(&(service_id, instance_id, method_id))
    }

    pub(crate) async fn insert_service_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.offered_services
            .write()
            .await
            .insert((service_id, instance_id, method_id))
    }

    pub(crate) async fn is_service_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.requested_services
            .read()
            .await
            .contains(&(service_id, instance_id, method_id))
    }

    pub(crate) async fn insert_service_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        method_id: MethodId,
    ) -> bool {
        self.requested_services
            .write()
            .await
            .insert((service_id, instance_id, method_id))
    }

    pub(crate) async fn is_event_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.offered_events
            .read()
            .await
            .contains(&(service_id, instance_id, event_id))
    }

    pub(crate) async fn insert_event_offered(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.offered_events
            .write()
            .await
            .insert((service_id, instance_id, event_id))
    }

    pub(crate) async fn is_event_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.requested_events
            .read()
            .await
            .contains(&(service_id, instance_id, event_id))
    }

    pub(crate) async fn insert_event_requested(
        &self,
        service_id: ServiceId,
        instance_id: InstanceId,
        event_id: EventId,
    ) -> bool {
        self.requested_events
            .write()
            .await
            .insert((service_id, instance_id, event_id))
    }
}

// TODO: Add unit tests
