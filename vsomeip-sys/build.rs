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
use std::path::{Path, PathBuf};
use std::{env, fs};

const VSOMEIP_TAGGED_RELEASE_BASE: &str = "https://github.com/COVESA/vsomeip/archive/refs/tags/";
const VSOMEIP_VERSION_ARCHIVE: &str = "3.4.10.tar.gz";

fn main() -> miette::Result<()> {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let vsomeip_archive_dest = Path::new(&out_dir).join("vsomeip").join("vsomeip.tar.gz");
    let vsomeip_decompressed_folder = Path::new(&out_dir).join("vsomeip").join("vsomeip-src");
    let vsomeip_archive_url = format!("{VSOMEIP_TAGGED_RELEASE_BASE}{VSOMEIP_VERSION_ARCHIVE}");

    download_and_write_file(&*vsomeip_archive_url, &vsomeip_archive_dest)
        .expect("Unable to download released archive");
    decompress::decompress(
        vsomeip_archive_dest,
        vsomeip_decompressed_folder.clone(),
        &ExtractOptsBuilder::default().strip(1).build().unwrap(),
    )
    .expect("Unable to extract tar.gz");

    let project_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let runtime_wrapper_dir = project_root.join("src/glue/include"); // Update the path as necessary

    // let interface_path = Path::new("src").join("vsomeip-src").join("interface");
    let interface_path = vsomeip_decompressed_folder.join("interface");
    // for some reason unless we explicitly provide paths to headers for the stdlib here we have issues
    // I don't think we should really _have_ to do this though, as their locations are
    // more or less consistent on every instance of the different platforms
    // reference: https://github.com/google/autocxx/issues/1347#issuecomment-1928551787
    let mut b = autocxx_build::Builder::new("src/lib.rs", &[&interface_path, &runtime_wrapper_dir])
        .extra_clang_args(&[
            "-I/usr/include/c++/11",
            "-I/usr/include/x86_64-linux-gnu/c++/11",
        ])
        .build()?;
    b.flag_if_supported("-std=c++17").compile("vsomeip-sys"); // arbitrary library name, pick anything
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rustc-link-lib=vsomeip3");
    println!("cargo:rustc-link-search=native=/usr/local/lib");

    let include_dir = project_root.join("src/glue"); // Update the path as necessary

    cxx_build::bridge("src/cxx_bridge.rs")
        .file("src/glue/application_registrations.cpp")
        .file("src/glue/src/application_wrapper.cpp")
        .include(&include_dir)
        // .includes("src/glue/application_registrations.h")
        // .include("/usr/include/c++/11")
        // .include("/usr/include/x86_64-linux-gnu/c++/11")
        // .include("/usr/include")
        // .include(&interface_path)
        // .include(&runtime_wrapper_dir)
        .flag_if_supported("-std=c++17")
        .compile("cxxbridge");
    println!("cargo:rerun-if-changed=src/cxx_bridge.rs");

    Ok(())
}

// Retrieves a file from `url` (from GitHub, for instance) and places it in the build directory (`OUT_DIR`) with the name
// provided by `destination` parameter.
fn download_and_write_file(
    url: &str,
    dest_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    // Send a GET request to the URL
    match reqwest::blocking::get(url) {
        Ok(mut response) => {
            if let Some(parent_path) = dest_path.parent() {
                std::fs::create_dir_all(parent_path)?;
            }
            let mut out_file = fs::File::create(dest_path)?;

            let result: Result<(), Box<dyn std::error::Error>> = response
                .copy_to(&mut out_file)
                .map(|_| ())
                .map_err(|e| e.to_string().into());

            result
        }
        Err(e) => Err(Box::from(e)),
    }
}
