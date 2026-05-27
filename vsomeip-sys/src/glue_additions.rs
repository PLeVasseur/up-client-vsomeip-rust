use crate::cxx_bridge::handler_registration::{
    offer_single_event, register_availability_handler_fn_ptr, register_message_handler_fn_ptr,
    register_state_handler_fn_ptr, register_subscription_status_handler_fn_ptr,
    request_single_event,
};
use crate::extern_callback_wrappers::{
    AvailabilityHandlerFnPtr, AvailableStateHandlerFnPtr, MessageHandlerFnPtr,
    SubscriptionStatusHandlerFnPtr,
};
use crate::ffi::glue::make_application_wrapper as make_application_wrapper_possible;
use crate::ffi::glue::{get_payload_raw, set_payload_raw};
use crate::unsafe_fns::create_payload_wrapper;
use crate::unsafe_fns::upcast;
use crate::{glue, vsomeip};
use cxx::{SharedPtr, UniquePtr};
use glue::{ApplicationWrapper, MessageWrapper, PayloadWrapper, RuntimeWrapper};
use log::{error, trace};
use std::pin::Pin;
use std::slice;
use vsomeip::message;

pub fn make_application_wrapper(
    app_shared_ptr: SharedPtr<vsomeip::application>,
) -> Option<UniquePtr<ApplicationWrapper>> {
    if app_shared_ptr.is_null() {
        return None;
    }

    Some(make_application_wrapper_possible(app_shared_ptr))
}

impl RuntimeWrapper {
    /// Gets a `Pin<&mut runtime>` from a [RuntimeWrapper]
    ///
    /// # Rationale
    ///
    /// In order to use the wrapped methods on a [vsomeip::runtime], we must have a `Pin<&mut runtime>`
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
    #[allow(clippy::mut_from_ref)]
    pub fn get_pinned(&self) -> Pin<&mut vsomeip::runtime> {
        // SAFETY:
        // - The wrapper owns a `UniquePtr` to a C++ runtime object; `get_mut`
        //   returns the mutable Rust view of that pointee when present.
        let runtime = unsafe { self.get_mut().as_mut().unwrap() };
        // SAFETY:
        // - The C++ object is not moved while the returned `Pin<&mut runtime>`
        //   is alive; this is the bridge invariant required by `Pin`.
        // - Per https://doc.rust-lang.org/stable/std/pin/struct.Pin.html#method.new_unchecked,
        //   callers must guarantee "the value's data will not be moved nor have
        //   its storage invalidated until it gets dropped."
        unsafe { Pin::new_unchecked(runtime) }
    }
}

impl ApplicationWrapper {
    /// Gets a `Pin<&mut application>` from an [ApplicationWrapper]
    ///
    /// # Rationale
    ///
    /// In order to use the wrapped methods on an [vsomeip::application], we must have a `Pin<&mut application>`
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
    /// Reorganize this such that we check for existence before calling .unwrap() and if
    /// it's None, then return None. Requires changing the API to Option<Pin<&mut application>>
    ///
    /// I do see a runtime panic here, perhaps when we try to work with the app before it's setup
    /// should probably do a sleep of half a second or something
    #[allow(clippy::mut_from_ref)]
    pub fn get_pinned(&self) -> Pin<&mut vsomeip::application> {
        // SAFETY:
        // - The wrapper owns a `UniquePtr` to a C++ application object;
        //   `get_mut` returns the mutable Rust view of that pointee when present.
        let application = unsafe { self.get_mut().as_mut().unwrap() };
        // SAFETY:
        // - The C++ object is not moved while the returned `Pin<&mut
        //   application>` is alive; this is the bridge invariant required by
        //   `Pin`.
        // - Per https://doc.rust-lang.org/stable/std/pin/struct.Pin.html#method.new_unchecked,
        //   callers must guarantee "the value's data will not be moved nor have
        //   its storage invalidated until it gets dropped."
        unsafe { Pin::new_unchecked(application) }
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
        &self,
        service: u16,
        instance: u16,
        notifier: u16,
        eventgroup: u16,
    ) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for the duration of this call.
        // - External C++ contract: the glue function does not store the wrapper
        //   pointer beyond the call and only forwards POD service IDs.
        unsafe {
            request_single_event(
                application_wrapper_ptr,
                service,
                instance,
                notifier,
                eventgroup,
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
        &self,
        service: u16,
        instance: u16,
        notifier: u16,
        eventgroup: u16,
    ) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for the duration of this call.
        // - External C++ contract: the glue function does not store the wrapper
        //   pointer beyond the call and only forwards POD service IDs.
        unsafe {
            offer_single_event(
                application_wrapper_ptr,
                service,
                instance,
                notifier,
                eventgroup,
            );
        }
    }

    /// Registers a [MessageHandlerFnPtr] with a vsomeip [vsomeip::application]
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
        &self,
        service: u16,
        instance: u16,
        method: u16,
        fn_ptr_handler: MessageHandlerFnPtr,
    ) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for registration.
        // - External C++ contract: vsomeip may store and invoke the callback on
        //   runtime threads; the function pointer uses the declared extern "C"
        //   ABI and must remain valid for the application lifetime.
        unsafe {
            register_message_handler_fn_ptr(
                application_wrapper_ptr,
                service,
                instance,
                method,
                fn_ptr_handler,
            );
        }
    }

    /// Registers an [AvailabilityHandlerFnPtr] with a vsomeip [vsomeip::application]
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
        &self,
        service: u16,
        instance: u16,
        fn_ptr_handler: AvailabilityHandlerFnPtr,
        major_version: u8,
        minor_version: u32,
    ) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for registration.
        // - External C++ contract: vsomeip may store and invoke the callback on
        //   runtime threads; the function pointer uses the declared extern "C"
        //   ABI and must remain valid for the application lifetime.
        unsafe {
            register_availability_handler_fn_ptr(
                application_wrapper_ptr,
                service,
                instance,
                fn_ptr_handler,
                major_version,
                minor_version,
            );
        }
    }

    /// Registers a [SubscriptionStatusHandlerFnPtr] with a vsomeip [vsomeip::application]
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
        &self,
        service: u16,
        instance: u16,
        eventgroup: u16,
        event: u16,
        fn_ptr_handler: SubscriptionStatusHandlerFnPtr,
        is_selective: bool,
    ) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for registration.
        // - External C++ contract: vsomeip may store and invoke the callback on
        //   runtime threads; the function pointer uses the declared extern "C"
        //   ABI and must remain valid for the application lifetime.
        unsafe {
            register_subscription_status_handler_fn_ptr(
                application_wrapper_ptr,
                service,
                instance,
                eventgroup,
                event,
                fn_ptr_handler,
                is_selective,
            );
        }
    }

    pub fn register_state_handler_fn_ptr_safe(&self, fn_ptr_handler: AvailableStateHandlerFnPtr) {
        let application_wrapper_ptr = self as *const ApplicationWrapper as *mut ApplicationWrapper;
        // SAFETY:
        // - `application_wrapper_ptr` is derived from `self` and remains valid
        //   for registration.
        // - External C++ contract: vsomeip may store and invoke the callback on
        //   runtime threads; the function pointer uses the declared extern "C"
        //   ABI and must remain valid for the application lifetime.
        unsafe {
            register_state_handler_fn_ptr(application_wrapper_ptr, fn_ptr_handler);
        }
    }
}

impl MessageWrapper {
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
    #[allow(clippy::mut_from_ref)]
    pub fn get_pinned(&self) -> Pin<&mut vsomeip::message> {
        // SAFETY: `get_mut` returns the C++ message pointee managed by this
        // wrapper when present.
        let message = unsafe { self.get_mut().as_mut().unwrap() };
        // SAFETY:
        // - The C++ message object is not moved while the returned pin is alive.
        // - Per https://doc.rust-lang.org/stable/std/pin/struct.Pin.html#method.new_unchecked,
        //   callers must guarantee "the value's data will not be moved nor have
        //   its storage invalidated until it gets dropped."
        unsafe { Pin::new_unchecked(message) }
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
    #[allow(clippy::mut_from_ref)]
    pub fn get_message_base_pinned(&self) -> Pin<&mut vsomeip::message_base> {
        let msg_ptr: *mut message = self.get_mut();
        if msg_ptr.is_null() {
            panic!("msg_ptr is null");
        }

        // SAFETY:
        // - `msg_ptr` was returned from this wrapper and checked for null.
        // - The wrapper owns the mutable access path for the duration of the
        //   returned pin.
        let msg_ref: &mut message = unsafe { &mut *msg_ptr };

        // SAFETY:
        // - The C++ message object is not moved while the returned pin is alive.
        // - Per https://doc.rust-lang.org/stable/std/pin/struct.Pin.html#method.new_unchecked,
        //   callers must guarantee "the value's data will not be moved nor have
        //   its storage invalidated until it gets dropped."
        let pinned_msg_ref: Pin<&mut message> = unsafe { Pin::new_unchecked(msg_ref) };

        upcast(pinned_msg_ref)
    }

    /// Sets a vsomeip [vsomeip::message]'s [vsomeip::payload]
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
    pub fn set_message_payload(&self, payload_wrapper: &mut UniquePtr<PayloadWrapper>) {
        let message_ptr = MessageWrapper::get_mut(self);
        let payload_ptr = PayloadWrapper::get_mut(payload_wrapper);
        // SAFETY:
        // - Both raw pointers are obtained from live wrapper objects and are
        //   valid for this call.
        // - External C++ contract: `set_payload_raw` stores the payload through
        //   vsomeip shared ownership, not through borrowed Rust references.
        unsafe { set_payload_raw(message_ptr, payload_ptr) };
    }

    /// Gets a vsomeip [vsomeip::message]'s [vsomeip::payload]
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
    pub fn get_message_payload(&self) -> Option<UniquePtr<PayloadWrapper>> {
        let message_ptr = MessageWrapper::get_mut(self) as *const vsomeip::message;

        if message_ptr.cast::<()>().is_null() {
            error!("message_ptr is null");
            return None;
        }

        // SAFETY:
        // - `message_ptr` is non-null and came from a live `MessageWrapper`.
        // - External C++ contract: the returned payload pointer, when non-null,
        //   denotes the message's current payload object.
        let payload_ptr = unsafe { get_payload_raw(message_ptr) };

        if payload_ptr.cast::<()>().is_null() {
            error!("payload_ptr is null");
            return None;
        }

        // SAFETY:
        // - `payload_ptr` was checked for null and came from the C++ message.
        // - External C++ contract: the helper wraps the payload pointer without
        //   invalidating the message's shared ownership.
        let payload_wrapper = unsafe { create_payload_wrapper(payload_ptr) };

        if payload_wrapper.is_null() {
            error!("Failed to create UniquePtr<PayloadWrapper>");
        } else {
            trace!("Successfully created UniquePtr<PayloadWrapper>");
        }

        Some(payload_wrapper)
    }
}

impl PayloadWrapper {
    /// Gets a `Pin<&mut payload>` from a [PayloadWrapper]
    ///
    /// # Rationale
    ///
    /// In order to use the wrapped methods on an [vsomeip::payload], we must have a `Pin<&mut payload>`
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
    #[allow(clippy::mut_from_ref)]
    pub fn get_pinned(&self) -> Pin<&mut vsomeip::payload> {
        // SAFETY: `get_mut` returns the C++ payload pointee managed by this
        // wrapper when present.
        let payload = unsafe { self.get_mut().as_mut().unwrap() };
        // SAFETY:
        // - The C++ payload object is not moved while the returned pin is alive.
        // - Per https://doc.rust-lang.org/stable/std/pin/struct.Pin.html#method.new_unchecked,
        //   callers must guarantee "the value's data will not be moved nor have
        //   its storage invalidated until it gets dropped."
        unsafe { Pin::new_unchecked(payload) }
    }

    /// Sets a vsomeip [vsomeip::payload]'s byte buffer
    ///
    /// # Rationale
    ///
    /// We expose a safe API which is idiomatic to Rust, passing in a slice of u8 bytes
    ///
    /// # TODO
    ///
    /// Add some runtime safety checks on the pointer
    pub fn set_data_safe(&self, data: &[u8]) {
        // Get the length of the data
        let length = data.len() as u32;

        trace!("length of payload: {length}");

        // Get a pointer to the data
        let data_ptr = data.as_ptr();

        trace!("data_ptr: {data_ptr:?}");

        // SAFETY:
        // - `data_ptr` is derived from `data` and is valid for `length` bytes for
        //   the duration of this call.
        // - External C++ contract: vsomeip copies the bytes during `set_data` or
        //   otherwise does not retain `data_ptr` beyond the call.
        unsafe { self.get_pinned().set_data(data_ptr, length) };
    }

    /// Gets a vsomeip [vsomeip::payload]'s byte buffer
    ///
    /// # Rationale
    ///
    /// We expose a safe API which is idiomatic to Rust, returning a Vec of u8 bytes
    ///
    /// # TODO
    ///
    /// Add some runtime safety checks on the pointer
    pub fn get_data_safe(&self) -> Vec<u8> {
        let length = self.get_pinned().get_length();
        let data_ptr = self.get_pinned().get_data();

        trace!("get_data_safe: length: {length}");

        if data_ptr.is_null() {
            trace!("get_data_safe: data_ptr is null");
            return Vec::new();
        }
        trace!("get_data_safe: data_ptr is not null: {data_ptr:?}");

        trace!("Before slice::from_raw_parts");

        // SAFETY:
        // - Null pointers are handled above by returning an empty Vec.
        // - For non-null `data_ptr`, vsomeip reports `length` initialized bytes
        //   that remain valid for the duration of this call.
        // - Per https://doc.rust-lang.org/stable/std/slice/fn.from_raw_parts.html#safety,
        //   `data` must be "non-null, valid for reads for
        //   `len * size_of::<T>()` many bytes," properly aligned, and contained
        //   within a single allocation; those properties are supplied by the
        //   vsomeip payload API.
        let data_slice: &[u8] = unsafe { slice::from_raw_parts(data_ptr, length as usize) };

        trace!("After slice::from_raw_parts");

        // Convert the slice to a Vec
        let data_vec: Vec<u8> = data_slice.to_vec();

        trace!("after conversion to vec: {data_vec:?}");

        data_vec
    }
}
