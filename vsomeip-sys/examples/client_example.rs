use cxx::let_cxx_string;
use std::thread;
use std::thread::sleep;
use std::time::Duration;
use vsomeip_sys::glue::{
    make_application_wrapper, make_message_wrapper, make_payload_wrapper, make_runtime_wrapper,
};
use vsomeip_sys::safe_glue::{
    get_pinned_application, get_pinned_message_base, get_pinned_payload, get_pinned_runtime,
    set_data_safe, set_message_payload,
};
use vsomeip_sys::vsomeip::runtime;

const SAMPLE_SERVICE_ID: u16 = 0x1234;
// const SAMPLE_INSTANCE_ID: u16 = 0x5678;
const SAMPLE_INSTANCE_ID: u16 = 1;
const SAMPLE_METHOD_ID: u16 = 0x0421;
const APP_NAME: &str = "Hello";

fn start_app() {
    let my_runtime = runtime::get();
    let runtime_wrapper = make_runtime_wrapper(my_runtime);

    let_cxx_string!(my_app_str = APP_NAME);
    let app_wrapper = make_application_wrapper(
        get_pinned_runtime(&runtime_wrapper).create_application(&my_app_str),
    );

    if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
        pinned_app.init();
    } else {
        panic!("No app found for app_name: {APP_NAME}");
    }

    if let Some(pinned_app) = get_pinned_application(&app_wrapper) {
        pinned_app.start();
    } else {
        panic!("No app found for app_name: {APP_NAME}");
    }
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

    let_cxx_string!(my_app_str = APP_NAME);

    let app_wrapper =
        make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&my_app_str));

    get_pinned_application(&app_wrapper).request_service(
        SAMPLE_SERVICE_ID,
        SAMPLE_INSTANCE_ID,
        vsomeip_sys::vsomeip::ANY_MAJOR,
        vsomeip_sys::vsomeip::ANY_MINOR,
    );

    loop {
        sleep(Duration::from_millis(1000));

        let mut request =
            make_message_wrapper(get_pinned_runtime(&runtime_wrapper).create_request(true));
        get_pinned_message_base(&request).set_service(SAMPLE_SERVICE_ID);
        get_pinned_message_base(&request).set_instance(SAMPLE_INSTANCE_ID);
        get_pinned_message_base(&request).set_method(SAMPLE_METHOD_ID);

        let mut payload_wrapper =
            make_payload_wrapper(get_pinned_runtime(&runtime_wrapper).create_payload());

        let payload_string = "Hello, vsomeip!";
        let payload_data = payload_string.as_bytes();

        set_data_safe(get_pinned_payload(&payload_wrapper), payload_data);
        set_message_payload(&mut request, &mut payload_wrapper);

        let shared_ptr_message = request.as_ref().unwrap().get_shared_ptr();
        println!("attempting send...");
        get_pinned_application(&app_wrapper).send(shared_ptr_message);
    }
}
