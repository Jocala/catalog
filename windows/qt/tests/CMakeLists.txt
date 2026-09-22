find_package(Qt6 REQUIRED COMPONENTS Core Gui Widgets Test)

function(catalogqt_test name)
    qt_add_executable(${name} ${ARGN})
    target_include_directories(${name} PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/..)
    target_link_libraries(${name} PRIVATE Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Test)
    add_test(NAME ${name} COMMAND ${name})
endfunction()

catalogqt_test(tst_settings tst_settings.cpp ../settings.cpp ../models.cpp)
catalogqt_test(tst_models tst_models.cpp tst_models.h ../settings.cpp ../models.cpp)
