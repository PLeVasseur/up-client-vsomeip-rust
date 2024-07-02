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

use decompress::ExtractOptsBuilder;
use reqwest::blocking::Client;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs};

const VSOMEIP_TAGGED_RELEASE_BASE: &str = "https://github.com/COVESA/vsomeip/archive/refs/tags/";
const VSOMEIP_VERSION_ARCHIVE: &str = "3.4.10.tar.gz";

fn main() -> miette::Result<()> {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    let vsomeip_interface_path = {
        // here we allow bringing the user's own vsomeip interface to bind against if they so wish
        // note that this binding is configured to work with the tagged release above of VSOMEIP_VERSION_ARCHIVE
        let user_supplied_vsomeip_include_path = env::var("VSOMEIP_INCLUDE_PATH");
        if let Ok(user_supplied_vsomeip_include_path) = user_supplied_vsomeip_include_path {
            PathBuf::from(user_supplied_vsomeip_include_path)
        } else {
            let vsomeip_archive_dest = Path::new(&out_dir).join("vsomeip").join("vsomeip.tar.gz");
            let vsomeip_archive_url =
                format!("{VSOMEIP_TAGGED_RELEASE_BASE}{VSOMEIP_VERSION_ARCHIVE}");
            let vsomeip_source_folder = Path::new(&out_dir).join("vsomeip").join("vsomeip-src");
            download_and_write_file(&vsomeip_archive_url, &vsomeip_archive_dest)
                .expect("Unable to download released archive");
            decompress::decompress(
                vsomeip_archive_dest,
                vsomeip_source_folder.clone(),
                &ExtractOptsBuilder::default().strip(1).build().unwrap(),
            )
            .expect("Unable to extract tar.gz");
            vsomeip_source_folder.join("interface")
        }
    };

    let vsomeip_lib_path = env::var("VSOMEIP_LIB_PATH")
        .expect("You must supply the path to a vsomeip library install, e.g. /usr/local/lib");
    let generic_cpp_stdlib = env::var("GENERIC_CPP_STDLIB_PATH")
        .expect("You must supply the path to generic C++ stdlib, e.g. /usr/include/c++/11");
    let arch_specific_cpp_stdlib = env::var("ARCH_SPECIFIC_CPP_STDLIB_PATH").expect("You must supply the path to architecture-specific C++ stdlib, e.g. /usr/include/x86_64-linux-gnu/c++/11");

    let project_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let runtime_wrapper_dir = project_root.join("src/glue/include"); // Update the path as necessary

    // Somewhat useful debugging
    // println!("cargo:warning=# CARGO_MANIFEST_DIR : {}", project_root.display());
    // println!("cargo:warning=# OUT_DIR            : {}", out_path.display());
    // println!("cargo:warning=# vsomeip_interface  : {}", interface_path.display());
    // println!("cargo:warning=# runtime_wrapper    : {}", runtime_wrapper_dir.display());

    // we use autocxx to generate bindings for all those requested in src/lib.rs in the include_cpp! {} macro
    let mut b = autocxx_build::Builder::new(
        "src/lib.rs",
        [&vsomeip_interface_path, &runtime_wrapper_dir],
    )
    .extra_clang_args(&[
        format!("-I{}", generic_cpp_stdlib).as_str(),
        format!("-I{}", arch_specific_cpp_stdlib).as_str(),
        format!("-I{}", vsomeip_interface_path.display()).as_str(),
    ])
    .build()?;
    b.flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-deprecated-declarations") // suppress warnings from C++
        .flag_if_supported("-Wno-unused-function") // compiler compiling vsomeip
        .compile("autocxx-portion");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rustc-link-lib=vsomeip3");
    println!("cargo:rustc-link-search=native={}", vsomeip_lib_path);

    let include_dir = project_root.join("src/glue"); // Update the path as necessary

    // we use cxx to generate bindings for those couple of functions for which autocxx fails
    // due to usage of std::function inside of the function body
    cxx_build::bridge("src/cxx_bridge.rs")
        .file("src/glue/application_registrations.cpp")
        .file("src/glue/src/application_wrapper.cpp")
        .include(&include_dir)
        .include(&vsomeip_interface_path)
        .include(&runtime_wrapper_dir)
        .flag_if_supported("-Wno-deprecated-declarations") // suppress warnings from C++
        .flag_if_supported("-Wno-unused-function") // compiler compiling vsomeip
        .flag_if_supported("-std=c++17")
        .extra_warnings(true)
        .compile("cxx-portion");
    println!("cargo:rerun-if-changed=src/cxx_bridge.rs");

    // we rewrite the autocxx generated code to suppress the cargo warning about unused imports
    let file_path = Path::new(&out_dir)
        .join("autocxx-build-dir")
        .join("rs")
        .join("autocxx-ffi-default-gen.rs");
    if let Ok(mut contents) = fs::read_to_string(&file_path) {
        // Insert #[allow(unused_imports)] for specific lines
        contents = contents.replace(
            "pub use bindgen :: root :: std_chrono_duration_int64_t_AutocxxConcrete ;",
            "#[allow(unused_imports)]  pub use bindgen :: root :: std_chrono_duration_int64_t_AutocxxConcrete ;"
        );

        contents = contents.replace(
            "pub use super :: super :: bindgen :: root :: std :: chrono :: seconds ;",
            "#[allow(unused_imports)]  pub use super :: super :: bindgen :: root :: std :: chrono :: seconds ;"
        );

        // Removing pub from an unsafe function we never use to suppress warning
        contents = contents.replace("pub unsafe fn create_payload1", "unsafe fn create_payload1");

        // Rewriting a doc comment translated from C++ to not use [] link syntax
        contents = contents.replace("successfully [de]registered", "successfully de/registered");

        // Adding a derived Debug for the message_type_e enum
        contents = contents.replace(
            "# [repr (u8)] # [derive (Clone , Hash , PartialEq , Eq)] pub enum message_type_e",
              "# [repr (u8)] # [derive (Clone , Hash , PartialEq , Eq, Debug)] pub enum message_type_e"
        );

        fs::write(&file_path, contents).expect("Unable to write file");
    }

    Ok(())
}

// Retrieves a file from `url` (from GitHub, for instance) and places it in the build directory (`OUT_DIR`) with the name
// provided by `destination` parameter.
fn download_and_write_file(url: &str, dest_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120)) // Set a timeout of 60 seconds
        .build()?;
    let mut retries = 3;

    while retries > 0 {
        match client.get(url).send() {
            Ok(response) => {
                // Log the response headers
                println!("Headers: {:?}", response.headers());

                // Check rate limiting headers
                let rate_limit_remaining = response.headers().get("X-RateLimit-Remaining");
                let rate_limit_reset = response.headers().get("X-RateLimit-Reset");
                println!("Rate Limit Remaining: {:?}", rate_limit_remaining);
                println!("Rate Limit Reset: {:?}", rate_limit_reset);

                // Get the response body as bytes
                let response_body = response.bytes()?;
                println!("Body length: {:?}", response_body.len());

                // Create parent directories if necessary
                if let Some(parent_path) = dest_path.parent() {
                    std::fs::create_dir_all(parent_path)?;
                }

                // Create or open the destination file
                let mut out_file = fs::File::create(dest_path)?;

                // Write the response body to the file
                let result: Result<(), Box<dyn Error>> = out_file
                    .write_all(&response_body)
                    .map_err(|e| e.to_string().into());

                // Return the result if successful
                if result.is_ok() {
                    return result;
                } else {
                    println!("Error copying response body to file: {:?}", result);
                }
            }
            Err(e) => {
                println!("Error: {:?}", e);
                retries -= 1;
                if retries > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                } else {
                    return Err(Box::from(e));
                }
            }
        }
    }

    Err("Failed to download file after multiple attempts".into())
}
