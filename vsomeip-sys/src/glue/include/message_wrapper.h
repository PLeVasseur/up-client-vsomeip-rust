#pragma once

#include <memory>
#include <iostream>
#include "vsomeip/vsomeip.hpp"  // Adjust the path as necessary to include the runtime type

class MessageWrapper {
public:
    explicit MessageWrapper(std::shared_ptr<vsomeip_v3::message> ptr) : ptr_(std::move(ptr)) {}

    vsomeip_v3::message* get_mut() const {
        return ptr_.get();
    }

    std::shared_ptr<vsomeip_v3::message> get_shared_ptr() const {
        return ptr_;
    }

private:
    std::shared_ptr<vsomeip_v3::message> ptr_;
};

std::unique_ptr<MessageWrapper> make_message_wrapper(std::shared_ptr<vsomeip_v3::message> ptr) {
    return std::make_unique<MessageWrapper>(std::move(ptr));
}

inline vsomeip_v3::message_base& upcast(vsomeip_v3::message& derived) {
    return derived;
}