# Re-sign the CPack-staged app (mirrors adblink): CPack copies the
# build-tree .app into its staging dir, losing nothing but needing a
# fresh signature for the shipped DMG.
execute_process(
    COMMAND codesign --force --deep --sign
        "Developer ID Application: jeff elkins (9Q77WK7W3R)"
        --timestamp --options=runtime
        --entitlements "${ENTITLEMENTS_PATH}"
        "${CPACK_TEMPORARY_DIRECTORY}/JocalaCatalog.app"
    RESULT_VARIABLE result
)
if(NOT result EQUAL 0)
    message(FATAL_ERROR "codesign failed: ${result}")
endif()
message(STATUS "Signed JocalaCatalog.app in CPack staging directory")
