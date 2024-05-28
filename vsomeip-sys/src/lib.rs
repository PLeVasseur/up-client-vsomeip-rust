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

use autocxx::prelude::*; // use all the main autocxx functions

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
    generate!("vsomeip_v3::ANY_MAJOR")
    generate!("vsomeip_v3::ANY_MINOR")
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

// autocxx fails to generate bindings to these functions, so we write the bindings for them
// by hand and inject them into the vsomeip_v3 namespace
#[cxx::bridge(namespace = "vsomeip_v3")]
mod autocxx_failed {
    unsafe extern "C++" {
        include!("vsomeip/vsomeip.hpp");

        type payload = crate::vsomeip::payload;
        pub unsafe fn set_data(self: Pin<&mut payload>, _data: *const u8, _length: u32);

        pub fn get_data(self: &payload) -> *const u8;

        pub fn get_length(self: &payload) -> u32;
    }
}

// wrappers for the extern "C" fns we need to provide to vsomeip
pub mod extern_callback_wrappers {
    use cxx::{type_id, ExternType, SharedPtr};
    use crate::vsomeip;

    #[repr(transparent)]
    pub struct AvailabilityHandlerFnPtr(
        pub  extern "C" fn(
            service: crate::ffi::vsomeip_v3::service_t,
            instance: crate::ffi::vsomeip_v3::instance_t,
            availability: bool,
        ),
    );

    unsafe impl ExternType for AvailabilityHandlerFnPtr {
        type Id = type_id!("glue::availability_handler_fn_ptr");
        type Kind = cxx::kind::Trivial;
    }

    #[repr(transparent)]
    pub struct MessageHandlerFnPtr(pub extern "C" fn(&SharedPtr<vsomeip::message>));

    unsafe impl ExternType for MessageHandlerFnPtr {
        type Id = type_id!("glue::message_handler_fn_ptr");
        type Kind = cxx::kind::Trivial;
    }
}

pub mod vsomeip {
    pub use crate::ffi::vsomeip_v3::*;
}

pub mod safe_glue {
    use crate::glue::upcast;
    use crate::glue::{
        create_payload_wrapper, make_application_wrapper, make_message_wrapper,
        make_payload_wrapper, make_runtime_wrapper, ApplicationWrapper, MessageWrapper,
        PayloadWrapper, RuntimeWrapper,
    };
    use crate::vsomeip::{application, message, payload, runtime};
    use crate::glue::{get_payload_raw, set_payload_raw};
    use crate::vsomeip::{instance_t, message_base, service_t};
    use crate::extern_callback_wrappers::{AvailabilityHandlerFnPtr, MessageHandlerFnPtr};
    use cxx::{SharedPtr, UniquePtr};
    use lazy_static::lazy_static;
    use std::cell::UnsafeCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::pin::Pin;
    use std::slice;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, Once};
    use crate::cxx_bridge::bar::register_message_handler_fn_ptr;
    use crate::cxx_bridge::bar::register_availability_handler_fn_ptr;

    pub fn get_pinned_runtime(wrapper: &RuntimeWrapper) -> Pin<&mut runtime> {
        unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
    }

    pub fn get_pinned_application(wrapper: &ApplicationWrapper) -> Pin<&mut application> {
        unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
    }

    pub fn get_pinned_message(wrapper: &MessageWrapper) -> Pin<&mut message> {
        unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
    }

    pub fn get_message(wrapper: &MessageWrapper) -> Pin<&mut message> {
        unsafe {
            let msg_ptr: *mut message = wrapper.get_mut();
            if msg_ptr.is_null() {
                panic!("msg_ptr is null");
            }
            Pin::new_unchecked(msg_ptr.as_mut().unwrap())
        }
    }

    pub fn get_pinned_message_base(wrapper: &MessageWrapper) -> Pin<&mut message_base> {
        unsafe {
            let msg_ptr: *mut message = wrapper.get_mut();
            if msg_ptr.is_null() {
                panic!("msg_ptr is null");
            }

            // Convert the raw pointer to a mutable reference
            let msg_ref: &mut message = &mut *msg_ptr;

            // Pin the mutable reference
            let pinned_msg_ref: Pin<&mut message> = Pin::new_unchecked(msg_ref);

            // Use the upcast function to get a pinned mutable reference to message_base
            let pinned_base_ref: Pin<&mut message_base> = upcast(pinned_msg_ref);

            pinned_base_ref
        }
    }

    pub fn get_pinned_payload(wrapper: &PayloadWrapper) -> Pin<&mut payload> {
        unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
    }

    pub fn set_data_safe(payload: Pin<&mut payload>, _data: Box<[u8]>) {
        // Get the length of the data
        let length = _data.len() as u32;

        // Get a pointer to the data
        let data_ptr = _data.as_ptr();

        unsafe {
            payload.set_data(data_ptr, length);
        }
    }

    pub fn get_data_safe(payload_wrapper: &PayloadWrapper) -> Vec<u8> {
        let length = get_pinned_payload(&payload_wrapper).get_length();
        let data_ptr = get_pinned_payload(&payload_wrapper).get_data();

        // Convert the raw pointer and length to a slice
        let data_slice: &[u8] = unsafe { slice::from_raw_parts(data_ptr, length as usize) };

        // Convert the slice to a Vec
        let data_vec: Vec<u8> = data_slice.to_vec();

        data_vec
    }

    pub fn set_message_payload(
        message_wrapper: &mut UniquePtr<MessageWrapper>,
        payload_wrapper: &mut UniquePtr<PayloadWrapper>,
    ) {
        unsafe {
            let message_pin = Pin::new_unchecked(&mut *message_wrapper);
            let payload_pin = Pin::new_unchecked(&mut *payload_wrapper);
            let message_ptr = MessageWrapper::get_mut(&**message_pin);
            let payload_ptr = PayloadWrapper::get_mut(&**payload_pin);
            set_payload_raw(message_ptr, payload_ptr);
        }
    }

    pub fn get_message_payload(
        message_wrapper: &mut UniquePtr<MessageWrapper>,
    ) -> UniquePtr<PayloadWrapper> {
        unsafe {
            if message_wrapper.is_null() {
                eprintln!("message_wrapper is null");
                return cxx::UniquePtr::null();
            }

            let message_pin = Pin::new_unchecked(message_wrapper.as_mut().unwrap());
            let message_ptr = MessageWrapper::get_mut(&*message_pin) as *const message;

            if (message_ptr as *const ()).is_null() {
                eprintln!("message_ptr is null");
                return UniquePtr::null();
            }

            let payload_ptr = get_payload_raw(message_ptr);

            if (payload_ptr as *const ()).is_null() {
                eprintln!("payload_ptr is null");
                return UniquePtr::null();
            }

            println!("get_message_payload: payload_ptr = {:?}", payload_ptr);

            // Use the intermediate function to create a UniquePtr<PayloadWrapper>
            let payload_wrapper = create_payload_wrapper(payload_ptr);

            if payload_wrapper.is_null() {
                eprintln!("Failed to create UniquePtr<PayloadWrapper>");
            } else {
                println!("Successfully created UniquePtr<PayloadWrapper>");
            }

            payload_wrapper
        }
    }

    pub fn register_message_handler_fn_ptr_safe(
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        _service: u16,
        _instance: u16,
        _method: u16,
        _fn_ptr_handler: MessageHandlerFnPtr,
    ) {
        unsafe {
            let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
            register_message_handler_fn_ptr(application_wrapper_ptr, _service, _instance, _method, _fn_ptr_handler);
        }
    }

    pub fn register_availability_handler_fn_ptr_safe(
        application_wrapper: &mut UniquePtr<ApplicationWrapper>,
        _service: u16,
        _instance: u16,
        _fn_ptr_handler: AvailabilityHandlerFnPtr,
        _major_version: u8,
        _minor_version: u32
    ) {
        unsafe {
            let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
            register_availability_handler_fn_ptr(application_wrapper_ptr, _service, _instance, _fn_ptr_handler, _major_version, _minor_version);
        }
    }
}

pub mod glue {
    pub use crate::ffi::glue::upcast;
    pub use crate::ffi::glue::{
        create_payload_wrapper, make_application_wrapper, make_message_wrapper,
        make_payload_wrapper, make_runtime_wrapper, ApplicationWrapper, MessageWrapper,
        PayloadWrapper, RuntimeWrapper,
    };
    pub use crate::ffi::glue::{get_payload_raw, set_payload_raw};
}

#[cfg(test)]
mod tests {
    use crate::ffi::vsomeip_v3::runtime;
    use crate::glue::{
        make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
    };
    use crate::safe_glue::{get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base, get_pinned_payload, get_pinned_runtime, register_availability_handler_fn_ptr_safe, set_data_safe, set_message_payload};
    use crate::vsomeip::{message, message_base};
    use crate::extern_callback_wrappers::AvailabilityHandlerFnPtr;
    use cxx::let_cxx_string;
    use std::pin::Pin;
    use std::slice;
    use std::time::Duration;

    #[test]
    fn test_make_runtime() {
        let my_runtime = runtime::get();
        let runtime_wrapper = make_runtime_wrapper(my_runtime);

        let_cxx_string!(my_app_str = "my_app");
        let mut app_wrapper = make_application_wrapper(
            get_pinned_runtime(&runtime_wrapper).create_application(&my_app_str),
        );
        get_pinned_application(&app_wrapper).init();

        extern "C" fn callback(
            service: crate::vsomeip::service_t,
            instance: crate::vsomeip::instance_t,
            availability: bool,
        ) {
            println!("hello from Rust!");
        }
        let callback = AvailabilityHandlerFnPtr(callback);
        register_availability_handler_fn_ptr_safe(&mut app_wrapper,
                                                  1,
                                                  2,
                                                  callback,
                                                  3,
                                                  4,
        );
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
        let foo = get_pinned_payload(&payload_wrapper);

        let data: Vec<u8> = vec![1, 2, 3, 4, 5];

        set_data_safe(get_pinned_payload(&payload_wrapper), Box::from(data));

        let data_vec = get_data_safe(&payload_wrapper);
        println!("{:?}", data_vec);

        set_message_payload(&mut request, &mut payload_wrapper);

        println!("set_message_payload");

        let loaded_payload = get_message_payload(&mut request);

        println!("get_message_payload");

        let loaded_data_vec = get_data_safe(&loaded_payload);

        println!("loaded_data_vec: {loaded_data_vec:?}");

        std::thread::sleep(Duration::from_millis(2000));
    }
}
