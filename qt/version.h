#pragma once
// Single source of truth for the product version: the About dialog
// shows it and the update checker compares against it. Keep in sync
// with CMake project(VERSION) and packaging/catalog.iss.in (#VERSION).
#include <QString>

const QString kCatalogVersion = "1.03";
