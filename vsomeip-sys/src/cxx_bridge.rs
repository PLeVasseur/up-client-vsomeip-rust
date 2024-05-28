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

#[cxx::bridge]
pub mod bar {
    unsafe extern "C++" {
        include!("vsomeip/vsomeip.hpp");
        include!("include/application_wrapper.h");
        include!("application_registrations.h");

        type message_handler_fn_ptr = crate::extern_callback_wrappers::MessageHandlerFnPtr;
        type ApplicationWrapper = crate::ffi::ApplicationWrapper;

        pub unsafe fn register_message_handler_fn_ptr(
            _application: *mut ApplicationWrapper,
            _service: u16,
            _instance: u16,
            _method: u16,
            _fn_ptr_handler: message_handler_fn_ptr,
        );

        type availability_handler_fn_ptr = crate::extern_callback_wrappers::AvailabilityHandlerFnPtr;

        pub unsafe fn register_availability_handler_fn_ptr(
            _application: *mut ApplicationWrapper,
            _service: u16,
            _instance: u16,
            _fn_ptr_handler: availability_handler_fn_ptr,
            _major: u8,
            _minor: u32,
        );
    }
}