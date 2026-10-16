I want to write a native image file viewer in Rust. It must work on Windows, macOS, and Linux. All the file viewing and manipulation needs to be native, without relying on the OS for support. 

The emphasis will be on JPEG XL, as an app to view, manipulate (resize, crop, etc) JPEG XL image and convert to other formats like normal JPEG, PNG, etc. Also convert other formats like JPEG and PNG to JPEG XL. When converting to JPEG XL the user has the option to use JPEG XL features like alpha channels, lossless, or tiling.

Although the viewer mustn't rely on the OS, it can use libraries like https://github.com/libjxl/libjxl or https://github.com/libjxl/jxl-rs