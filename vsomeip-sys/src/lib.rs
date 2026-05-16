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

#![allow(unused_attributes)]

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
    generate!("vsomeip_v3::DEFAULT_MAJOR")
    generate!("vsomeip_v3::DEFAULT_MINOR")
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

mod extern_callback_wrappers;
mod glue_additions;
mod unsafe_fns {
    pub use crate::ffi::glue::create_payload_wrapper;
    pub use crate::ffi::glue::upcast;
}

pub mod vsomeip {
    pub use crate::ffi::vsomeip_v3::*;
}

pub mod glue {
    pub use crate::extern_callback_wrappers::*;
    pub use crate::ffi::glue::{
        make_message_wrapper, make_payload_wrapper, make_runtime_wrapper, ApplicationWrapper,
        MessageWrapper, PayloadWrapper, RuntimeWrapper,
    };
    pub use crate::glue_additions::make_application_wrapper;
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
    use crate::vsomeip;
    use crate::vsomeip::state_type_e;
    use cxx::{let_cxx_string, SharedPtr};
    use lazy_static::lazy_static;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{mpsc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    static NEXT_NETWORK_ID: AtomicUsize = AtomicUsize::new(1);

    struct VsomeipTestNetwork {
        name: String,
    }

    struct IsolatedVsomeipConfig {
        _dir: TempDir,
        path: PathBuf,
    }

    impl VsomeipTestNetwork {
        fn new(test_name: &str) -> Self {
            let id = NEXT_NETWORK_ID.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            Self {
                name: format!("vsys_{pid}_{id}_{test_name}"),
            }
        }

        fn config_with_applications(&self, applications: &[(&str, u16)]) -> IsolatedVsomeipConfig {
            let dir = TempDir::new().expect("failed to create temporary vSomeIP config directory");
            let path = dir.path().join("vsomeip.json");
            let applications = applications
                .iter()
                .map(|(name, id)| format!(r#"{{ "name": "{}", "id": "0x{:04x}" }}"#, name, id))
                .collect::<Vec<_>>()
                .join(", ");
            let config = format!(
                r#"{{
  "unicast": "127.0.0.1",
  "network": "{}",
  "applications": [{}]
}}
"#,
                self.name, applications
            );
            fs::write(&path, config).expect("failed to write temporary vSomeIP config");
            IsolatedVsomeipConfig { _dir: dir, path }
        }
    }

    impl IsolatedVsomeipConfig {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    lazy_static! {
        static ref RAW_VSOMEIP_TEST_LOCK: Mutex<()> = Mutex::new(());
        static ref RAW_VSOMEIP_CONFIG: IsolatedVsomeipConfig =
            VsomeipTestNetwork::new("raw_vsomeip").config_with_applications(&[
                ("check_available_app", 0x7102),
                ("publisher", 0x7103),
                ("subscriber", 0x7104),
            ]);
    }

    #[test]
    fn test_make_runtime() {
        let my_runtime = runtime::get();
        let runtime_wrapper = make_runtime_wrapper(my_runtime);
        let request = make_message_wrapper(runtime_wrapper.get_pinned().create_request(true));

        let reliable = (*request).get_message_base_pinned().is_reliable();

        println!("reliable? {reliable}");

        let request = make_message_wrapper(runtime_wrapper.get_pinned().create_request(true));
        (*request).get_message_base_pinned().set_service(1);
        (*request).get_message_base_pinned().set_instance(2);
        (*request).get_message_base_pinned().set_method(3);

        let mut payload_wrapper =
            make_payload_wrapper(runtime_wrapper.get_pinned().create_payload());
        let _foo = payload_wrapper.get_pinned();

        let data: Vec<u8> = vec![1, 2, 3, 4, 5];

        (*payload_wrapper).set_data_safe(&data);

        let data_vec = (*payload_wrapper).get_data_safe();
        println!("{:?}", data_vec);

        (*request).set_message_payload(&mut payload_wrapper);

        println!("set_message_payload");

        let Some(loaded_payload) = (*request).get_message_payload() else {
            panic!("Unable to get PayloadWrapper from MessageWrapper");
        };

        println!("get_message_payload");

        let loaded_data_vec = loaded_payload.get_data_safe();

        println!("loaded_data_vec: {loaded_data_vec:?}");

        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn test_available_state_handler() {
        let _guard = RAW_VSOMEIP_TEST_LOCK.lock().unwrap();
        lazy_static! {
            static ref SENDER: Mutex<Option<Sender<state_type_e>>> = Mutex::new(None);
        }

        extern "C" fn available_state_handler(available_state: state_type_e) {
            println!("available_state: {available_state:?}");

            if let Some(ref tx) = *SENDER.lock().unwrap() {
                let _ = tx.send(available_state);
            }
        }

        let (tx, rx) = mpsc::channel();
        *SENDER.lock().unwrap() = Some(tx);

        let config_path = RAW_VSOMEIP_CONFIG.path().display().to_string();
        let handle = thread::spawn(move || {
            let my_runtime = runtime::get();
            let runtime_wrapper = make_runtime_wrapper(my_runtime);

            let_cxx_string!(app_name_cxx = "check_available_app");
            let_cxx_string!(config_path_cxx = config_path);
            let Some(app_wrapper) = make_application_wrapper(
                runtime_wrapper
                    .get_pinned()
                    .create_application1(&app_name_cxx, &config_path_cxx),
            ) else {
                panic!("Unable to create application");
            };
            app_wrapper.get_pinned().init();
            let state_handler = AvailableStateHandlerFnPtr(available_state_handler);
            app_wrapper.register_state_handler_fn_ptr_safe(state_handler);
            app_wrapper.get_pinned().start();
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let state = rx
                .recv_timeout(remaining)
                .expect("timed out waiting for vSomeIP registration state");
            if state == state_type_e::ST_REGISTERED {
                break;
            }
        }

        let runtime_wrapper = make_runtime_wrapper(runtime::get());
        let_cxx_string!(app_name_cxx = "check_available_app");
        let Some(app_wrapper) =
            make_application_wrapper(runtime_wrapper.get_pinned().get_application(&app_name_cxx))
        else {
            panic!("Application not started yet");
        };
        app_wrapper.get_pinned().stop();
        handle.join().expect("vSomeIP application thread panicked");
        *SENDER.lock().unwrap() = None;
    }

    #[test]
    fn test_service_availability_handler() {
        let _guard = RAW_VSOMEIP_TEST_LOCK.lock().unwrap();
        lazy_static! {
            static ref TIMES_MESSAGE_RECEIVED: AtomicUsize = AtomicUsize::new(0);
            static ref PAIR: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
        }

        fn wait_for_service_available() -> bool {
            let (lock, cvar) = &*PAIR;
            let mut started = lock.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !*started && Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let result = cvar.wait_timeout(started, remaining).unwrap();
                started = result.0;
            }
            *started
        }

        fn wait_for_messages(baseline: usize) -> bool {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if TIMES_MESSAGE_RECEIVED.load(Ordering::SeqCst) > baseline {
                    return true;
                }
                thread::sleep(Duration::from_millis(25));
            }
            false
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
            availability: bool,
        ) {
            if availability {
                println!("publishing service now available");
                set_service_available();
            }
        }

        TIMES_MESSAGE_RECEIVED.store(0, Ordering::SeqCst);
        *PAIR.0.lock().unwrap() = false;

        let app_name_publisher = "publisher";
        let service_id = 0x3212;
        let instance_id = 1;
        let event_id = 0x34;
        let eventgroup_id = 0x34;

        let app_name_subscriber = "subscriber";

        let runtime_wrapper = make_runtime_wrapper(runtime::get());

        let_cxx_string!(app_name_publisher_cxx = app_name_publisher);
        let_cxx_string!(app_name_subscriber_cxx = app_name_subscriber);

        let publisher_config_path = RAW_VSOMEIP_CONFIG.path().display().to_string();
        let publisher_handle = thread::spawn(move || {
            let runtime_wrapper = make_runtime_wrapper(runtime::get());

            let_cxx_string!(app_name_cxx = "publisher");
            let_cxx_string!(config_path_cxx = publisher_config_path);

            let Some(app_wrapper) = make_application_wrapper(
                runtime_wrapper
                    .get_pinned()
                    .create_application1(&app_name_cxx, &config_path_cxx),
            ) else {
                panic!("Unable to init app");
            };

            app_wrapper.get_pinned().init();
            app_wrapper.get_pinned().start();
        });

        let subscriber_config_path = RAW_VSOMEIP_CONFIG.path().display().to_string();
        let subscriber_handle = thread::spawn(move || {
            let runtime_wrapper = make_runtime_wrapper(runtime::get());

            let_cxx_string!(app_name_cxx = "subscriber");
            let_cxx_string!(config_path_cxx = subscriber_config_path);

            let Some(app_wrapper) = make_application_wrapper(
                runtime_wrapper
                    .get_pinned()
                    .create_application1(&app_name_cxx, &config_path_cxx),
            ) else {
                panic!("Unable to init app");
            };
            app_wrapper.get_pinned().init();
            app_wrapper.get_pinned().start();
        });

        thread::sleep(Duration::from_millis(500));

        let Some(publisher_app_wrapper) = make_application_wrapper(
            runtime_wrapper
                .get_pinned()
                .get_application(&app_name_publisher_cxx),
        ) else {
            panic!("Application does not exist app_name: {app_name_publisher}");
        };
        let Some(subscriber_app_wrapper) = make_application_wrapper(
            runtime_wrapper
                .get_pinned()
                .get_application(&app_name_subscriber_cxx),
        ) else {
            panic!("Application does not exist app_name: {app_name_subscriber}");
        };

        subscriber_app_wrapper.get_pinned().request_service(
            service_id,
            instance_id,
            vsomeip::ANY_MAJOR,
            vsomeip::ANY_MINOR,
        );

        subscriber_app_wrapper.get_pinned().subscribe(
            service_id,
            instance_id,
            eventgroup_id,
            vsomeip::ANY_MAJOR,
            event_id,
        );

        let my_callback = MessageHandlerFnPtr(my_msg_handler);

        subscriber_app_wrapper.register_message_handler_fn_ptr_safe(
            service_id,
            instance_id,
            event_id,
            my_callback,
        );

        thread::sleep(Duration::from_millis(500));

        let pub_service_availability_handler =
            AvailabilityHandlerFnPtr(publishing_service_availability_handler);
        subscriber_app_wrapper.register_availability_handler_fn_ptr_safe(
            service_id,
            instance_id,
            pub_service_availability_handler,
            vsomeip::ANY_MAJOR,
            vsomeip::ANY_MINOR,
        );

        publisher_app_wrapper.get_pinned().offer_service(
            service_id,
            instance_id,
            vsomeip::ANY_MAJOR,
            vsomeip::ANY_MINOR,
        );

        publisher_app_wrapper.offer_single_event_safe(service_id, instance_id, event_id, event_id);

        assert!(
            wait_for_service_available(),
            "timed out waiting for publishing service availability"
        );

        let baseline_received = TIMES_MESSAGE_RECEIVED.load(Ordering::SeqCst);
        for _ in 0..10 {
            let vsomeip_payload =
                make_payload_wrapper(runtime_wrapper.get_pinned().create_payload());
            let payload = [1, 2, 3, 4];
            vsomeip_payload.set_data_safe(&payload);
            let attachable_payload = vsomeip_payload.get_shared_ptr();
            publisher_app_wrapper.get_pinned().notify(
                service_id,
                instance_id,
                event_id,
                attachable_payload,
                true,
            );
        }

        assert!(
            wait_for_messages(baseline_received),
            "timed out waiting for a vSomeIP event message"
        );

        publisher_app_wrapper.get_pinned().stop();
        subscriber_app_wrapper.get_pinned().stop();
        publisher_handle.join().expect("publisher thread panicked");
        subscriber_handle
            .join()
            .expect("subscriber thread panicked");
    }
}
