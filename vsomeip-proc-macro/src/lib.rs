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
            static ref LISTENER_REGISTRY: Mutex<HashMap<usize, Arc<dyn UListener>>> =
                Mutex::new(HashMap::new());
            static ref FREE_LISTENER_IDS: Mutex<HashSet<usize>> = {
                #free_listener_ids_init
                Mutex::new(set)
            };
            static ref LISTENER_ID_MAP: Mutex<HashMap<(UUri, Option<UUri>, ComparableListener), usize>> =
                Mutex::new(HashMap::new());
        }

        #generated_fns

        fn call_shared_extern_fn(listener_id: usize, vsomeip_msg: &SharedPtr<vsomeip::message>) {
            let cloned_vsomeip_msg = vsomeip_msg.clone();
            let mut vsomeip_msg_wrapper = make_message_wrapper(cloned_vsomeip_msg);
            let app_name = UPClientVsomeip::get_app_name();
            let runtime_wrapper = make_runtime_wrapper(vsomeip::runtime::get());
            let_cxx_string!(app_name_cxx = app_name);
            let application_wrapper = make_application_wrapper(
                get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
            );
            let res = convert_vsomeip_msg_to_umsg(&mut vsomeip_msg_wrapper, &application_wrapper, &runtime_wrapper);

            let Ok(umsg) = res else {
                if let Err(err) = res {
                    // TODO: Add some logging here
                }
                return;
            };

            // TODO: Replace with the log crate
            println!("Calling extern function #{}", listener_id);
            let registry = LISTENER_REGISTRY.lock().unwrap();
            if let Some(listener) = registry.get(&listener_id) {
                let listener = Arc::clone(listener);
                tokio::spawn(async move {
                    shared_async_fn(listener, umsg).await;
                });
            } else {
                println!("Listener not found for ID {}", listener_id);
            }
        }

        async fn shared_async_fn(listener: Arc<dyn UListener>, umsg: UMessage) {
            listener.on_receive(umsg).await;
        }

        fn get_extern_fn(listener_id: usize) -> extern "C" fn(&SharedPtr<vsomeip::message>) {
            match listener_id {
                #(#match_arms)*
                _ => panic!("Listener ID out of range"),
            }
        }
    };

    expanded.into()
}
