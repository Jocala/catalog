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

find_package(Qt6 REQUIRED COMPONENTS Core Gui Widgets Concurrent)

# Static Rust FFI lib (cargo build --release -p catalog-ffi with
# RUSTFLAGS="-C target-feature=+crt-static" to match static Qt's CRT).
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
)

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
)

qt_add_executable(catalogqt
    ${SOURCES}
    ${HEADERS}
    resources.qrc
)

set_target_properties(catalogqt PROPERTIES OUTPUT_NAME "JocalaCatalog")

target_include_directories(catalogqt PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(catalogqt PRIVATE
    Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Concurrent
    ${FFI_LIB}
    ws2_32 bcrypt ntdll userenv advapi32
)

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
