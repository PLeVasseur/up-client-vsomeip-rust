use crate::message_conversions::convert_vsomeip_msg_to_umsg;
use crate::{ApplicationName, AuthorityName, ClientId};
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use log::{error, info, trace};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::{mpsc, Arc};
use tokio::runtime::Runtime;
use tokio::sync::RwLock;
use tokio::task::LocalSet;
use tokio::time::Instant;
use up_rust::{ComparableListener, UListener, UUri};
use up_rust::{UCode, UMessage, UStatus};
use vsomeip_proc_macro::generate_message_handler_extern_c_fns;
use vsomeip_sys::glue::{make_application_wrapper, make_message_wrapper, make_runtime_wrapper};
use vsomeip_sys::safe_glue::get_pinned_runtime;
use vsomeip_sys::vsomeip;

const THREAD_NUM: usize = 10;

// Create a separate tokio Runtime for running the callback
lazy_static! {
    static ref CB_RUNTIME: Runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(THREAD_NUM)
        .enable_all()
        .build()
        .expect("Unable to create callback runtime");
}

static RUNTIME: Lazy<Arc<Runtime>> =
    Lazy::new(|| Arc::new(Runtime::new().expect("Failed to create Tokio runtime")));

fn get_runtime() -> Arc<Runtime> {
    Arc::clone(&RUNTIME)
}

generate_message_handler_extern_c_fns!(10000);

type ListenerIdMap = RwLock<HashMap<(UUri, Option<UUri>, ComparableListener), usize>>;
lazy_static! {
    pub(crate) static ref LISTENER_ID_CLIENT_ID_MAPPING: RwLock<HashMap<usize, ClientId>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref LISTENER_ID_AUTHORITY_NAME: RwLock<HashMap<usize, AuthorityName>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref LISTENER_ID_REMOTE_AUTHORITY_NAME: RwLock<HashMap<usize, AuthorityName>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref LISTENER_REGISTRY: RwLock<HashMap<usize, Arc<dyn UListener>>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref LISTENER_ID_MAP: ListenerIdMap = RwLock::new(HashMap::new());
}

pub(crate) async fn free_listener_id(listener_id: usize) -> UStatus {
    info!("listener_id was not used since we already have registered for this");
    let mut free_ids = FREE_LISTENER_IDS.write().await;
    free_ids.insert(listener_id);
    UStatus::fail_with_code(
        UCode::ALREADY_EXISTS,
        "Already have registered with this source, sink and listener",
    )
}

pub(crate) async fn insert_into_listener_id_map(
    authority_name: &AuthorityName,
    remote_authority_name: &AuthorityName,
    key: (UUri, Option<UUri>, ComparableListener),
    listener_id: usize,
) -> bool {
    trace!(
        "authority_name: {}, remote_authority_name: {}, listener_id: {}",
        authority_name,
        remote_authority_name,
        listener_id
    );

    // TODO: Should ensure that we don't record a partial transaction by rolling back any pieces which succeeded if a latter part fails

    let mut id_map = LISTENER_ID_MAP.write().await;
    if id_map.insert(key, listener_id).is_some() {
        trace!(
            "Not inserted into LISTENER_ID_MAP since we already have registered for this Request"
        );
        return false;
    } else {
        trace!("Inserted into LISTENER_ID_MAP");
    }

    let mut listener_id_authority_name = LISTENER_ID_AUTHORITY_NAME.write().await;

    trace!(
        "checking listener_id_authority_name: {:?}",
        *listener_id_authority_name
    );

    if listener_id_authority_name
        .insert(listener_id, authority_name.to_string())
        .is_some()
    {
        trace!(
            "Not inserted into LISTENER_ID_AUTHORITY_NAME since we already have registered for this Request"
        );
        return false;
    } else {
        trace!("Inserted into LISTENER_ID_AUTHORITY_NAME");
    }

    let mut listener_id_remote_authority_name = LISTENER_ID_REMOTE_AUTHORITY_NAME.write().await;
    if listener_id_remote_authority_name
        .insert(listener_id, remote_authority_name.to_string())
        .is_some()
    {
        trace!(
            "Not inserted into LISTENER_ID_REMOTE_AUTHORITY_NAME since we already have registered for this Request"
        );
        return false;
    } else {
        trace!("Inserted into LISTENER_ID_REMOTE_AUTHORITY_NAME");
    }

    true
}

pub(crate) async fn find_available_listener_id() -> Result<usize, UStatus> {
    let mut free_ids = FREE_LISTENER_IDS.write().await;
    if let Some(&id) = free_ids.iter().next() {
        free_ids.remove(&id);
        trace!("find_available_listener_id: {id}");
        Ok(id)
    } else {
        Err(UStatus::fail_with_code(
            UCode::RESOURCE_EXHAUSTED,
            "No more extern C fns available",
        ))
    }
}

// TODO: Implement functions here which interact with the above

lazy_static! {
    pub(crate) static ref CLIENT_ID_APP_MAPPING: RwLock<HashMap<ClientId, String>> =
        RwLock::new(HashMap::new());
}

// TODO: Implement functions here which interact with the above

pub(crate) async fn find_app_name(client_id: ClientId) -> Result<ApplicationName, UStatus> {
    let client_id_app_mapping = CLIENT_ID_APP_MAPPING.read().await;
    if let Some(app_name) = client_id_app_mapping.get(&client_id) {
        Ok(app_name.clone())
    } else {
        Err(UStatus::fail_with_code(
            UCode::NOT_FOUND,
            format!("There was no app_name found for client_id: {}", client_id),
        ))
    }
}
