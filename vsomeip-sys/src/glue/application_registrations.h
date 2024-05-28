#pragma once

#include <memory>
#include "vsomeip/vsomeip.hpp"  // Adjust the path as necessary to include the runtime type
#include "include/application_wrapper.h"

void register_message_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
        vsomeip_v3::instance_t _instance, vsomeip_v3::method_t _method
      , message_handler_fn_ptr _fn_ptr_handler
        );

void register_availability_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
        vsomeip_v3::instance_t _instance
      , availability_handler_fn_ptr _fn_ptr_handler,
      vsomeip_v3::major_version_t _major, vsomeip_v3::minor_version_t _minor);

//void register_message_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
//        vsomeip_v3::instance_t _instance, vsomeip_v3::method_t _method
//      , message_handler_fn_ptr _fn_ptr_handler
//        ) {
//
//    auto _handler = vsomeip_v3::message_handler_t(_fn_ptr_handler);
//
//    application_wrapper->get_shared_ptr()->register_message_handler(_service, _instance, _method, _handler);
//}