#include "../include/application_wrapper.h"

std::unique_ptr<ApplicationWrapper> make_application_wrapper(std::shared_ptr<vsomeip_v3::application> ptr) {
    return std::make_unique<ApplicationWrapper>(std::move(ptr));
}