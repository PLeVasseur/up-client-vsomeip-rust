#pragma once

#include <memory>
#include "vsomeip/vsomeip.hpp"  // Adjust the path as necessary to include the runtime type

using message_handler_fn_ptr = void (*)(const std::shared_ptr< vsomeip_v3::message > &);
using availability_handler_fn_ptr = void (*)(vsomeip_v3::service_t, vsomeip_v3::instance_t, bool);

class ApplicationWrapper {
public:
    explicit ApplicationWrapper(std::shared_ptr<vsomeip_v3::application> ptr) : ptr_(std::move(ptr)) {}

    vsomeip_v3::application* get_mut() const {
        return ptr_.get();
    }

    std::shared_ptr<vsomeip_v3::application> get_shared_ptr() const {
        return ptr_;
    }

    ApplicationWrapper* get_self() {
        return this;
    }

private:
    std::shared_ptr<vsomeip_v3::application> ptr_;
};


std::unique_ptr<ApplicationWrapper> make_application_wrapper(std::shared_ptr<vsomeip_v3::application> ptr);

//std::unique_ptr<ApplicationWrapper> make_application_wrapper(std::shared_ptr<vsomeip_v3::application> ptr) {
//    return std::make_unique<ApplicationWrapper>(std::move(ptr));
//}

//void register_message_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
//        vsomeip_v3::instance_t _instance, vsomeip_v3::method_t _method
//
//      , message_handler_fn_ptr _fn_ptr_handler
//        ) {
//
//    auto _handler = vsomeip_v3::message_handler_t(_fn_ptr_handler);
//
////    application_wrapper.get_shared_ptr()->register_message_handler(_service, _instance, _method, _handler);
//}