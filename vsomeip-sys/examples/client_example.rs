use crossbeam_channel::{bounded, Receiver, Sender};
use cxx::{let_cxx_string, SharedPtr};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once};
use std::thread;
use std::thread::sleep;
use std::time::Duration;
use vsomeip_sys::pinned::{create_callback, get_pinned_application, get_pinned_message, get_pinned_message_base, get_pinned_payload, get_pinned_runtime, make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper, set_data_safe, AvailabilityHandlerCallbackStorage, register_message_handler_fn_ptr_safe, register_availability_handler_fn_ptr_safe};
use vsomeip_sys::vsomeip::{application, instance_t, message, message_base, runtime, service_t};
use vsomeip_sys::AvailabilityHandlerFnPtr;

const SAMPLE_SERVICE_ID: u16 = 0x1234;
const SAMPLE_INSTANCE_ID: u16 = 0x5678;
const SAMPLE_METHOD_ID: u16 = 0x0421;

fn start_app() {
    let my_runtime = runtime::get();
    let runtime_wrapper = make_runtime_wrapper(my_runtime);

    let_cxx_string!(my_app_str = "World");
    let mut app_wrapper = make_application_wrapper(
        get_pinned_runtime(&runtime_wrapper).create_application(&my_app_str),
    );

    let (my_availability_callback, _callback_id) =
        AvailabilityHandlerCallbackStorage::create_callback(|service, instance, availability| {
            println!(
                "Service [{:04x}.{:x}] is {}",
                service,
                instance,
                if availability {
                    "available."
                } else {
                    "NOT available."
                }
            );
        });
    get_pinned_application(&app_wrapper).init();
    // get_pinned_application(&app_wrapper).register_availability_handler(
    //     SAMPLE_SERVICE_ID,
    //     SAMPLE_INSTANCE_ID,
    //     my_availability_callback,
    //     vsomeip_sys::vsomeip::ANY_MAJOR,
    //     vsomeip_sys::vsomeip::ANY_MINOR,
    // );
    register_availability_handler_fn_ptr_safe(&mut app_wrapper,
                                              SAMPLE_SERVICE_ID,
                                              SAMPLE_INSTANCE_ID,
                                              my_availability_callback,
                                              vsomeip_sys::vsomeip::ANY_MAJOR,
                                              vsomeip_sys::vsomeip::ANY_MINOR,
    );
    let (my_response_callback) = create_callback(|response| println!("Got a response!"));
    // get_pinned_application(&app_wrapper).request_service(
    //     SAMPLE_SERVICE_ID,
    //     SAMPLE_INSTANCE_ID,
    //     vsomeip_sys::vsomeip::ANY_MAJOR,
    //     vsomeip_sys::vsomeip::ANY_MINOR,
    // );

    get_pinned_application(&app_wrapper).start();
}

fn main() {
    thread::spawn(move || {
        start_app();
    });

    println!("past the thread spawn");

    sleep(Duration::from_millis(2000));

    println!("past the sleep");

    let my_runtime = runtime::get();
    let runtime_wrapper = make_runtime_wrapper(my_runtime);

    println!("after we get the runtime");

    let_cxx_string!(my_app_str = "World");

    let app_wrapper =
        make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&my_app_str));

    let request = make_message_wrapper(get_pinned_runtime(&runtime_wrapper).create_request(true));
    get_pinned_message_base(&request).set_service(SAMPLE_SERVICE_ID);
    get_pinned_message_base(&request).set_instance(SAMPLE_INSTANCE_ID);
    get_pinned_message_base(&request).set_method(SAMPLE_METHOD_ID);

    let payload_wrapper =
        make_payload_wrapper(get_pinned_runtime(&runtime_wrapper).create_payload());

    let mut payload_data: Vec<u8> = Vec::new();
    for i in 0..10 {
        payload_data.push((i as u16 % 256) as u8);
    }

    set_data_safe(
        get_pinned_payload(&payload_wrapper),
        Box::from(payload_data),
    );

    let shared_ptr_message = request.as_ref().unwrap().get_shared_ptr();
    println!("attempting send...");
    get_pinned_application(&app_wrapper).send(shared_ptr_message);

    let request = make_message_wrapper(get_pinned_runtime(&runtime_wrapper).create_request(true));
    get_pinned_message_base(&request).set_service(SAMPLE_SERVICE_ID);
    get_pinned_message_base(&request).set_instance(SAMPLE_INSTANCE_ID);
    get_pinned_message_base(&request).set_method(SAMPLE_METHOD_ID);
    let shared_ptr_message = request.as_ref().unwrap().get_shared_ptr();
    println!("attempting send...");
    get_pinned_application(&app_wrapper).send(shared_ptr_message);

    sleep(Duration::from_millis(10000));

    get_pinned_application(&app_wrapper).unregister_availability_handler(
        SAMPLE_SERVICE_ID,
        SAMPLE_INSTANCE_ID,
        vsomeip_sys::vsomeip::ANY_MAJOR,
        vsomeip_sys::vsomeip::ANY_MINOR,
    )
}
