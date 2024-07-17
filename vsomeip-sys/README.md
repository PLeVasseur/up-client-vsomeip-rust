# vsomeip-sys

## What is it?

A fairly basic wrapper around the essentials needed to make a uProtocol uTransport implementation for SOME/IP based on top of the C++ vsomeip library.

## How do I build it?

1. Ensure you have a Rust toolchain installed
2. Ensure you have the vsomeip library installed
3. Ensure that you have the [requirements](https://github.com/COVESA/vsomeip?tab=readme-ov-file#build-instructions-for-linux) of the vsomeip project install

Then,

```bash
VSOMEIP_LIB_PATH=<path/to/vsomeip/lib> GENERIC_CPP_STDLIB_PATH=<path/to/generic/cpp/stdlib> ARCH_SPECIFIC_CPP_STDLIB_PATH=<path/to/arch_specific/cpp/stdlib> cargo build
```

If you are running a Linux-based OS on x86_64, it's possible these look like:

```
GENERIC_CPP_STDLIB_PATH = /usr/include/c++/11
ARCH_SPECIFIC_CPP_STDLIB_PATH=/usr/include/x86_64-linux-gnu/c++/11
```

## Running a binary built with vsomeip-sys

You will need to ensure that `LD_LIBRARY_PATH` includes the path to your vsomeip library install by:

```bash
LD_LIBRARY_PATH=$LD_LIBRARY_PATH:<path/to/vsomeip/lib> ./your_binary_built_with_vsomeip-sys
```

or by permanently modifying your `LD_LIBRARY_PATH` to include the path to your vsomeip library install.

## Allowing to build vsomeip for you (optional)

If you would like, you may supply:

```bash
VSOMEIP_COMPILE=true
```

as an environment variable when building the crate to have the vsomeip library source release we download for generating bindings built into an install you can deploy.

### vsomeip install path (optional)

You may additionally set:

```bash
VSOMEIP_INSTALL_PATH=/path/to/install/vsomeip
```

if you wish to choose where to install vsomeip. You can after-the-fact move this to some desired location on your system.

## Supplying your own vsomeip includes (optional)

You may modify the build command above to include setting the environment variable:

```
VSOMEIP_INCLUDE_PATH=<path/to/vsomeip/include>
```

`VSOMEIP_INCLUDE_PATH` must point to the folder which has the `vsomeip` folder within it containing the vsomeip includes.

## My build and deployment environments differ

### Specifying vsomeip library install location (optional)

In this case, we would specify the path to the vsomeip library _as it will exist in the deployment environment_.

So, for example, if we know in our deployment environment that the vsomeip library will be installed in `<your/deployment/env/vsomeip/lib>` then we would modify the above command to:

```bash
VSOMEIP_LIB_PATH=<your/deployment/env/vsomeip/lib>
```

This will compile just fine since the path is now baked into the binary and will be checked at runtime of the binary _on the deployment environment_.

### Note

Alternatively, if you ensure that the vsomeip library will be on the `LD_LIBRARY_PATH`, then it will be found.