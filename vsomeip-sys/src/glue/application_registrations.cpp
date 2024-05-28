#include "application_registrations.h"

void register_message_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
        vsomeip_v3::instance_t _instance, vsomeip_v3::method_t _method
      , message_handler_fn_ptr _fn_ptr_handler
        ) {

    auto _handler = vsomeip_v3::message_handler_t(_fn_ptr_handler);

    application_wrapper->get_shared_ptr()->register_message_handler(_service, _instance, _method, _handler);
}

void register_availability_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
        vsomeip_v3::instance_t _instance
      , availability_handler_fn_ptr _fn_ptr_handler,
      vsomeip_v3::major_version_t _major, vsomeip_v3::minor_version_t _minor
        ) {


    // Convert the function pointer to the expected type using a lambda
    vsomeip_v3::availability_state_handler_t _handler = [=](vsomeip_v3::service_t service, vsomeip_v3::instance_t instance, vsomeip_v3::availability_state_e availability_state) {
        bool available = !(availability_state == vsomeip_v3::availability_state_e::AS_UNAVAILABLE);

        _fn_ptr_handler(service, instance, available);
    };

    // uncommenting this line suddenly seems like a problem and we are told there exists
    // no matching function, despite the fact that when we don't have this uncommented
    // the binding exists in the generated Rust code...
    application_wrapper->get_shared_ptr()->register_availability_handler(_service, _instance, _handler, _major, _minor);
}

