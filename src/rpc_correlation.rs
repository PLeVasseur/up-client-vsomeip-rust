use crate::{ClientId, ReqId, RequestId, SessionId};
use lazy_static::lazy_static;
use std::collections::HashMap;
use tokio::sync::RwLock;

// TODO: Spin this off into its own concept as what's needed for RPC Request -> Response correlation
lazy_static! {
    pub(crate) static ref UE_REQUEST_CORRELATION: RwLock<HashMap<RequestId, ReqId>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref ME_REQUEST_CORRELATION: RwLock<HashMap<ReqId, RequestId>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref CLIENT_ID_SESSION_ID_TRACKING: RwLock<HashMap<ClientId, SessionId>> =
        RwLock::new(HashMap::new());
}

pub(crate) async fn retrieve_session_id(client_id: ClientId) -> SessionId {
    let mut client_id_session_id_tracking = CLIENT_ID_SESSION_ID_TRACKING.write().await;

    let current_sesion_id = client_id_session_id_tracking.entry(client_id).or_insert(1);
    let returned_session_id = *current_sesion_id;
    *current_sesion_id += 1;
    returned_session_id
}
