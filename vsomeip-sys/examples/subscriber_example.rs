use cxx::{let_cxx_string, SharedPtr};
use std::thread;
use std::thread::{park, sleep};
use std::time::Duration;
use vsomeip_sys::extern_callback_wrappers::{MessageHandlerFnPtr, SubscriptionStatusHandlerFnPtr};
use vsomeip_sys::glue::{make_application_wrapper, make_message_wrapper, make_runtime_wrapper};
use vsomeip_sys::safe_glue::{
    get_data_safe, get_message_payload, get_pinned_application, get_pinned_message_base,
    get_pinned_runtime, register_message_handler_fn_ptr_safe,
    register_subscription_status_handler_fn_ptr_safe, request_single_event_safe,
};
use vsomeip_sys::vsomeip;
use vsomeip_sys::vsomeip::{message, runtime, ANY_MAJOR, ANY_MINOR};

const SAMPLE_SERVICE_ID: u16 = 0x1234;
// const SAMPLE_INSTANCE_ID: u16 = 0x5678;
const SAMPLE_INSTANCE_ID: u16 = 1;

const SAMPLE_EVENTGROUP_ID: u16 = 0x4465;
const SAMPLE_EVENT_ID: u16 = 0x4465;

fn start_app() {
    let my_runtime = runtime::get();
    let runtime_wrapper = make_runtime_wrapper(my_runtime);

    let_cxx_string!(my_app_str = "Subscriber");
    let app_wrapper = make_application_wrapper(
        get_pinned_runtime(&runtime_wrapper).create_application(&my_app_str),
    );

    get_pinned_application(&app_wrapper).init();
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

    let_cxx_string!(my_app_str = "Subscriber");

    let mut app_wrapper =
        make_application_wrapper(get_pinned_runtime(&runtime_wrapper).get_application(&my_app_str));

    let client_id = get_pinned_application(&app_wrapper).get_client();
    println!("client_id: {client_id}");

    get_pinned_application(&app_wrapper).request_service(
        SAMPLE_SERVICE_ID,
        SAMPLE_INSTANCE_ID,
        ANY_MAJOR,
        ANY_MINOR,
    );
    request_single_event_safe(
        &mut app_wrapper,
        SAMPLE_SERVICE_ID,
        SAMPLE_INSTANCE_ID,
        SAMPLE_EVENT_ID,
        SAMPLE_EVENTGROUP_ID,
    );

    extern "C" fn subscription_status_listener(
        service: vsomeip::service_t,
        instance: vsomeip::instance_t,
        eventgroup: vsomeip::eventgroup_t,
        _event: vsomeip::event_t,
        status: u16, // TODO: This should really be an enum with a repr of u16: 0x00: OK or 0x7 Not OK
    ) {
        println!("Subscription status changed:\n service: {} instance: {} eventgroup: {} event: {} status: {}",
                 service, instance, eventgroup, instance, status);
    }

    let subscription_status_handler_fn_ptr =
        SubscriptionStatusHandlerFnPtr(subscription_status_listener);

    register_subscription_status_handler_fn_ptr_safe(
        &mut app_wrapper,
        vsomeip::ANY_SERVICE,
        vsomeip::ANY_INSTANCE,
        vsomeip::ANY_EVENTGROUP,
        vsomeip::ANY_EVENT,
        subscription_status_handler_fn_ptr,
        true,
    );

    get_pinned_application(&app_wrapper).subscribe(
        SAMPLE_SERVICE_ID,
        SAMPLE_INSTANCE_ID,
        SAMPLE_EVENTGROUP_ID,
        ANY_MAJOR,
        SAMPLE_EVENT_ID,
    );

    extern "C" fn my_msg_handler(_msg: &SharedPtr<message>) {
        println!("received event!");

        let cloned_msg = _msg.clone();
        let mut msg_wrapper = make_message_wrapper(cloned_msg);

        let msg_type = get_pinned_message_base(&msg_wrapper).get_message_type();
        println!("message_type_e: {msg_type:?}");

        let payload_wrapper = get_message_payload(&mut msg_wrapper);
        let payload = get_data_safe(&payload_wrapper);

        println!("payload:\n{payload:?}")
    }
    let my_callback = MessageHandlerFnPtr(my_msg_handler);

    register_message_handler_fn_ptr_safe(
        &mut app_wrapper,
        SAMPLE_SERVICE_ID,
        vsomeip::ANY_INSTANCE,
        SAMPLE_EVENT_ID,
        my_callback,
    );

    park();
}
