# SPDX-License-Identifier: MIT
# FilmCraft fork: Dolphin / KIO thumbnails of FilmCraft projects (FORK-PLAN §1a.46).
#
# A thin KF6 KIO ThumbnailCreator that runs `filmcraft-cli thumbnail` and hands its PNG to KIO.
# Fork-only and self-contained: not part of the Cargo workspace; deleting this folder removes it.
#
#   cmake -B build -DCMAKE_INSTALL_PREFIX=/usr && cmake --build build && ctest --test-dir build
#
# See README.md for the user-local install of the fork build.
cmake_minimum_required(VERSION 3.16)
project(filmcraft-kde-thumbnailer VERSION 0.1.0 LANGUAGES CXX)

find_package(ECM 6.0 REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Gui)
find_package(KF6 6.0 REQUIRED COMPONENTS KIO CoreAddons)

# The CLI the plugin runs: a name looked up in PATH, or an absolute path (the fork build).
set(FILMCRAFT_CLI "filmcraft-cli" CACHE STRING "filmcraft-cli to run (name in PATH or absolute path)")
# Seconds before a thumbnail that is still rendering is given up.
set(FILMCRAFT_THUMB_TIMEOUT_S 20 CACHE STRING "thumbnail timeout in seconds")

kcoreaddons_add_plugin(filmcraftthumbnail SOURCES filmcraftthumbnail.cpp INSTALL_NAMESPACE "kf6/thumbcreator")
target_compile_definitions(filmcraftthumbnail PRIVATE
    FILMCRAFT_CLI="${FILMCRAFT_CLI}"
    FILMCRAFT_THUMB_TIMEOUT_S=${FILMCRAFT_THUMB_TIMEOUT_S})
target_link_libraries(filmcraftthumbnail PRIVATE KF6::KIOGui Qt6::Gui)

include(CTest)
if(BUILD_TESTING)
    # loads the built plugin like Dolphin's thumbnail worker and asks it for a thumbnail
    add_executable(thumbtest test/thumbtest.cpp)
    target_link_libraries(thumbtest PRIVATE KF6::KIOGui KF6::CoreAddons Qt6::Gui)
    add_test(NAME thumbtest COMMAND thumbtest $<TARGET_FILE:filmcraftthumbnail> ${CMAKE_CURRENT_BINARY_DIR}/test-data)
endif()
