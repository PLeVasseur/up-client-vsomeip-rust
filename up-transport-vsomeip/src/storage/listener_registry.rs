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
use crate::ListenerId;
use crate::{ApplicationName, ClientId};
use bimap::BiMap;
use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use up_rust::{ComparableListener, UCode, UListener, UStatus, UUri};

type ListenerIdAndListenerConfig = BiMap<ListenerId, (UUri, Option<UUri>, ComparableListener)>;
type ListenerIdToClientId = HashMap<ListenerId, ClientId>;
type ClientIdToListenerId = HashMap<ClientId, HashSet<ListenerId>>;
type ClientAndAppName = BiMap<ClientId, ApplicationName>;
/// A listener registry to maintain state related to [ListenerId], active [vsomeip_sys::vsomeip::application]s,
/// and the [ComparableListener]s associated with [ListenerId]s
pub struct ListenerRegistry {
    listener_id_and_listener_config: TimedStdRwLock<ListenerIdAndListenerConfig>,
    listener_id_to_client_id: TimedStdRwLock<ListenerIdToClientId>,
    client_id_to_listener_id: TimedStdRwLock<ClientIdToListenerId>,
    client_and_app_name: TimedStdRwLock<ClientAndAppName>,
}

impl ListenerRegistry {
    /// Create a new [ListenerRegistry]
    pub fn new() -> Self {
        Self {
            listener_id_and_listener_config: TimedStdRwLock::new(BiMap::new()),
            listener_id_to_client_id: TimedStdRwLock::new(HashMap::new()),
            client_id_to_listener_id: TimedStdRwLock::new(HashMap::new()),
            client_and_app_name: TimedStdRwLock::new(BiMap::new()),
        }
    }

    /// Insert [ListenerId] and [ClientId]
    pub fn insert_listener_id_client_id(
        &self,
        listener_id: ListenerId,
        client_id: ClientId,
    ) -> Option<(usize, ClientId)> {
        let mut listener_id_to_client_id = self.listener_id_to_client_id.write();
        let mut client_id_to_listener_id = self.client_id_to_listener_id.write();

        debug!("before listener_id_to_client_id: {listener_id_to_client_id:?}");

        if listener_id_to_client_id.contains_key(&listener_id) {
            info!("We already used listener_id: {listener_id}");
            debug!("listener_id_to_client_id: {listener_id_to_client_id:?}");

            let client_id = listener_id_to_client_id.get(&listener_id).unwrap();
            return Some((listener_id, *client_id));
        }

        let insert_res = listener_id_to_client_id.insert(listener_id, client_id);
        if let Some(client_id) = insert_res {
            info!("We already inserted listener_id: {listener_id} with client_id: {client_id}");

            return Some((listener_id, client_id));
        }

        let listener_ids = client_id_to_listener_id.entry(client_id).or_default();
        let newly_added = listener_ids.insert(listener_id);
        if !newly_added {
            info!("Attempted to inserted already existing listener_id: {listener_id} into client_id: {client_id}");

            return Some((listener_id, client_id));
        }

        info!("Newly added listener_id: {listener_id} client_id: {client_id}");
        debug!("after listener_id_to_client_id: {listener_id_to_client_id:?}");

        None
    }

    /// Remove [ListenerId] and listener configuration
    pub fn remove_listener_id_and_listener_config_based_on_listener_id(
        &self,
        listener_id: ListenerId,
    ) -> Result<(), UStatus> {
        let removed = self
            .listener_id_and_listener_config
            .write()
            .remove_by_left(&listener_id);
        if removed.is_none() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("No listener_config for listener_id: {listener_id}"),
            ));
        }

        Ok(())
    }

    /// Remove [ClientId] based on [ListenerId]
    pub fn remove_client_id_based_on_listener_id(&self, listener_id: usize) -> Option<ClientId> {
        let mut listener_id_to_client_id = self.listener_id_to_client_id.write();
        let mut client_id_to_listener_id = self.client_id_to_listener_id.write();

        let removed = listener_id_to_client_id.remove(&listener_id);
        if let Some(client_id) = removed {
            client_id_to_listener_id
                .entry(client_id)
                .or_default()
                .remove(&listener_id);

            return Some(client_id);
        }

        None
    }

    /// Insert [ListenerId] and listener configuration
    pub fn insert_listener_id_and_listener_config(
        &self,
        listener_id: usize,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<(), UStatus> {
        let mut listener_id_and_listener_config = self.listener_id_and_listener_config.write();
        let insertion_res =
            listener_id_and_listener_config.insert_no_overwrite(listener_id, listener_config);

        if let Err(existed) = insertion_res {
            let (listener_id, (src, sink, _comp_listener)) = existed;

            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!("Unable to set_listener_id_and_listener_config, one side or the other already exists: (listener_id: {}, (src: {:?}, sink: {:?}, comp_listener: {:?}))", listener_id, src, sink, ""),
            ));
        }

        Ok(())
    }

    /// Insert a [ClientId] and [ApplicationName]
    pub fn insert_client_and_app_name(
        &self,
        client_id: ClientId,
        app_name: ApplicationName,
    ) -> Result<(), UStatus> {
        let mut client_and_app_name = self.client_and_app_name.write();
        if let Err(existing) = client_and_app_name.insert_no_overwrite(client_id, app_name.clone())
        {
            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!("Already exists that pair of client_id and app_name: {existing:?}"),
            ));
        }

        Ok(())
    }

    /// Remove [ApplicationName] based on [ClientId]
    pub fn remove_app_name_for_client_id(&self, client_id: ClientId) -> Option<ApplicationName> {
        let mut client_and_app_name = self.client_and_app_name.write();

        let (_client_id, app_name) = client_and_app_name.remove_by_left(&client_id)?;

        Some(app_name.clone())
    }

    /// Find count of listeners registered to [ClientId]
    pub fn listener_count_for_client_id(&self, client_id: ClientId) -> usize {
        let client_id_to_listener_id = self.client_id_to_listener_id.read();

        let listener_ids = client_id_to_listener_id.get(&client_id);
        if let Some(listener_ids) = listener_ids {
            return listener_ids.len();
        }

        0
    }

    /// Get [ApplicationName] for a [ClientId]
    pub fn get_app_name_for_client_id(&self, client_id: ClientId) -> Option<ApplicationName> {
        let client_and_app_name = self.client_and_app_name.read();

        let app_name = client_and_app_name.get_by_left(&client_id)?;

        Some(app_name.clone())
    }

    /// Get [ListenerId] based on a listener configuration
    pub fn get_listener_id_for_listener_config(
        &self,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Option<usize> {
        let listener_id_and_listener_config = self.listener_id_and_listener_config.read();

        let listener_id = listener_id_and_listener_config.get_by_right(&listener_config)?;

        Some(*listener_id)
    }

    /// Get trait object [UListener] for a [ListenerId]
    pub fn get_listener_for_listener_id(&self, listener_id: usize) -> Option<Arc<dyn UListener>> {
        let listener_id_and_listener_config = self.listener_id_and_listener_config.read();

        let (_, _, comp_listener) = listener_id_and_listener_config.get_by_left(&listener_id)?;

        Some(comp_listener.into_inner())
    }

    /// Get all [ListenerId]s
    pub fn get_listener_ids(&self) -> Vec<usize> {
        let listener_id_to_client_id = self.listener_id_and_listener_config.read();

        listener_id_to_client_id.left_values().copied().collect()
    }

    /// Prints lock wait times
    pub async fn print_rwlock_times(&self) {
        #[cfg(feature = "timing")]
        {
            println!("listener_id_and_listener_config:");
            println!(
                "reads: {:?}",
                self.listener_id_and_listener_config.read_durations()
            );
            println!(
                "writes: {:?}",
                self.listener_id_and_listener_config.write_durations()
            );

            println!("listener_id_to_client_id:");
            println!(
                "reads: {:?}",
                self.listener_id_to_client_id.read_durations()
            );
            println!(
                "writes: {:?}",
                self.listener_id_to_client_id.write_durations()
            );

            println!("client_id_to_listener_id:");
            println!(
                "reads: {:?}",
                self.client_id_to_listener_id.read_durations()
            );
            println!(
                "writes: {:?}",
                self.client_id_to_listener_id.write_durations()
            );

            println!("client_and_app_name:");
            println!("reads: {:?}", self.client_and_app_name.read_durations());
            println!("writes: {:?}", self.client_and_app_name.write_durations());
        }
    }
}
