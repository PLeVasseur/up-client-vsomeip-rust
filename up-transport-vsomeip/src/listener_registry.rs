use crate::{ApplicationName, AuthorityName, ClientId};
use bimap::BiMap;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use up_rust::{ComparableListener, UCode, UListener, UStatus, UUri};

pub(crate) struct ListenerRegistry {
    listener_id_and_listener_config: BiMap<usize, (UUri, Option<UUri>, ComparableListener)>,
    listener_id_to_client_id: HashMap<usize, ClientId>,
    client_id_to_listener_id: HashMap<ClientId, HashSet<usize>>,
    client_and_app_name: BiMap<ClientId, ApplicationName>,
}

impl ListenerRegistry {
    pub fn new() -> Self {
        Self {
            listener_id_and_listener_config: BiMap::new(),
            listener_id_to_client_id: HashMap::new(),
            client_id_to_listener_id: HashMap::new(),
            client_and_app_name: BiMap::new(),
        }
    }

    pub fn remove_listener_id_and_listener_config_based_on_listener_id(
        &mut self,
        listener_id: usize,
    ) -> Result<(), UStatus> {
        let removed = self
            .listener_id_and_listener_config
            .remove_by_left(&listener_id);
        if removed.is_none() {
            return Err(UStatus::fail_with_code(
                UCode::NOT_FOUND,
                format!("No listener_config for listener_id: {listener_id}"),
            ));
        }

        Ok(())
    }

    pub fn insert_listener_id_client_id(
        &mut self,
        listener_id: usize,
        client_id: ClientId,
    ) -> Option<(usize, ClientId)> {
        if self.listener_id_to_client_id.contains_key(&listener_id) {
            let client_id = self.listener_id_to_client_id.get(&listener_id).unwrap();
            return Some((listener_id, client_id.clone()));
        }

        let insert_res = self.listener_id_to_client_id.insert(listener_id, client_id);
        if let Some(client_id) = insert_res {
            return Some((listener_id, client_id));
        }

        let listener_ids = self.client_id_to_listener_id.entry(client_id).or_default();
        let newly_added = listener_ids.insert(listener_id);
        if !newly_added {
            return Some((listener_id, client_id));
        }

        None
    }

    pub fn remove_client_id_based_on_listener_id(
        &mut self,
        listener_id: usize,
    ) -> Option<ClientId> {
        let removed = self.listener_id_to_client_id.remove(&listener_id);
        if let Some(client_id) = removed {
            self.client_id_to_listener_id
                .entry(client_id.clone())
                .or_default()
                .remove(&listener_id);

            return Some(client_id);
        }

        None
    }

    pub fn listener_count_for_client_id(&self, client_id: ClientId) -> usize {
        let listener_ids = self.client_id_to_listener_id.get(&client_id);
        if let Some(listener_ids) = listener_ids {
            return listener_ids.len();
        }

        0
    }

    pub fn insert_listener_id_and_listener_config(
        &mut self,
        listener_id: usize,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Result<(), UStatus> {
        let insertion_res = self
            .listener_id_and_listener_config
            .insert_no_overwrite(listener_id, listener_config);

        if let Err(_err) = insertion_res {
            return Err(UStatus::fail_with_code(
                UCode::ALREADY_EXISTS,
                "Unable to set_listener_id_and_listener_config, one side or the other already exists",
            ));
        }

        Ok(())
    }

    pub fn insert_client_and_app_name(
        &mut self,
        client_id: ClientId,
        app_name: ApplicationName,
    ) -> Result<(), UStatus> {
        if let Err(err) = self
            .client_and_app_name
            .insert_no_overwrite(client_id, app_name.clone())
        {
            return Err(UStatus::fail_with_code(UCode::ALREADY_EXISTS, format!("Already exists that pair of client_id and app_name: client_id: {client_id}, app_name: {app_name}")));
        }

        Ok(())
    }

    pub fn remove_app_name_for_client_id(
        &mut self,
        client_id: ClientId,
    ) -> Option<ApplicationName> {
        let Some((_client_id, app_name)) = self.client_and_app_name.remove_by_left(&client_id)
        else {
            return None;
        };

        Some(app_name.clone())
    }

    pub fn get_app_name_for_client_id(&self, client_id: ClientId) -> Option<ApplicationName> {
        let Some(app_name) = self.client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub fn get_app_name_for_listener_id(&self, listener_id: usize) -> Option<ApplicationName> {
        let Some(client_id) = self.listener_id_to_client_id.get(&listener_id) else {
            return None;
        };

        let Some(app_name) = self.client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub fn get_listener_id_for_listener_config(
        &self,
        listener_config: (UUri, Option<UUri>, ComparableListener),
    ) -> Option<usize> {
        let Some(listener_id) = self
            .listener_id_and_listener_config
            .get_by_right(&listener_config)
        else {
            return None;
        };

        Some(listener_id.clone())
    }

    pub fn get_listener_for_listener_id(&self, listener_id: usize) -> Option<Arc<dyn UListener>> {
        let Some((_, _, comp_listener)) = self
            .listener_id_and_listener_config
            .get_by_left(&listener_id)
        else {
            return None;
        };

        Some(comp_listener.into_inner())
    }
}
