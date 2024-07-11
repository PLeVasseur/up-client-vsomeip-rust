use crate::{ApplicationName, AuthorityName, ClientId};
use bimap::BiMap;
use std::sync::Arc;
use up_rust::{ComparableListener, UCode, UListener, UStatus, UUri};

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

    pub fn set_listener_id_and_listener_config(
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
