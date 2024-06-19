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

use crate::cxx_bridge::handler_registration::{
    offer_single_event, register_availability_handler_fn_ptr, register_message_handler_fn_ptr,
    register_subscription_status_handler_fn_ptr, request_single_event,
};
use crate::extern_callback_wrappers::{
    AvailabilityHandlerFnPtr, MessageHandlerFnPtr, SubscriptionStatusHandlerFnPtr,
};
use crate::ffi::glue::{get_payload_raw, set_payload_raw};
use crate::glue::upcast;
use crate::glue::{ApplicationWrapper, MessageWrapper, PayloadWrapper, RuntimeWrapper};
use crate::unsafe_fns::create_payload_wrapper;
use crate::vsomeip::message_base;
use crate::vsomeip::{application, message, payload, runtime};
use cxx::UniquePtr;
use log::trace;
use std::pin::Pin;
use std::slice;

/// Gets a `Pin<&mut runtime>` from a [RuntimeWrapper]
///
/// # Rationale
///
/// In order to use the wrapped methods on a [runtime], we must have a `Pin<&mut runtime>`
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use a UniquePtr<[RuntimeWrapper]>
///
/// Since we use a UniquePtr<[RuntimeWrapper]>, we then need a way to drill down and extract
/// the `Pin<&mut runtime>`.
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
pub fn get_pinned_runtime(wrapper: &RuntimeWrapper) -> Pin<&mut runtime> {
    unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
}

/// Gets a `Pin<&mut application>` from an [ApplicationWrapper]
///
/// # Rationale
///
/// In order to use the wrapped methods on an [application], we must have a `Pin<&mut application>`
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use a UniquePtr<[ApplicationWrapper]>
///
/// Since we use a UniquePtr<[ApplicationWrapper]>, we then need a way to drill down and extract
/// the `Pin<&mut application>`.
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
///
/// I do see a runtime panic here, perhaps when we try to work with the app before it's setup
/// should probably do a sleep of half a second or something
pub fn get_pinned_application(wrapper: &ApplicationWrapper) -> Pin<&mut application> {
    unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
}

/// Gets a `Pin<&mut message>` from a [MessageWrapper]
///
/// # Rationale
///
/// In order to use the wrapped methods on an [message], we must have a `Pin<&mut message>`
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use a UniquePtr<[MessageWrapper]>
///
/// Since we use a UniquePtr<[MessageWrapper]>, we then need a way to drill down and extract
/// the `Pin<&mut message>`.
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
pub fn get_pinned_message(wrapper: &MessageWrapper) -> Pin<&mut message> {
    unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
}

/// Gets a `Pin<&mut message_base>` from a [MessageWrapper]
///
/// # Rationale
///
/// In order to use the methods implemented on [message_base] which are inherited by [message],
/// we must explicitly upcast into a [message_base] and return a `Pin<&mut message_base>`
///
/// It appears like cxx may never handle the case of calling virtual methods of base classes,
/// so this is the workaround that works
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use a UniquePtr<[MessageWrapper]>
///
/// Since we use a UniquePtr<[MessageWrapper]>, we then need a way to drill down and extract
/// the `Pin<&mut message_base>`.
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
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

/// Gets a `Pin<&mut payload>` from a [PayloadWrapper]
///
/// # Rationale
///
/// In order to use the wrapped methods on an [payload], we must have a `Pin<&mut payload>`
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use a UniquePtr<[PayloadWrapper]>
///
/// Since we use a UniquePtr<[PayloadWrapper]>, we then need a way to drill down and extract
/// the `Pin<&mut payload>`.
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
pub fn get_pinned_payload(wrapper: &PayloadWrapper) -> Pin<&mut payload> {
    unsafe { Pin::new_unchecked(wrapper.get_mut().as_mut().unwrap()) }
}

/// Sets a vsomeip [payload]'s byte buffer
///
/// # Rationale
///
/// We expose a safe API which is idiomatic to Rust, passing in a slice of u8 bytes
///
/// First call [get_pinned_payload], then you may call this function next
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
pub fn set_data_safe(payload: Pin<&mut payload>, _data: &[u8]) {
    // Get the length of the data
    let length = _data.len() as u32;

    trace!("length of payload: {length}");

    // Get a pointer to the data
    let data_ptr = _data.as_ptr();

    trace!("data_ptr: {data_ptr:?}");

    unsafe {
        payload.set_data(data_ptr, length);
    }
}

/// Gets a vsomeip [payload]'s byte buffer
///
/// # Rationale
///
/// We expose a safe API which is idiomatic to Rust, returning a Vec of u8 bytes
///
/// # TODO
///
/// Add some runtime safety checks on the pointer
pub fn get_data_safe(payload_wrapper: &PayloadWrapper) -> Vec<u8> {
    let length = get_pinned_payload(payload_wrapper).get_length();
    let data_ptr = get_pinned_payload(payload_wrapper).get_data();

    trace!("get_data_safe: length: {length}");

    if data_ptr.is_null() {
        trace!("get_data_safe: data_ptr is null");
        return Vec::new();
    }
    trace!("get_data_safe: data_ptr is not null");

    trace!("Before slice::from_raw_parts");

    // Convert the raw pointer and length to a slice
    // TODO: May be crashing here? Would be good to check this
    let data_slice: &[u8] = unsafe { slice::from_raw_parts(data_ptr, length as usize) };

    trace!("After slice::from_raw_parts");

    // Convert the slice to a Vec
    let data_vec: Vec<u8> = data_slice.to_vec();

    trace!("after conversion to vec: {data_vec:?}");

    data_vec
}

/// Sets a vsomeip [message]'s [payload]
///
/// # Rationale
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use UniquePtr<[MessageWrapper]> and UniquePtr<[PayloadWrapper]>
///
/// We expose a safe API which handles the underlying details of pinning and calling unsafe functions
///
/// # TODO
///
/// Add some runtime safety checks on the pointers
pub fn set_message_payload(
    message_wrapper: &mut UniquePtr<MessageWrapper>,
    payload_wrapper: &mut UniquePtr<PayloadWrapper>,
) {
    unsafe {
        let message_pin = Pin::new_unchecked(&mut *message_wrapper);
        let payload_pin = Pin::new_unchecked(&mut *payload_wrapper);
        let message_ptr = MessageWrapper::get_mut(&message_pin);
        let payload_ptr = PayloadWrapper::get_mut(&payload_pin);
        set_payload_raw(message_ptr, payload_ptr);
    }
}

/// Gets a vsomeip [message]'s [payload]
///
/// # Rationale
///
/// Because vsomeip makes heavy use of std::shared_ptr and Rust's ownership and borrowing model
/// doesn't work well together with it, in Rust code use UniquePtr<[MessageWrapper]> and UniquePtr<[PayloadWrapper]>
///
/// We expose a safe API which handles the underlying details of pinning and calling unsafe functions
///
/// # TODO
///
/// Add some runtime safety checks on the pointers
pub fn get_message_payload(
    message_wrapper: &mut UniquePtr<MessageWrapper>,
) -> UniquePtr<PayloadWrapper> {
    unsafe {
        if message_wrapper.is_null() {
            eprintln!("message_wrapper is null");
            return cxx::UniquePtr::null();
        }

        let message_pin = Pin::new_unchecked(message_wrapper.as_mut().unwrap());
        let message_ptr = MessageWrapper::get_mut(&message_pin) as *const message;

        // TODO: Should handle by returning None instead of a null pointer
        if (message_ptr as *const ()).is_null() {
            eprintln!("message_ptr is null");
            return UniquePtr::null();
        }

        let payload_ptr = get_payload_raw(message_ptr);

        if (payload_ptr as *const ()).is_null() {
            eprintln!("payload_ptr is null");
            return UniquePtr::null();
        }

        // Use the intermediate function to create a UniquePtr<PayloadWrapper>
        let payload_wrapper = create_payload_wrapper(payload_ptr);

        if payload_wrapper.is_null() {
            eprintln!("Failed to create UniquePtr<PayloadWrapper>");
        } else {
            trace!("Successfully created UniquePtr<PayloadWrapper>");
        }

        payload_wrapper
    }
}

/// Requests a single [eventgroup_t][crate::vsomeip::eventgroup_t] for the application
///
/// # Rationale
///
/// autocxx and cxx cannot generate bindings to a C++ function which contains
/// a templated std::set
///
/// We also have agreed to have a 1:1 mapping between eventgroup_t and std::set<eventgroup_t>
/// so this functio will be fine for now
///
/// If this changes in the future, we can instead create a wrapper called an EventGroup which will
/// have a single method on it to add a single [eventgroup_t][crate::vsomeip::eventgroup_t] so that we're able to have more than
/// one [eventgroup_t][crate::vsomeip::eventgroup_t]
pub fn request_single_event_safe(
    application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    _service: u16,
    _instance: u16,
    _notifier: u16,
    _eventgroup: u16,
) {
    unsafe {
        let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
        request_single_event(
            application_wrapper_ptr,
            _service,
            _instance,
            _notifier,
            _eventgroup,
        );
    }
}

/// Offers a single [eventgroup_t][crate::vsomeip::eventgroup_t] from the application
///
/// # Rationale
///
/// autocxx and cxx cannot generate bindings to a C++ function which contains
/// a templated std::set
///
/// We also have agreed to have a 1:1 mapping between eventgroup_t and std::set<eventgroup_t>
/// so this function will be fine for now
///
/// If this changes in the future, we can instead create a wrapper called an EventGroup which will
/// have a single method on it to add a single [eventgroup_t][crate::vsomeip::eventgroup_t] so that we're able to have more than
/// one [eventgroup_t][crate::vsomeip::eventgroup_t]
pub fn offer_single_event_safe(
    application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    _service: u16,
    _instance: u16,
    _notifier: u16,
    _eventgroup: u16,
) {
    unsafe {
        let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
        offer_single_event(
            application_wrapper_ptr,
            _service,
            _instance,
            _notifier,
            _eventgroup,
        );
    }
}

/// Registers a [MessageHandlerFnPtr] with a vsomeip [application]
///
/// # Rationale
///
/// autocxx fails to generate bindings to application::register_message_handler()
/// due to its signature containing a std::function
///
/// Therefore, we have this function which will call the glue C++ register_message_handler_fn_ptr
/// reference there for more details
///
/// # TODO
///
/// Add some runtime safety checks on the pointers
pub fn register_message_handler_fn_ptr_safe(
    application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    _service: u16,
    _instance: u16,
    _method: u16,
    _fn_ptr_handler: MessageHandlerFnPtr,
) {
    unsafe {
        let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
        register_message_handler_fn_ptr(
            application_wrapper_ptr,
            _service,
            _instance,
            _method,
            _fn_ptr_handler,
        );
    }
}

/// Registers an [AvailabilityHandlerFnPtr] with a vsomeip [application]
///
/// # Rationale
///
/// autocxx fails to generate bindings to application::register_availability_handler()
/// due to its signature containing a std::function
///
/// Therefore, we have this function which will call the glue C++ register_availability_handler_fn_ptr
///
/// # TODO
///
/// Add some runtime safety checks on the pointers
pub fn register_availability_handler_fn_ptr_safe(
    application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    _service: u16,
    _instance: u16,
    _fn_ptr_handler: AvailabilityHandlerFnPtr,
    _major_version: u8,
    _minor_version: u32,
) {
    unsafe {
        let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
        register_availability_handler_fn_ptr(
            application_wrapper_ptr,
            _service,
            _instance,
            _fn_ptr_handler,
            _major_version,
            _minor_version,
        );
    }
}

/// Registers a [SubscriptionStatusHandlerFnPtr] with a vsomeip [application]
///
/// # Rationale
///
/// autocxx fails to generate bindings to application::register_subscription_status_handler()
/// due to its signature containing a std::function
///
/// Therefore, we have this function which will call the glue C++ register_subscription_status_handler_fn_ptr
///
/// # TODO
///
/// Add some runtime safety checks on the pointers
pub fn register_subscription_status_handler_fn_ptr_safe(
    application_wrapper: &mut UniquePtr<ApplicationWrapper>,
    _service: u16,
    _instance: u16,
    _eventgroup: u16,
    _event: u16,
    _fn_ptr_handler: SubscriptionStatusHandlerFnPtr,
    _is_selective: bool,
) {
    unsafe {
        let application_wrapper_ptr = application_wrapper.pin_mut().get_self();
        register_subscription_status_handler_fn_ptr(
            application_wrapper_ptr,
            _service,
            _instance,
            _eventgroup,
            _event,
            _fn_ptr_handler,
            _is_selective,
        );
    }
}
