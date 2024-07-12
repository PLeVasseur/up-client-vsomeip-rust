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
/// implementation.
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
            extern "C" fn #extern_fn_name(vsomeip_msg: &SharedPtr<vsomeip::message>) {
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

        lazy_static! {
            static ref FREE_LISTENER_IDS: TokioRwLock<HashSet<usize>> = {
                #free_listener_ids_init
                TokioRwLock::new(set)
            };
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
                let transport_storage_res = ProcMacroTransportStorage::get_listener_id_transport(listener_id).await;

                let transport_storage = {
                    match transport_storage_res {
                        Some(transport_storage) => transport_storage.clone(),
                        None => {
                            warn!("No transport storage found for listener_id: {listener_id}");
                            return;
                        }
                    }
                };

                // Separate the scope for accessing the registry
                let app_name = {
                    let registry = transport_storage.get_registry().await.clone();
                    match registry.get_app_name_for_listener_id(listener_id).await {
                        Some(app_name) => app_name,
                        None => {
                            warn!("No vsomeip app_name found for listener_id: {listener_id}");
                            return;
                        }
                    }
                };

                let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());
                let_cxx_string!(app_name_cxx = &*app_name);

                let application_wrapper = make_application_wrapper(
                    get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                );
                if application_wrapper.get_mut().is_null() {
                    error!("Unable to obtain vsomeip application with app_name: {app_name}");
                    return;
                }
                let cloned_vsomeip_msg = vsomeip_msg.clone();
                let mut vsomeip_msg_wrapper = make_message_wrapper(cloned_vsomeip_msg);

                trace!("Made vsomeip_msg_wrapper");

                let authority_name = transport_storage.get_local_authority();
                let remote_authority_name = transport_storage.get_remote_authority();

                // Change: Cloning transport_storage again for the async call
                let transport_storage_clone = transport_storage.clone();
                let res = convert_vsomeip_msg_to_umsg(
                    &authority_name,
                    &remote_authority_name,
                    transport_storage_clone,
                    &mut vsomeip_msg_wrapper,
                    &application_wrapper,
                    &runtime_wrapper,
                )
                .await;

                trace!("Ran convert_vsomeip_msg_to_umsg");

                let Ok(umsg) = res else {
                    if let Err(err) = res {
                        error!("Unable to convert vsomeip message to UMessage: {:?}", err);
                    }
                    return;
                };

                trace!("Was able to convert to UMessage");

                trace!("Calling listener registered under {}", listener_id);

                // Separate the scope for accessing the registry for listener
                let listener = {
                    let registry = transport_storage.get_registry().await.clone();
                    match registry.get_listener_for_listener_id(listener_id).await {
                        Some(listener) => {
                            // Send the listener and umsg back to the main thread
                            if tx.send((listener, umsg)).is_err() {
                                error!("Failed to send listener and umsg to main thread");
                            }
                        },
                        None => {
                            error!("Listener not found for ID {}", listener_id);
                            return;
                        }
                    }
                };
            });
            runtime.block_on(local_set);

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
                trace!("Within spawned thread -- calling shared_async_fn");
                shared_async_fn(listener, umsg).await;
                trace!("Within spawned thread -- finished shared_async_fn");
            });
        }

        async fn shared_async_fn(listener: Arc<dyn UListener>, umsg: UMessage) {
            trace!("shared_async_fn with umsg: {:?}", umsg);
            listener.on_receive(umsg).await;
        }

        pub(crate) fn get_extern_fn(listener_id: usize) -> extern "C" fn(&SharedPtr<vsomeip::message>) {
            trace!("get_extern_fn with listener_id: {}", listener_id);
            match listener_id {
                #(#match_arms)*
                _ => panic!("Listener ID out of range"),
            }
        }
    };

    expanded.into()
}
