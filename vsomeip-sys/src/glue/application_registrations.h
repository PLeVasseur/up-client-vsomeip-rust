/********************************************************************************
 * Copyright (c) 2024 Contributors to the Eclipse Foundation
 *
 * See the NOTICE file(s) distributed with this work for additional
 * information regarding copyright ownership.
 *
 * This program and the accompanying materials are made available under the
 * terms of the Apache License Version 2.0 which is available at
 * https://www.apache.org/licenses/LICENSE-2.0
 *
 * SPDX-License-Identifier: Apache-2.0
 ********************************************************************************/

#pragma once

#include <memory>
#include "vsomeip/vsomeip.hpp"
#include "include/application_wrapper.h"

namespace glue {

void register_message_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
                                     vsomeip_v3::instance_t _instance, vsomeip_v3::method_t _method,
                                     message_handler_fn_ptr _fn_ptr_handler);

void register_availability_handler_fn_ptr(ApplicationWrapper* application_wrapper, vsomeip_v3::service_t _service,
                                          vsomeip_v3::instance_t _instance, availability_handler_fn_ptr _fn_ptr_handler,
                                          vsomeip_v3::major_version_t _major, vsomeip_v3::minor_version_t _minor);

} // namespace glue