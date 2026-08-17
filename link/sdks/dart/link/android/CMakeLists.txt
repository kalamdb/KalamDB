cmake_minimum_required(VERSION 3.18.1)
project(kalam_link_dart)

# Declare the pre-built kalam_link_dart shared library so that Flutter's
# ffiPlugin mechanism knows which native library to load.  The .so files are
# pre-built for each ABI by build_android_libs.sh and stored in jniLibs/.
# When the consuming Flutter app is built, Android Gradle automatically
# packages every .so found under src/main/jniLibs/ into the APK / AAB.

add_library(kalam_link_dart SHARED IMPORTED GLOBAL)
set_target_properties(kalam_link_dart PROPERTIES
    IMPORTED_LOCATION
        "${CMAKE_CURRENT_SOURCE_DIR}/src/main/jniLibs/${ANDROID_ABI}/libkalam_link_dart.so"
)
