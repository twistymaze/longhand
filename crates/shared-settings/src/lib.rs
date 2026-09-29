//! The shared settings of a storage folder (KTD1): the settings file
//! `.longhand/settings.json`, one markdown file for each custom summary
//! prompt in `.longhand/prompts/`, and the local cache that each computer
//! keeps of the last shared settings that it read. Every computer that
//! uses the storage folder reads and changes these files. Nothing here
//! depends on Tauri.

pub mod cache;
pub mod file;
pub mod join;
pub mod prompts;
pub mod view;
