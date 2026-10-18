# CMake toolchain file for the Windows CI builds with Ninja and sccache.
# (.github/actions/sccache-setup/action.yml sets CMAKE_TOOLCHAIN_FILE.)
#
# A CMake Debug build uses /MDd and /Zi by default. Rust links the release
# C runtime, so /MDd objects do not link. sccache does not support /Zi:
# parallel compiles write one PDB file for each target and fail with
# C1041. These settings give /MD and /Z7 on the command line, so sccache
# sees the real options. The policy defaults apply also to projects that
# require an older CMake version.

# cmake-rs does not give the compiler when a toolchain file is set.
set(CMAKE_C_COMPILER cl)
set(CMAKE_CXX_COMPILER cl)

set(CMAKE_POLICY_DEFAULT_CMP0091 NEW)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreadedDLL)

set(CMAKE_POLICY_DEFAULT_CMP0141 NEW)
set(CMAKE_MSVC_DEBUG_INFORMATION_FORMAT "$<$<CONFIG:Debug,RelWithDebInfo>:Embedded>")
