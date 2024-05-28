#pragma once

#include <memory>
#include "vsomeip/vsomeip.hpp"  // Adjust the path as necessary to include the runtime type

class RuntimeWrapper {
public:
    explicit RuntimeWrapper(std::shared_ptr<vsomeip_v3::runtime> ptr) : ptr_(std::move(ptr)) {}

    vsomeip_v3::runtime* get_mut() const {
        return ptr_.get();
    }

private:
    std::shared_ptr<vsomeip_v3::runtime> ptr_;
};

std::unique_ptr<RuntimeWrapper> make_runtime_wrapper(std::shared_ptr<vsomeip_v3::runtime> ptr) {
    return std::make_unique<RuntimeWrapper>(std::move(ptr));
}
