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

use cmake::Config;
use decompress::ExtractOptsBuilder;
use reqwest::blocking::Client;
use std::error::Error;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use std::{env, fs};

const VSOMEIP_TAGGED_RELEASE_BASE: &str = "https://github.com/COVESA/vsomeip/archive/refs/tags/";
const VSOMEIP_VERSION_ARCHIVE: &str = "3.4.10.tar.gz";

fn main() -> miette::Result<()> {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    let user_supplied_vsomeip_include_path = env::var("VSOMEIP_INCLUDE_PATH");
    let (vsomeip_interface_path, vsomeip_compile_paths) = {
        // here we allow bringing the user's own vsomeip interface to bind against if they so wish
        // note that this binding is configured to work with the tagged release above of VSOMEIP_VERSION_ARCHIVE
        if let Ok(ref user_supplied_vsomeip_include_path) = user_supplied_vsomeip_include_path {
            (PathBuf::from(user_supplied_vsomeip_include_path), None)
        } else {
            let vsomeip_archive_dest = Path::new(&out_dir).join("vsomeip").join("vsomeip.tar.gz");
            let vsomeip_archive_url =
                format!("{VSOMEIP_TAGGED_RELEASE_BASE}{VSOMEIP_VERSION_ARCHIVE}");
            let vsomeip_source_folder = Path::new(&out_dir).join("vsomeip").join("vsomeip-src");
            download_and_write_file(&vsomeip_archive_url, &vsomeip_archive_dest)
                .expect("Unable to download released archive");
            decompress::decompress(
                vsomeip_archive_dest.clone(),
                vsomeip_source_folder.clone(),
                &ExtractOptsBuilder::default().strip(1).build().unwrap(),
            )
            .expect("Unable to extract tar.gz");
            println!(
                "cargo:warning=# vsomeip_source_folder: {}",
                vsomeip_source_folder.display()
            );
            let vsomeip_install_folder =
                Path::new(&out_dir).join("vsomeip").join("vsomeip-install");
            (
                vsomeip_source_folder.join("interface"),
                Some((vsomeip_source_folder, vsomeip_install_folder)),
            )
        }
    };

    let vsomeip_install_path = env::var("VSOMEIP_INSTALL_PATH");
    let compile_vsomeip = env::var("COMPILE_VSOMEIP");
    if let Ok(flag) = compile_vsomeip {
        println!("cargo:warning=# COMPILE_VSOMEIP flag set: {}", flag);
        if flag == "true" {
            println!("cargo:warning=# COMPILE_VSOMEIP flag set to true");
            if let Some((vsomeip_project_root, vsomeip_install_path_default)) =
                vsomeip_compile_paths
            {
                println!("cargo:warning=# vsomeip_project_root set");
                println!(
                    "cargo:warning=# vsomeip_project_root: {}",
                    vsomeip_project_root.display()
                );
                let vsomeip_install_path = {
                    if let Ok(vsomeip_install_path) = vsomeip_install_path {
                        PathBuf::from(vsomeip_install_path)
                    } else {
                        vsomeip_install_path_default
                    }
                };
                println!(
                    "cargo:warning=# vsomeip_install_path: {}",
                    vsomeip_install_path.display()
                );
                compile_vsomeip_from_source(vsomeip_project_root, vsomeip_install_path)?;
            }
        }
    }

    let vsomeip_lib_path = env::var("VSOMEIP_LIB_PATH")
        .expect("You must supply the path to a vsomeip library install, e.g. /usr/local/lib");
    let generic_cpp_stdlib = env::var("GENERIC_CPP_STDLIB_PATH")
        .expect("You must supply the path to generic C++ stdlib, e.g. /usr/include/c++/11");
    let arch_specific_cpp_stdlib = env::var("ARCH_SPECIFIC_CPP_STDLIB_PATH").expect("You must supply the path to architecture-specific C++ stdlib, e.g. /usr/include/x86_64-linux-gnu/c++/11");

    let project_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let runtime_wrapper_dir = project_root.join("src/glue/include"); // Update the path as necessary

    // Somewhat useful debugging
    println!("cargo:warning=# vsomeip_lib_path  : {}", vsomeip_lib_path);
    println!(
        "cargo:warning=# vsomeip_interface_path  : {}",
        vsomeip_interface_path.display()
    );
    println!(
        "cargo:warning=# generic_cpp_stdlib  : {}",
        generic_cpp_stdlib
    );
    println!(
        "cargo:warning=# arch_specific_cpp_stdlib  : {}",
        arch_specific_cpp_stdlib
    );

    generate_bindings(
        &out_dir,
        &vsomeip_interface_path,
        vsomeip_lib_path,
        &generic_cpp_stdlib,
        &arch_specific_cpp_stdlib,
        project_root,
        &runtime_wrapper_dir,
    )?;

    Ok(())
}

fn generate_bindings(
    out_dir: &OsString,
    vsomeip_interface_path: &PathBuf,
    vsomeip_lib_path: String,
    generic_cpp_stdlib: &String,
    arch_specific_cpp_stdlib: &String,
    project_root: PathBuf,
    runtime_wrapper_dir: &PathBuf,
) -> miette::Result<()> {
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
        .include(vsomeip_interface_path)
        .include(runtime_wrapper_dir)
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
    println!("cargo:warning=# file_path : {}", file_path.display());

    if !file_path.exists() {
        panic!("Unable to find autocxx generated code to rewrite");
    }

    // Run rustfmt on the file
    let status = Command::new("rustfmt")
        .arg(&file_path)
        .status()
        .expect("Failed to execute rustfmt");

    if !status.success() {
        panic!("Failed to format autocxx generated file");
    }

    // Open the input file for reading
    let input_file = File::open(&file_path).expect("Failed to open the input file for reading");
    let reader = BufReader::new(input_file);

    // Create a temporary file for writing the modified content
    let temp_file_path = file_path.with_extension("tmp");
    let temp_file =
        File::create(&temp_file_path).expect("Failed to create a temporary file for writing");
    let mut writer = BufWriter::new(temp_file);

    for line in reader.lines() {
        let mut line = line.expect("Failed to read a line from the input file");
        line = fix_unused_imports(line);
        line = fix_unsafe_fn_unused(line);
        line = fix_doc_build(line);
        line = add_enum_debug(line);

        writeln!(writer, "{}", line).expect("Failed to write a line to the temporary file");
    }

    writer.flush().expect("Failed to flush the writer buffer");
    fs::rename(temp_file_path, file_path)
        .expect("Failed to rename the temporary file to the original file");

    println!("cargo:warning=# rewrote the autocxx file");
    Ok(())
}

fn fix_unused_imports(line: String) -> String {
    let mut fixed_line = line.replace(
        "pub use bindgen::root::std_chrono_duration_long_AutocxxConcrete",
        "#[allow(unused_imports)] pub use bindgen::root::std_chrono_duration_long_AutocxxConcrete",
    );
    fixed_line = fixed_line.replace(
        "pub use bindgen::root::std_chrono_duration_int64_t_AutocxxConcrete;",
        "#[allow(unused_imports)] pub use bindgen::root::std_chrono_duration_int64_t_AutocxxConcrete;"
    );
    fixed_line = fixed_line.replace(
        "pub use super::super::bindgen::root::std::chrono::seconds;",
        "",
    );

    fixed_line
}

fn fix_unsafe_fn_unused(line: String) -> String {
    let mut fixed_line = line.replace("pub unsafe fn create_payload1", "unsafe fn create_payload1");
    fixed_line = fixed_line.replace(
        "pub unsafe fn set_data(self: Pin<&mut payload>, _data: *const u8, _length: u32);",
        "pub(crate) unsafe fn set_data(self: Pin<&mut payload>, _data: *const u8, _length: u32);",
    );

    fixed_line
}

fn fix_doc_build(line: String) -> String {
    // we may have to fix more in the future
    #[allow(clippy::let_and_return)]
    let fixed_line = line.replace("successfully [de]registered", "successfully de/registered");
    fixed_line
}

fn add_enum_debug(line: String) -> String {
    // we may have to fix more in the future
    #[allow(clippy::let_and_return)]
    let fixed_line = line.replace(
        "#[derive(Clone, Hash, PartialEq, Eq)]",
        "#[derive(Clone, Hash, PartialEq, Eq, Debug)]",
    );
    fixed_line
}

// Retrieves a file from `url` (from GitHub, for instance) and places it in the build directory (`OUT_DIR`) with the name
// provided by `destination` parameter.
fn download_and_write_file(url: &str, dest_path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(120))
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

fn compile_vsomeip_from_source(
    vsomeip_project_root: PathBuf,
    vsomeip_install_path: PathBuf,
) -> miette::Result<()> {
    let install_path = format!("{}", vsomeip_install_path.display());

    println!("cargo:warning=# install_path: {}", install_path);

    let vsomeip_cmake_build = Config::new(vsomeip_project_root)
        .define("CMAKE_INSTALL_PREFIX", install_path.clone())
        .define("ENABLE_SIGNAL_HANDLING", "1")
        .build_target("install")
        .build();

    println!(
        "cargo:warning=# vsomeip_cmake_build: {}",
        vsomeip_cmake_build.display()
    );

    Ok(())
}
