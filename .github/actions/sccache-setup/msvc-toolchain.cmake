# CMake toolchain file for the Windows CI builds with Ninja and sccache.
# (.github/actions/sccache-setup/action.yml sets CMAKE_TOOLCHAIN_FILE.)
#
# A CMake Debug build uses /MDd and /Zi by default. Rust links the release
# C runtime, so /MDd objects do not link. sccache does not support /Zi:
# parallel compiles write one PDB file for each target and fail with
# C1041. These settings give /MD and /Z7 on the command line, so sccache
# sees the real options.

# cmake-rs does not give the compiler when a toolchain file is set.
set(CMAKE_C_COMPILER cl)
set(CMAKE_CXX_COMPILER cl)

# A project that requires CMake older than 3.15 (knf-rs-sys) gets the
# runtime and the debug format from these flags. CMake reads the toolchain
# file before its MSVC module, and that module keeps cache values that
# exist.
foreach(lang C CXX)
  set(CMAKE_${lang}_FLAGS_DEBUG "/MD /Z7 /Ob0 /Od /RTC1" CACHE STRING "")
  set(CMAKE_${lang}_FLAGS_RELWITHDEBINFO "/MD /Z7 /O2 /Ob1 /DNDEBUG" CACHE STRING "")
endforeach()

# A newer project (policies CMP0091 and CMP0141) gets them from these
# variables.
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)
set(CMAKE_MSVC_DEBUG_INFORMATION_FORMAT "$<$<CONFIG:Debug,RelWithDebInfo>:Embedded>")
