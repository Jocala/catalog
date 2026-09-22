# CatalogQt: Qt Widgets + static Rust catalog-ffi (replaces CatalogWin).
# Mirrors adblink conventions (static Qt, Ninja, Qt Test, package-win).
cmake_minimum_required(VERSION 3.16)
project(catalogqt VERSION 1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

if("${CMAKE_BUILD_TYPE}" STREQUAL "")
    set(CMAKE_BUILD_TYPE "Release" CACHE STRING "" FORCE)
endif()

if(WIN32)
    set(CMAKE_WIN32_EXECUTABLE ON)
    set(CMAKE_PREFIX_PATH "C:/Qt/Qt.6.11.1-static" CACHE PATH "")
endif()

if(APPLE)
    # adblink pattern: static Qt, universal binary, macOS 14 floor.
    set(CMAKE_PREFIX_PATH "/Users/jeff/Qt/Qt.6.11.1-static" CACHE PATH "")
    set(CMAKE_OSX_ARCHITECTURES "x86_64;arm64" CACHE STRING "")
    set(CMAKE_OSX_DEPLOYMENT_TARGET "14.0" CACHE STRING "")
endif()

find_package(Qt6 REQUIRED COMPONENTS Core Gui Widgets Concurrent)

# Static Rust FFI lib. Per-platform recipe (pass -DFFI_LIB=... at
# configure when it differs):
# - win10: cargo build --release -p catalog-ffi with
#   RUSTFLAGS="-C target-feature=+crt-static" (match static Qt's CRT).
# - debian: plain cargo build --release -p catalog-ffi.
# - macOS: both arches + lipo into one universal .a (see below).
set(FFI_LIB "C:/source/reader/catalog/target/release/catalog_ffi.lib" CACHE FILEPATH "")

set(SOURCES
    main.cpp
    ffijson.cpp
    settings.cpp
    models.cpp
    catalogstore.cpp
    covercache.cpp
    bookmodel.cpp
    mainwindow.cpp
    bookinfodialog.cpp
    settingsdialog.cpp
    searchdialog.cpp
    aboutdialog.cpp
    helpdialog.cpp
    kobojob.cpp
    theme.cpp
    listdelegate.cpp
)
if(WIN32)
    list(APPEND SOURCES catalogqt.rc)
endif()

set(HEADERS
    ffi.h
    ffijson.h
    settings.h
    models.h
    catalogstore.h
    covercache.h
    bookmodel.h
    mainwindow.h
    bookinfodialog.h
    settingsdialog.h
    searchdialog.h
    aboutdialog.h
    helpdialog.h
    kobojob.h
    theme.h
    listdelegate.h
)

if(APPLE)
    # Bundle icon: canonical mirror of Resources/AppIcon.icns so the Qt
    # tree builds standalone (same convention as assets/appicon.ico).
    set(APP_ICON_MACOS "${CMAKE_CURRENT_SOURCE_DIR}/assets/AppIcon.icns")
    set_source_files_properties(${APP_ICON_MACOS}
        PROPERTIES MACOSX_PACKAGE_LOCATION "Resources")
endif()

qt_add_executable(catalogqt
    ${SOURCES}
    ${HEADERS}
    resources.qrc
    ${APP_ICON_MACOS}
)

set_target_properties(catalogqt PROPERTIES OUTPUT_NAME "JocalaCatalog")

if(APPLE)
    set_target_properties(catalogqt PROPERTIES
        MACOSX_BUNDLE TRUE
        MACOSX_BUNDLE_GUI_IDENTIFIER "com.jocala.catalogqt"
        MACOSX_BUNDLE_BUNDLE_NAME "JocalaCatalog"
        MACOSX_BUNDLE_ICON_FILE "AppIcon"
        MACOSX_BUNDLE_BUNDLE_VERSION "${PROJECT_VERSION}"
        MACOSX_BUNDLE_SHORT_VERSION_STRING "${PROJECT_VERSION}"
    )
endif()

target_include_directories(catalogqt PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(catalogqt PRIVATE
    Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Concurrent
    ${FFI_LIB}
)
if(WIN32)
    # Native sockets/crypto/account APIs the Rust staticlib needs on MSVC.
    target_link_libraries(catalogqt PRIVATE
        ws2_32 bcrypt ntdll userenv advapi32)
elseif(APPLE)
    # Frameworks for static Qt + the Rust staticlib (TLS/roots, system
    # accounts, USB/HID for device paths). Mirrors adblink's link set;
    # trim only if a link proves one unused.
    find_library(COCOA_FRAMEWORK Cocoa REQUIRED)
    find_library(CARBON_FRAMEWORK Carbon REQUIRED)
    find_library(IOKIT_FRAMEWORK IOKit REQUIRED)
    find_library(SECURITY_FRAMEWORK Security REQUIRED)
    find_library(SYSCONFIG_FRAMEWORK SystemConfiguration REQUIRED)
    find_library(COREFOUNDATION_FRAMEWORK CoreFoundation REQUIRED)
    target_link_libraries(catalogqt PRIVATE
        ${COCOA_FRAMEWORK}
        ${CARBON_FRAMEWORK}
        ${IOKIT_FRAMEWORK}
        ${SECURITY_FRAMEWORK}
        ${SYSCONFIG_FRAMEWORK}
        ${COREFOUNDATION_FRAMEWORK})
else()
    # Linux: pthreads + dl + libm for the same staticlib, plus the
    # system libs the engine's zip backend expects (bzip2, xz).
    find_package(Threads REQUIRED)
    target_link_libraries(catalogqt PRIVATE
        Threads::Threads ${CMAKE_DL_LIBS} m bz2 lzma)
endif()

if(WIN32)
    qt_import_plugins(catalogqt
        INCLUDE Qt6::QWindowsIntegrationPlugin
        INCLUDE Qt6::QModernWindowsStylePlugin
        INCLUDE Qt6::QSvgIconPlugin
        INCLUDE Qt6::QSvgPlugin
        INCLUDE Qt6::QGifPlugin
        INCLUDE Qt6::QICOPlugin
        INCLUDE Qt6::QJpegPlugin
    )
elseif(APPLE)
    qt_import_plugins(catalogqt
        INCLUDE Qt6::QCocoaIntegrationPlugin
        INCLUDE Qt6::QMacStylePlugin
        INCLUDE Qt6::QSvgIconPlugin
        INCLUDE Qt6::QSvgPlugin
        INCLUDE Qt6::QGifPlugin
        INCLUDE Qt6::QICOPlugin
        INCLUDE Qt6::QJpegPlugin
        INCLUDE Qt6::QICNSPlugin
    )
endif()

# Install layout: single exe + nothing else (help/about assets ride in qrc).
install(TARGETS catalogqt
    RUNTIME DESTINATION .
)

if(WIN32)
    configure_file(
        "${CMAKE_CURRENT_SOURCE_DIR}/packaging/catalogqt.iss.in"
        "${CMAKE_BINARY_DIR}/catalogqt.iss"
        @ONLY
    )
    add_custom_target(package-win
        COMMAND "c:/bin/inno/iscc" "${CMAKE_BINARY_DIR}/catalogqt.iss"
        DEPENDS catalogqt
        COMMENT "Generating Inno Setup installer..."
    )
endif()

enable_testing()
add_subdirectory(tests)
