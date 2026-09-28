cmake_minimum_required(VERSION 3.24)
project(longhand_nemo_helper LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(LONGHAND_NEMO_SOURCE "" CACHE PATH "Pinned NeMo source")
set(LONGHAND_NEMO_BUILD "" CACHE PATH "Built NeMo SDK")
if(NOT EXISTS "${LONGHAND_NEMO_SOURCE}/src/asr/recognizer.h")
  message(FATAL_ERROR "Set LONGHAND_NEMO_SOURCE to the pinned SDK source")
endif()
execute_process(COMMAND git rev-parse HEAD WORKING_DIRECTORY "${LONGHAND_NEMO_SOURCE}" OUTPUT_VARIABLE revision OUTPUT_STRIP_TRAILING_WHITESPACE COMMAND_ERROR_IS_FATAL ANY)
if(NOT revision STREQUAL "97a15afa5caa9bce5baaa86c1184103877af4101")
  message(FATAL_ERROR "Unexpected NeMo source revision: ${revision}")
endif()
add_executable(longhand-nemo-helper main.cpp)
# Windows macros must not replace the C++ min/max functions.
target_compile_definitions(longhand-nemo-helper PRIVATE $<$<PLATFORM_ID:Windows>:NOMINMAX>)
target_include_directories(longhand-nemo-helper PRIVATE
  "${LONGHAND_NEMO_SOURCE}/include"
  "${LONGHAND_NEMO_SOURCE}/src/asr"
  "${LONGHAND_NEMO_SOURCE}/src/asr/features"
  "${LONGHAND_NEMO_SOURCE}/src/asr/encoder"
  "${LONGHAND_NEMO_SOURCE}/src/asr/decoders"
  "${LONGHAND_NEMO_SOURCE}/src/asr/vad"
  "${LONGHAND_NEMO_SOURCE}/src/asr/diar"
  "${LONGHAND_NEMO_SOURCE}/src/asr/postproc"
  "${LONGHAND_NEMO_SOURCE}/src/asr/pnc"
  "${LONGHAND_NEMO_SOURCE}/src/common"
  "${LONGHAND_NEMO_SOURCE}/src/runtime/ggml"
  "${LONGHAND_NEMO_SOURCE}/ggml/include"
  "${LONGHAND_NEMO_SOURCE}/llama.cpp/vendor")
find_library(NEMO_CPP NAMES nemo_speech_asr PATHS "${LONGHAND_NEMO_BUILD}/bin" "${LONGHAND_NEMO_BUILD}/lib" "${LONGHAND_NEMO_BUILD}/src/asr" "${LONGHAND_NEMO_BUILD}/src/asr/Release" NO_DEFAULT_PATH REQUIRED)
find_library(NEMO_C NAMES nemo_speech_asr_c PATHS "${LONGHAND_NEMO_BUILD}/bin" "${LONGHAND_NEMO_BUILD}/lib" "${LONGHAND_NEMO_BUILD}/src/asr" "${LONGHAND_NEMO_BUILD}/src/asr/Release" NO_DEFAULT_PATH REQUIRED)
find_library(GGML_BASE NAMES ggml-base PATHS "${LONGHAND_NEMO_BUILD}/bin" "${LONGHAND_NEMO_BUILD}/lib" "${LONGHAND_NEMO_BUILD}/ggml/src" "${LONGHAND_NEMO_BUILD}/ggml/src/Release" NO_DEFAULT_PATH REQUIRED)
target_link_libraries(longhand-nemo-helper PRIVATE "${NEMO_CPP}" "${NEMO_C}" "${GGML_BASE}")
if(APPLE)
  set_target_properties(longhand-nemo-helper PROPERTIES INSTALL_RPATH "@loader_path/../lib;@loader_path")
elseif(UNIX)
  set_target_properties(longhand-nemo-helper PROPERTIES INSTALL_RPATH "$ORIGIN/../lib;$ORIGIN")
endif()
install(TARGETS longhand-nemo-helper RUNTIME DESTINATION bin)
find_library(GGML NAMES ggml PATHS "${LONGHAND_NEMO_BUILD}/bin" "${LONGHAND_NEMO_BUILD}/lib" "${LONGHAND_NEMO_BUILD}/ggml/src" NO_DEFAULT_PATH REQUIRED)
target_link_libraries(longhand-nemo-helper PRIVATE "${GGML}")
include(CTest)
if(BUILD_TESTING)
  add_executable(result-rules-test result_rules_test.cpp)
  target_include_directories(result-rules-test PRIVATE "${LONGHAND_NEMO_SOURCE}/src/asr")
  add_test(NAME result-rules COMMAND result-rules-test)
endif()
