use crate::{ApplicationName, AuthorityName, ClientId};
use bimap::BiMap;
use std::sync::Arc;
use up_rust::{ComparableListener, UListener, UUri};

pub(crate) struct ListenerRegistry {
    listener_id_and_listener_config: BiMap<usize, (UUri, Option<UUri>, ComparableListener)>,
    listener_and_client: BiMap<usize, ClientId>,
    client_and_app_name: BiMap<ClientId, ApplicationName>,
}

impl ListenerRegistry {
    pub fn new() -> Self {
        Self {
            listener_id_and_listener_config: BiMap::new(),
            listener_and_client: BiMap::new(),
            client_and_app_name: BiMap::new(),
        }
    }

    pub fn get_app_name_for_client_id(&self, client_id: ClientId) -> Option<ApplicationName> {
        let Some(app_name) = self.client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
    }

    pub fn get_app_name_for_listener_id(&self, listener_id: usize) -> Option<ApplicationName> {
        let Some(client_id) = self.listener_and_client.get_by_left(&listener_id) else {
            return None;
        };

        let Some(app_name) = self.client_and_app_name.get_by_left(&client_id) else {
            return None;
        };

        Some(app_name.clone())
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
