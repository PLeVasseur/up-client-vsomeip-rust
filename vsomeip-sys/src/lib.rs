/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
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

mod cxx_bridge;

use autocxx::prelude::*;

// using autocxx to generate Rust bindings for all these
include_cpp! {
    #include "vsomeip/vsomeip.hpp"
    #include "runtime_wrapper.h"
    #include "application_wrapper.h"
    #include "message_wrapper.h"
    #include "payload_wrapper.h"
    safety!(unsafe) // see details of unsafety policies described in the 'safety' section of the book
    generate!("vsomeip_v3::runtime")
    generate!("vsomeip_v3::application")
    generate!("vsomeip_v3::message_base")
    generate!("vsomeip_v3::message_t")
    generate!("vsomeip_v3::subscription_status_handler_t")
    generate!("vsomeip_v3::state_handler_t")
    generate!("vsomeip_v3::state_type_e")
    generate!("vsomeip_v3::ANY_MAJOR")
    generate!("vsomeip_v3::ANY_MINOR")
    generate!("vsomeip_v3::ANY_INSTANCE")
    generate!("vsomeip_v3::ANY_SERVICE")
    generate!("vsomeip_v3::ANY_METHOD")
    generate!("vsomeip_v3::ANY_EVENT")
    generate!("vsomeip_v3::ANY_EVENTGROUP")
    generate!("glue::RuntimeWrapper")
    generate!("glue::make_runtime_wrapper")
    generate!("glue::ApplicationWrapper")
    generate!("glue::make_application_wrapper")
    generate!("glue::MessageWrapper")
    generate!("glue::make_message_wrapper")
    generate!("glue::upcast")
    generate!("glue::PayloadWrapper")
    generate!("glue::make_payload_wrapper")
    generate!("glue::set_payload_raw")
    generate!("glue::get_payload_raw")
    generate!("glue::create_payload_wrapper")
}

pub mod vsomeip {
    pub use crate::ffi::vsomeip_v3::*;
}

pub mod extern_callback_wrappers;
pub mod safe_glue;

mod unsafe_fns {
    pub use crate::ffi::glue::create_payload_wrapper;
}

pub mod glue {
    pub use crate::ffi::glue::upcast;
    pub use crate::ffi::glue::{
        make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
        ApplicationWrapper, MessageWrapper, PayloadWrapper, RuntimeWrapper,
    };
}

#[cfg(test)]
mod tests {
    use crate::extern_callback_wrappers::{
        AvailabilityHandlerFnPtr, AvailableStateHandlerFnPtr, MessageHandlerFnPtr,
    };
    use crate::ffi::vsomeip_v3::runtime;
    use crate::glue::{
        make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
    };
    use crate::safe_glue::{
        get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base,
        get_pinned_payload, get_pinned_runtime, offer_single_event_safe,
        register_availability_handler_fn_ptr_safe, register_message_handler_fn_ptr_safe,
        register_state_handler_fn_ptr_safe, set_data_safe, set_message_payload,
    };
    use crate::vsomeip;
    use crate::vsomeip::state_type_e;
    use cxx::{let_cxx_string, SharedPtr};
    use lazy_static::lazy_static;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender};
    use std::sync::{mpsc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn test_make_runtime() {
        let my_runtime = runtime::get();
        let runtime_wrapper = make_runtime_wrapper(my_runtime);

        let_cxx_string!(my_app_str = "my_app");
        let mut app_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).create_application(&my_app_str),
        );
        if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
            pinned_app.init();
        }

        extern "C" fn callback(
            _service: crate::vsomeip::service_t,
            _instance: crate::vsomeip::instance_t,
            _availability: bool,
        ) {
            println!("hello from Rust!");
        }
        let callback = AvailabilityHandlerFnPtr(callback);
        register_availability_handler_fn_ptr_safe(&mut app_wrapper, 1, 2, callback, 3, 4);
        let request =
            make_message_wrapper(get_pinned_runtime(&runtime_wrapper).create_request(true));

        let reliable = get_pinned_message_base(&request).is_reliable();

        println!("reliable? {reliable}");

        let mut request =
            make_message_wrapper(get_pinned_runtime(&runtime_wrapper).create_request(true));
        get_pinned_message_base(&request).set_service(1);
        get_pinned_message_base(&request).set_instance(2);
        get_pinned_message_base(&request).set_method(3);

        let mut payload_wrapper =
            make_payload_wrapper(get_pinned_runtime(&runtime_wrapper).create_payload());
        let _foo = get_pinned_payload(&payload_wrapper);

        let data: Vec<u8> = vec![1, 2, 3, 4, 5];

        set_data_safe(get_pinned_payload(&payload_wrapper), &data);

        let data_vec = get_data_safe(&payload_wrapper);
        println!("{:?}", data_vec);

        set_message_payload(&mut request, &mut payload_wrapper);

        println!("set_message_payload");

        let Some(loaded_payload) = get_message_payload(&mut request) else {
            panic!("Unable to get PayloadWrapper from MessageWrapper");
        };

        println!("get_message_payload");

        let loaded_data_vec = get_data_safe(&loaded_payload);

        println!("loaded_data_vec: {loaded_data_vec:?}");

        std::thread::sleep(Duration::from_millis(2000));
    }

    #[test]
    fn test_available_state_handler() {
        // Create a globally accessible sender
        lazy_static! {
            static ref SENDER: Mutex<Option<Sender<state_type_e>>> = Mutex::new(None);
            static ref RECEIVER: Mutex<Option<Receiver<state_type_e>>> = Mutex::new(None);
        }
        // in production code we'll probably have to have a registry of channel receivers
        // tied to specific available_state_handler_i which we look up and get in the controlling thread
        extern "C" fn available_state_handler(available_state: state_type_e) {
            println!("available_state: {available_state:?}");

            if let Some(ref tx) = *SENDER.lock().unwrap() {
                tx.send(available_state).unwrap();
            }
        }

        let app_name = "check_available_app";

        // Create a channel
        let (tx, rx) = mpsc::channel();

        // Store the sender and receiver in the global variables
        *SENDER.lock().unwrap() = Some(tx);
        *RECEIVER.lock().unwrap() = Some(rx);

        let app_name_check = app_name.to_string();
        let handle = thread::spawn(move || {
            let my_runtime = runtime::get();
            let runtime_wrapper = make_runtime_wrapper(my_runtime);

            let_cxx_string!(app_name_cxx = app_name_check);
            let app_wrapper = make_application_wrapper(
                get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
            );

            println!("Before starting app, in theory:");
            match get_pinned_application(&app_wrapper) {
                Some(_pinned_app) => {
                    panic!("Application had started");
                }
                None => {
                    println!("Application not started yet");
                }
            }

            let binding = RECEIVER.lock().unwrap();
            let rx = binding.as_ref().unwrap();

            while let Ok(msg) = rx.recv_timeout(Duration::from_secs(5)) {
                println!("Received: {:?}", msg);

                match msg {
                    state_type_e::ST_REGISTERED => {
                        let app_wrapper = make_application_wrapper(
                            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                        );

                        println!("After starting app, in theory:");
                        match get_pinned_application(&app_wrapper) {
                            Some(_pinned_app) => {
                                println!("Application had started");
                            }
                            None => {
                                panic!("Application not started yet");
                            }
                        }
                    }
                    state_type_e::ST_DEREGISTERED => {
                        let app_wrapper = make_application_wrapper(
                            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_cxx),
                        );

                        println!("After stopping app, in theory:");
                        match get_pinned_application(&app_wrapper) {
                            Some(_pinned_app) => {
                                panic!("Application still running");
                            }
                            None => {
                                println!("Application has stopped");
                            }
                        }
                    }
                }
            }
        });

        thread::sleep(Duration::from_millis(500));

        let app_name_start = app_name.to_string();
        thread::spawn(move || {
            let my_runtime = runtime::get();
            let runtime_wrapper = make_runtime_wrapper(my_runtime);

            let_cxx_string!(app_name_cxx = app_name_start);
            let mut app_wrapper = make_application_wrapper(
                get_pinned_runtime(&runtime_wrapper).create_application(&app_name_cxx),
            );
            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.init();
            }
            let state_handler = AvailableStateHandlerFnPtr(available_state_handler);
            register_state_handler_fn_ptr_safe(&mut app_wrapper, state_handler);
            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.start();
            }

            thread::sleep(Duration::from_millis(500));

            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.stop();
            }
        });

        let _ = handle.join();
    }

    #[test]
    fn test_service_availability_handler() {
        lazy_static! {
            static ref TIMES_MESSAGE_RECEIVED: AtomicUsize = AtomicUsize::new(0);
            static ref PAIR: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
        }

        fn wait_for_service_available() {
            let (lock, cvar) = &*PAIR;
            let mut started = lock.lock().unwrap();
            while !*started {
                started = cvar.wait(started).unwrap();
            }
            println!("The bool has changed to true!");
        }

        fn set_service_available() {
            let (lock, cvar) = &*PAIR;
            let mut started = lock.lock().unwrap();
            *started = true;
            cvar.notify_one();
        }

        extern "C" fn my_msg_handler(_msg: &SharedPtr<vsomeip::message>) {
            TIMES_MESSAGE_RECEIVED.fetch_add(1, Ordering::SeqCst);
        }

        extern "C" fn publishing_service_availability_handler(
            _service: vsomeip::service_t,
            _instance: vsomeip::instance_t,
            _availability: bool,
        ) {
            println!("publishing service now available");
            set_service_available();
        }

        let test_duration = 10;

        let app_name_publisher = "publisher";
        let service_id = 0x3212;
        let instance_id = 1;
        let event_id = 0x34;
        let eventgroup_id = 0x34;

        let app_name_subscriber = "subscriber";

        let runtime_wrapper = make_runtime_wrapper(runtime::get());

        let_cxx_string!(app_name_publisher_cxx = app_name_publisher);
        let_cxx_string!(app_name_subscriber_cxx = app_name_subscriber);

        let app_name_publisher_start = app_name_publisher.to_string();
        thread::spawn(|| {
            let runtime_wrapper = make_runtime_wrapper(runtime::get());

            let_cxx_string!(app_name_cxx = app_name_publisher_start);

            let app_wrapper = make_application_wrapper(
                get_pinned_runtime(&runtime_wrapper).create_application(&app_name_cxx),
            );

            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.init();
            } else {
                panic!("Unable to init app");
            }
            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.start();
            } else {
                panic!("Unable to start app");
            }
        });

        let app_name_subscriber_start = app_name_subscriber.to_string();
        thread::spawn(|| {
            let runtime_wrapper = make_runtime_wrapper(runtime::get());

            let_cxx_string!(app_name_cxx = app_name_subscriber_start);

            let app_wrapper = make_application_wrapper(
                get_pinned_runtime(&runtime_wrapper).create_application(&app_name_cxx),
            );

            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.init();
            } else {
                panic!("Unable to init app");
            }
            if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
                pinned_app.start();
            } else {
                panic!("Unable to start app");
            }
        });

        thread::sleep(Duration::from_millis(500));

        let mut publisher_app_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_publisher_cxx),
        );
        let mut subscriber_app_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).get_application(&app_name_subscriber_cxx),
        );

        if let Some(pinned_app) = get_pinned_application(&subscriber_app_wrapper) {
            pinned_app.request_service(
                service_id,
                instance_id,
                vsomeip::ANY_MAJOR,
                vsomeip::ANY_MINOR,
            )
        }

        if let Some(pinned_app) = get_pinned_application(&subscriber_app_wrapper) {
            pinned_app.subscribe(
                service_id,
                instance_id,
                eventgroup_id,
                vsomeip::ANY_MAJOR,
                event_id,
            );
        } else {
            panic!("Application does not exist app_name: {app_name_subscriber}");
        }

        let my_callback = MessageHandlerFnPtr(my_msg_handler);

        register_message_handler_fn_ptr_safe(
            &mut subscriber_app_wrapper,
            service_id,
            instance_id,
            event_id,
            my_callback,
        );

        thread::sleep(Duration::from_millis(500));

        let pub_service_availability_handler =
            AvailabilityHandlerFnPtr(publishing_service_availability_handler);
        register_availability_handler_fn_ptr_safe(
            &mut publisher_app_wrapper,
            service_id,
            instance_id,
            pub_service_availability_handler,
            vsomeip::ANY_MAJOR,
            vsomeip::ANY_MINOR,
        );

        if let Some(pinned_app) = get_pinned_application(&publisher_app_wrapper) {
            pinned_app.offer_service(
                service_id,
                instance_id,
                vsomeip::ANY_MAJOR,
                vsomeip::ANY_MINOR,
            );
        } else {
            panic!("Unable to offer service");
        }

        offer_single_event_safe(
            &mut publisher_app_wrapper,
            service_id,
            instance_id,
            event_id,
            event_id,
        );

        wait_for_service_available();

        // Track the start time and set the duration for the loop
        let duration = Duration::from_millis(test_duration);
        let start_time = Instant::now();

        #[allow(unused_variables)]
        let mut iterations: usize = 0;
        while Instant::now().duration_since(start_time) < duration {
            if let Some(pinned_app) = get_pinned_application(&publisher_app_wrapper) {
                let vsomeip_payload =
                    make_payload_wrapper(get_pinned_runtime(&runtime_wrapper).create_payload());
                let payload = [1, 2, 3, 4];
                set_data_safe(get_pinned_payload(&vsomeip_payload), &payload);
                let attachable_payload = vsomeip_payload.get_shared_ptr();
                pinned_app.notify(service_id, instance_id, event_id, attachable_payload, true);
            }

            iterations += 1;
        }

        thread::sleep(Duration::from_millis(500));

        #[allow(unused_variables)]
        let times_message_received = TIMES_MESSAGE_RECEIVED.load(Ordering::SeqCst);

        // TODO: It seems like checking if the service is up is not enough.
        //  May unfortunately need to leave the sleep for now
        // assert_eq!(iterations, times_message_received);
    }
}
