# Builds libmgba (static, library only, no external dependencies) and
# Pactlocke's C shim over it. The shim must see exactly the defines libmgba was
# compiled with, because several of them change the layout of struct mCore and
# differ by OS, so they are copied from the mgba target rather than guessed.
cmake_minimum_required(VERSION 3.13)
project(pact_shim C)

if(NOT MGBA_DIR)
	message(FATAL_ERROR "Set MGBA_DIR to the libmgba source tree")
endif()

set(LIBMGBA_ONLY ON CACHE BOOL "" FORCE)
set(DISABLE_DEPS ON CACHE BOOL "" FORCE)
set(BUILD_STATIC ON CACHE BOOL "" FORCE)
set(BUILD_SHARED OFF CACHE BOOL "" FORCE)
set(M_CORE_GB OFF CACHE BOOL "" FORCE)
set(ENABLE_DEBUGGERS OFF CACHE BOOL "" FORCE)
set(ENABLE_GDB_STUB OFF CACHE BOOL "" FORCE)
set(ENABLE_SCRIPTING OFF CACHE BOOL "" FORCE)
set(USE_EDITLINE OFF CACHE BOOL "" FORCE)
set(SKIP_GIT ON CACHE BOOL "" FORCE)
add_subdirectory(${MGBA_DIR} mgba)

add_library(pact_shim STATIC pact_shim.c)
get_target_property(MGBA_DEFINES mgba COMPILE_DEFINITIONS)
target_compile_definitions(pact_shim PRIVATE ${MGBA_DEFINES})
target_include_directories(pact_shim PRIVATE ${MGBA_DIR}/include ${CMAKE_CURRENT_BINARY_DIR}/mgba/include)

install(TARGETS pact_shim mgba ARCHIVE DESTINATION lib)
