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
