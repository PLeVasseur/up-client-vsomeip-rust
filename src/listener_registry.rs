use crate::message_conversions::convert_vsomeip_msg_to_umsg;
use crate::{AuthorityName, ClientId, SessionId};
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use log::{error, trace};
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

// TODO: Implement functions here which interact with the above

lazy_static! {
    pub(crate) static ref CLIENT_ID_APP_MAPPING: RwLock<HashMap<ClientId, String>> =
        RwLock::new(HashMap::new());
}

// TODO: Implement functions here which interact with the above
