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
use crate::{ApplicationName, ClientId};
use bimap::BiMap;
use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use up_rust::{ComparableListener, UCode, UListener, UStatus, UUri};

pub(crate) struct ListenerRegistry {
    listener_id_and_listener_config:
        TimedRwLock<BiMap<usize, (UUri, Option<UUri>, ComparableListener)>>,
    listener_id_to_client_id: TimedRwLock<HashMap<usize, ClientId>>,
    client_id_to_listener_id: TimedRwLock<HashMap<ClientId, HashSet<usize>>>,
    client_and_app_name: TimedRwLock<BiMap<ClientId, ApplicationName>>,
}

impl ListenerRegistry {
    pub fn new() -> Self {
        Self {
            listener_id_and_listener_config: TimedRwLock::new(BiMap::new()),
            listener_id_to_client_id: TimedRwLock::new(HashMap::new()),
            client_id_to_listener_id: TimedRwLock::new(HashMap::new()),
            client_and_app_name: TimedRwLock::new(BiMap::new()),
        }
    }

    pub async fn remove_listener_id_and_listener_config_based_on_listener_id(
        &self,
        listener_id: usize,
    ) -> Result<(), UStatus> {
        let removed = self
            .listener_id_and_listener_config
            .write()
            .await
            .remove_by_left(&listener_id);
        if removed.is_none() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("No listener_config for listener_id: {listener_id}"),
            ));
        }

        Ok(())
    }

    pub async fn insert_listener_id_client_id(
        &self,
        listener_id: usize,
        client_id: ClientId,
    ) -> Option<(usize, ClientId)> {
        let mut listener_id_to_client_id = self.listener_id_to_client_id.write().await;
        let mut client_id_to_listener_id = self.client_id_to_listener_id.write().await;

        debug!("before listener_id_to_client_id: {listener_id_to_client_id:?}");

        if listener_id_to_client_id.contains_key(&listener_id) {
            info!("We already used listener_id: {listener_id}");
            debug!("listener_id_to_client_id: {listener_id_to_client_id:?}");

            let client_id = listener_id_to_client_id.get(&listener_id).unwrap();
            return Some((listener_id, client_id.clone()));
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

    pub async fn remove_client_id_based_on_listener_id(
        &self,
        listener_id: usize,
    ) -> Option<ClientId> {
        let mut listener_id_to_client_id = self.listener_id_to_client_id.write().await;
        let mut client_id_to_listener_id = self.client_id_to_listener_id.write().await;

        let removed = listener_id_to_client_id.remove(&listener_id);
        if let Some(client_id) = removed {
            client_id_to_listener_id
                .entry(client_id.clone())
                .or_default()
                .remove(&listener_id);

            return Some(client_id);
        }

        None
    }

    pub async fn listener_count_for_client_id(&self, client_id: ClientId) -> usize {
        let client_id_to_listener_id = self.client_id_to_listener_id.read().await;

        let listener_ids = client_id_to_listener_id.get(&client_id);
        if let Some(listener_ids) = listener_ids {
            return listener_ids.len();
        }

        0
    }

    pub async fn insert_listener_id_and_listener_config(
        &self,
        listener_id: usize,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<(), UStatus> {
        let mut listener_id_and_listener_config =
            self.listener_id_and_listener_config.write().await;
        let insertion_res =
            listener_id_and_listener_config.insert_no_overwrite(listener_id, listener_config);

        if let Err(_err) = insertion_res {
            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                "Unable to set_listener_id_and_listener_config, one side or the other already exists",
            ));
        }

        Ok(())
    }

    pub async fn insert_client_and_app_name(
        &self,
        client_id: ClientId,
        app_name: ApplicationName,
    ) -> Result<(), UStatus> {
        let mut client_and_app_name = self.client_and_app_name.write().await;
        if let Err(existing) = client_and_app_name.insert_no_overwrite(client_id, app_name.clone())
        {
            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                format!("Already exists that pair of client_id and app_name: {existing:?}"),
            ));
        }

        Ok(())
    }

    pub async fn remove_app_name_for_client_id(
        &self,
        client_id: ClientId,
    ) -> Option<ApplicationName> {
        let mut client_and_app_name = self.client_and_app_name.write().await;

        let Some((_client_id, app_name)) = client_and_app_name.remove_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub async fn get_app_name_for_client_id(&self, client_id: ClientId) -> Option<ApplicationName> {
        let client_and_app_name = self.client_and_app_name.read().await;

        let Some(app_name) = client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub async fn get_app_name_for_listener_id(
        &self,
        listener_id: usize,
    ) -> Option<ApplicationName> {
        let listener_id_to_client_id = self.listener_id_to_client_id.read().await;
        let client_and_app_name = self.client_and_app_name.read().await;

        let Some(client_id) = listener_id_to_client_id.get(&listener_id) else {
            return None;
        };

        let Some(app_name) = client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub async fn get_listener_id_for_listener_config(
        &self,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Option<usize> {
        let listener_id_and_listener_config = self.listener_id_and_listener_config.read().await;

        let Some(listener_id) = listener_id_and_listener_config.get_by_right(&listener_config)
        else {
            return None;
        };

        Some(listener_id.clone())
    }

    pub async fn get_listener_for_listener_id(
        &self,
        listener_id: usize,
    ) -> Option<Arc<dyn UListener>> {
        let listener_id_and_listener_config = self.listener_id_and_listener_config.read().await;

        let Some((_, _, comp_listener)) = listener_id_and_listener_config.get_by_left(&listener_id)
        else {
            return None;
        };

        Some(comp_listener.into_inner())
    }

    pub async fn print_rwlock_times(&self) {
        println!("listener_id_and_listener_config:");
        println!(
            "reads: {:?}",
            self.listener_id_and_listener_config.read_durations().await
        );
        println!(
            "writes: {:?}",
            self.listener_id_and_listener_config.write_durations().await
        );

        println!("listener_id_to_client_id:");
        println!(
            "reads: {:?}",
            self.listener_id_to_client_id.read_durations().await
        );
        println!(
            "writes: {:?}",
            self.listener_id_to_client_id.write_durations().await
        );

        println!("client_id_to_listener_id:");
        println!(
            "reads: {:?}",
            self.client_id_to_listener_id.read_durations().await
        );
        println!(
            "writes: {:?}",
            self.client_id_to_listener_id.write_durations().await
        );

        println!("client_and_app_name:");
        println!(
            "reads: {:?}",
            self.client_and_app_name.read_durations().await
        );
        println!(
            "writes: {:?}",
            self.client_and_app_name.write_durations().await
        );
    }
}
