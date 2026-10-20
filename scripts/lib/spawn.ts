// Settings that the scripts give to the child processes that they start.

/// The output buffer of a child process. Node's default of 1 MB is too small
/// for some tools: cargo-deny, for example, writes more JSON than that.
export const MAX_OUTPUT_BYTES = 256 * 1024 * 1024;
