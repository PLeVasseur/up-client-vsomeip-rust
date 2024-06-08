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
            pub static ref LISTENER_CLIENT_ID_MAPPING: Mutex<HashMap<usize, ClientId>> = Mutex::new(HashMap::new());
            pub static ref CLIENT_ID_APP_MAPPING: Mutex<HashMap<ClientId, String>> = Mutex::new(HashMap::new());
            pub static ref UE_REQUEST_CORRELATION: Mutex<HashMap<RequestId, ReqId>> = Mutex::new(HashMap::new());
            pub static ref ME_REQUEST_CORRELATION: Mutex<HashMap<ReqId, RequestId>> =
                Mutex::new(HashMap::new());
            pub static ref CLIENT_ID_SESSION_ID_TRACKING: Mutex<HashMap<ClientId, SessionId>> =
                Mutex::new(HashMap::new());
        }

        // TODO: Architecting things this way with some lazy_static here means we can have only one
        //  UPClientVsomeip per process, which is fine in practice, but will make unit tests and integration
        //  tests more painful
        lazy_static! {
            static ref LISTENER_REGISTRY: Mutex<HashMap<usize, Arc<dyn UListener>>> =
                Mutex::new(HashMap::new());
            static ref FREE_LISTENER_IDS: Mutex<HashSet<usize>> = {
                #free_listener_ids_init
                Mutex::new(set)
            };
            static ref LISTENER_ID_MAP: Mutex<HashMap<(UUri, Option<UUri>, ComparableListener), usize>> =
                Mutex::new(HashMap::new());
            static ref POINT_TO_POINT_LISTENERS: Mutex<HashSet<usize>> = Mutex::new(HashSet::new());
        }

        #generated_fns

        fn call_shared_extern_fn(listener_id: usize, vsomeip_msg: &SharedPtr<vsomeip::message>) {
            trace!("Calling call_shared_extern_fn with listener_id: {}", listener_id);

            let app_name = {
                let listener_client_id_mapping = LISTENER_CLIENT_ID_MAPPING.lock().unwrap();
                if let Some(client_id) = listener_client_id_mapping.get(&listener_id) {
                    let client_id_app_mapping = CLIENT_ID_APP_MAPPING.lock().unwrap();
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

            let point_to_point_listeners = POINT_TO_POINT_LISTENERS.lock().unwrap();
            if point_to_point_listeners.contains(&listener_id) {
                if !is_point_to_point_message(&mut vsomeip_msg_wrapper) {
                    // TODO: Log an INFO level message, since it's fairly likely to occur
                    //  and we don't want to spam the log
                    trace!("We're listening for point-to-point messages, but this isn't one");

                    return;
                } else {
                    // TODO: Add logging here that this proceeded

                    trace!("Not a point-to-point message");
                }
            }

            trace!("Checked point-to-point or not");

            let res = convert_vsomeip_msg_to_umsg(&mut vsomeip_msg_wrapper, &application_wrapper, &runtime_wrapper);

            // TODO: Add another function, call it shared_async_fn_err and that's where we can
            //  send error cases when their listener should be called back

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
            let registry = LISTENER_REGISTRY.lock().unwrap();
            if let Some(listener) = registry.get(&listener_id) {
                trace!("Retrieved listener");
                let listener = Arc::clone(listener);

                // TODO: Should probably push this over to an existing async runtime...
                //  for now we will just create one here as a hack
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    tokio::spawn(async move {
                        trace!("Within spawned thread -- calling shared_async_fn");
                        shared_async_fn(listener, umsg).await;
                        trace!("Within spawned thread -- finished shared_async_fn");
                    });
                });
            } else {
                error!("Listener not found for ID {}", listener_id);
            }

            trace!("Reached bottom of call_shared_extern_fn");
        }

        async fn shared_async_fn(listener: Arc<dyn UListener>, umsg: UMessage) {
            trace!("shared_async_fn with umsg: {:?}", umsg);
            listener.on_receive(umsg).await;
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
