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

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, LitInt};

/// Generates "N" number of extern "C" fns to be used and recycled by the up-client-vsomeip-rust
/// imlementation.
///
/// # Rationale
///
/// The vsomeip-sys crate requires extern "C" fns to be passed to it when registering a message handler
///
/// By using pre-generated extern "C" fns we are then able to ignore that implementation detail inside
/// of the UTransport implementation of vsomeip
#[proc_macro]
pub fn generate_message_handler_extern_c_fns(input: TokenStream) -> TokenStream {
    let num_fns = parse_macro_input!(input as LitInt)
        .base10_parse::<usize>()
        .unwrap();

    let mut generated_fns = quote! {};
    let mut match_arms = Vec::with_capacity(num_fns);
    let mut free_listener_ids_init = quote! {
        let mut set = HashSet::with_capacity(#num_fns);
    };

    for i in 0..num_fns {
        let extern_fn_name = format_ident!("extern_on_msg_wrapper_{}", i);

        let fn_code = quote! {
            #[no_mangle]
            pub extern "C" fn #extern_fn_name(vsomeip_msg: &SharedPtr<vsomeip::message>) {
                trace!("Calling extern_fn: {}", #i);
                call_shared_extern_fn(#i, vsomeip_msg);
            }
        };

        generated_fns.extend(fn_code);

        let match_arm = quote! {
            #i => #extern_fn_name,
        };
        match_arms.push(match_arm);

        free_listener_ids_init.extend(quote! {
            set.insert(#i);
        });
    }

    let expanded = quote! {

        // TODO: Architecting things this way with some lazy_static here means we can have only one
        //  UPClientVsomeip per process, which is fine in practice, but will make unit tests and integration
        //  tests more painful
        lazy_static! {
            pub static ref AUTHORITY_NAME: Mutex<String> = Mutex::new(String::new());
            pub static ref LISTENER_CLIENT_ID_MAPPING: InstrumentedRwLock<HashMap<usize, ClientId>> = InstrumentedRwLock::new(HashMap::new());
            pub static ref CLIENT_ID_APP_MAPPING: InstrumentedRwLock<HashMap<ClientId, String>> = InstrumentedRwLock::new(HashMap::new());
            pub static ref UE_REQUEST_CORRELATION: InstrumentedRwLock<HashMap<RequestId, ReqId>> = InstrumentedRwLock::new(HashMap::new());
            pub static ref ME_REQUEST_CORRELATION: InstrumentedRwLock<HashMap<ReqId, RequestId>> =
                InstrumentedRwLock::new(HashMap::new());
            pub static ref CLIENT_ID_SESSION_ID_TRACKING: InstrumentedRwLock<HashMap<ClientId, SessionId>> =
                InstrumentedRwLock::new(HashMap::new());
        }

        // TODO: Architecting things this way with some lazy_static here means we can have only one
        //  UPClientVsomeip per process, which is fine in practice, but will make unit tests and integration
        //  tests more painful
        lazy_static! {
            pub static ref LISTENER_REGISTRY: InstrumentedRwLock<HashMap<usize, Arc<dyn UListener>>> =
                InstrumentedRwLock::new(HashMap::new());
            pub static ref FREE_LISTENER_IDS: InstrumentedRwLock<HashSet<usize>> = {
                #free_listener_ids_init
                InstrumentedRwLock::new(set)
            };
            pub static ref LISTENER_ID_MAP: InstrumentedRwLock<HashMap<(UUri, Option<UUri>, ComparableListener), usize>> =
                InstrumentedRwLock::new(HashMap::new());
            // static ref POINT_TO_POINT_LISTENERS: Mutex<HashSet<usize>> = Mutex::new(HashSet::new());
        }

        #generated_fns


fn call_shared_extern_fn(listener_id: usize, vsomeip_msg: &SharedPtr<vsomeip::message>) {
    // Ensure the runtime is initialized and get a handle to it
    let runtime = get_runtime();

    // Create a LocalSet for running !Send futures
    let local_set = LocalSet::new();

    let vsomeip_msg = make_message_wrapper(vsomeip_msg.clone()).get_shared_ptr();

    // Create a standard mpsc channel to send the listener and umsg back to the main thread
    let (tx, rx) = mpsc::channel();

    let before_local_set_spawn_local = Instant::now();

    // Use the runtime to run the async function within the LocalSet
    local_set.spawn_local(async move {
        let top_of_local_set_spawn_local = Instant::now();
        trace!("Calling call_shared_extern_fn with listener_id: {}", listener_id);

        let app_name = {
            let listener_client_id_mapping = LISTENER_CLIENT_ID_MAPPING.read().await;
            if let Some(client_id) = listener_client_id_mapping.get(&listener_id) {
                let client_id_app_mapping = CLIENT_ID_APP_MAPPING.read().await;
                if let Some(app_name) = client_id_app_mapping.get(&client_id) {
                    Ok(app_name.clone())
                } else {
                    Err(UStatus::fail_with_code(UCode::NOT_FOUND, format!("There was no app_name found for listener_id: {} and client_id: {}", listener_id, client_id)))
                }
            } else {
                Err(UStatus::fail_with_code(UCode::NOT_FOUND, format!("There was no client_id found for listener_id: {}", listener_id)))
            }
        };

        let Ok(app_name) = app_name else {
            error!("App wasn't found to interact with: {:?}", app_name.err().unwrap());
            return;
        };

        let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());
        let_cxx_string!(app_name_cxx = &*app_name);

        // TODO: May want to add a check here that we did succeed. Perhaps within make_application_wrapper
        let application_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
        );
        let cloned_vsomeip_msg = vsomeip_msg.clone();
        let mut vsomeip_msg_wrapper = make_message_wrapper(cloned_vsomeip_msg);

        trace!("Made vsomeip_msg_wrapper");

        let res = convert_vsomeip_msg_to_umsg(&mut vsomeip_msg_wrapper, &application_wrapper, &runtime_wrapper).await;

        let after_converting_vsomeip_msg_to_umsg = Instant::now();
        let following_converting_vsomeip_msg_to_umsg = after_converting_vsomeip_msg_to_umsg - top_of_local_set_spawn_local;
        error!("following_converting_vsomeip_msg_to_umsg: {following_converting_vsomeip_msg_to_umsg:?}");

        trace!("Ran convert_vsomeip_msg_to_umsg");

        let Ok(umsg) = res else {
            if let Err(err) = res {
                error!("Unable to convert vsomeip message to UMessage: {:?}", err);
            }
            return;
        };

        trace!("Was able to convert to UMessage");

        // TODO: Replace with the log crate
        trace!("Calling extern function {}", listener_id);
        let registry = LISTENER_REGISTRY.read().await;
        let after_getting_registry = Instant::now();
        let following_getting_registry = after_getting_registry - top_of_local_set_spawn_local;
        error!("following_getting_registry: {following_getting_registry:?}");

        if let Some(listener) = registry.get(&listener_id) {
            trace!("Retrieved listener");
            let listener = Arc::clone(listener);

            // Send the listener and umsg back to the main thread
            if tx.send((listener, umsg)).is_err() {
                error!("Failed to send listener and umsg to main thread");
            }

        } else {
            error!("Listener not found for ID {}", listener_id);
        }

        let bottom_of_local_set_spawn_local = Instant::now();
        let duration_of_local_set_spawned_task = bottom_of_local_set_spawn_local - top_of_local_set_spawn_local;
        error!("duration_of_local_set_spawned_task: {duration_of_local_set_spawned_task:?}");
    });

    let before_runtime_block_on = Instant::now();
    runtime.block_on(local_set);
    let after_runtime_block_on = Instant::now();

    let duration_of_runtime_block = after_runtime_block_on - before_runtime_block_on;
    error!("duration_of_runtime_block: {duration_of_runtime_block:?}");

    trace!("Reached bottom of call_shared_extern_fn");

    // Receive the listener and umsg from the mpsc channel
    let (listener, umsg) = match rx.recv() {
        Ok((listener, umsg)) => (listener, umsg),
        Err(_) => {
            error!("Failed to receive listener and umsg from mpsc channel");
            return;
        }
    };

    // Spawn shared_async_fn on the multi-threaded executor
    CB_RUNTIME.spawn(async move {
        let before_shared_async_fn = Instant::now();
        trace!("Within spawned thread -- calling shared_async_fn");

        shared_async_fn(listener, umsg).await;

        let after_shared_async_fn = Instant::now();
        let duration_of_shared_async_fn = after_shared_async_fn - before_shared_async_fn;
        error!("duration_of_shared_async_fn: {duration_of_shared_async_fn:?}");
        trace!("Within spawned thread -- finished shared_async_fn");
    });

    let after_spawning_onto_multithreaded_executor = Instant::now();

    let following_spawning_onto_multithreaded_executor = after_spawning_onto_multithreaded_executor - before_runtime_block_on;
    error!("following_spawning_onto_multithreaded_executor: {following_spawning_onto_multithreaded_executor:?}");
}

        async fn shared_async_fn(listener: Arc<dyn UListener>, umsg: UMessage) {
            let before_listener_on_receive = Instant::now();
            trace!("shared_async_fn with umsg: {:?}", umsg);
            listener.on_receive(umsg).await;
            let after_listener_on_receive = Instant::now();
            let duration_of_listener_on_receive = after_listener_on_receive - before_listener_on_receive;
            error!("duration_of_listener_on_receive: {duration_of_listener_on_receive:?}");
        }

        fn get_extern_fn(listener_id: usize) -> extern "C" fn(&SharedPtr<vsomeip::message>) {
            trace!("get_extern_fn with listener_id: {}", listener_id);
            match listener_id {
                #(#match_arms)*
                _ => panic!("Listener ID out of range"),
            }
        }
    };

    expanded.into()
}
