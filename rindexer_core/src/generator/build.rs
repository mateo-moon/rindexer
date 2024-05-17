use ethers::contract::Abigen;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

use crate::helpers::{camel_to_snake, create_mod_file, write_file};
use crate::manifest::yaml::{read_manifest, Contract, Global, Indexer, Network};

use super::events_bindings::{
    abigen_contract_file_name, abigen_contract_name, generate_event_bindings,
    generate_event_handlers,
};
use super::{context_bindings::generate_context_code, networks_bindings::generate_networks_code};

/// Generates the file location path for a given output directory and location.
///
/// # Arguments
///
/// * `output` - The output directory.
/// * `location` - The location within the output directory.
///
/// # Returns
///
/// A `String` representing the full path to the file.
fn generate_file_location(output: &str, location: &str) -> String {
    format!("{}/{}.rs", output, location)
}

/// Writes the networks configuration to a file.
///
/// # Arguments
///
/// * `output` - The output directory.
/// * `networks` - A reference to a vector of `Network` configurations.
///
/// # Returns
///
/// A `Result` indicating success or failure.
fn write_networks(output: &str, networks: &[Network]) -> Result<(), Box<dyn Error>> {
    let networks_code = generate_networks_code(networks)?;
    write_file(&generate_file_location(output, "networks"), &networks_code)
}

/// Writes the global configuration to a file if it exists.
///
/// # Arguments
///
/// * `output` - The output directory.
/// * `global` - An optional reference to a `Global` configuration.
/// * `networks` - A reference to a slice of `Network` configurations.
///
/// # Returns
///
/// A `Result` indicating success or failure.
fn write_global(
    output: &str,
    global: &Option<Global>,
    networks: &[Network],
) -> Result<(), Box<dyn Error>> {
    if let Some(global) = global {
        let context_code = generate_context_code(&global.contracts, networks)?;
        write_file(
            &generate_file_location(output, "global_contracts"),
            &context_code,
        )?;
    }
    Ok(())
}

/// Identifies if the contract uses a filter and updates its name if necessary.
///
/// # Arguments
///
/// * `contract` - A mutable reference to a `Contract`.
///
/// # Returns
///
/// A `bool` indicating whether the contract uses a filter.
fn identify_filter(contract: &mut Contract) -> bool {
    let filter_count = contract
        .details
        .iter()
        .filter(|details| details.indexing_contract_setup().is_filter())
        .count();

    if filter_count > 0 && filter_count != contract.details.len() {
        panic!("Cannot mix and match address and filter for the same contract definition.");
    }

    if filter_count > 0 {
        contract.override_name(format!("{}Filter", contract.name));
        true
    } else {
        false
    }
}

/// Writes event bindings and ABI generation for the given indexer and its contracts.
///
/// # Arguments
///
/// * `output` - The output directory.
/// * `indexer` - A reference to an `Indexer`.
/// * `global` - An optional reference to a `Global` configuration.
///
/// # Returns
///
/// A `Result` indicating success or failure.
fn write_indexer_events(
    output: &str,
    indexer: Indexer,
    global: &Option<Global>,
) -> Result<(), Box<dyn Error>> {
    for mut contract in indexer.contracts {
        let databases = global.as_ref().map_or(&None, |g| &g.databases);
        let is_filter = identify_filter(&mut contract);
        let events_code = generate_event_bindings(&indexer.name, &contract, is_filter, databases)?;

        let event_path = format!(
            "{}/events/{}",
            camel_to_snake(&indexer.name),
            camel_to_snake(&contract.name)
        );
        write_file(&generate_file_location(output, &event_path), &events_code)?;

        // Write ABI gen
        let abi_gen = Abigen::new(abigen_contract_name(&contract), &contract.abi)?.generate()?;
        write_file(
            &generate_file_location(
                output,
                &format!(
                    "{}/events/{}",
                    camel_to_snake(&indexer.name),
                    abigen_contract_file_name(&contract)
                ),
            ),
            &abi_gen.to_string(),
        )?;
    }
    Ok(())
}

/// Generates code for the rindexer based on the manifest file.
///
/// # Arguments
///
/// * `manifest_location` - A reference to the path of the manifest file.
/// * `output` - The output directory.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub fn generate_rindexer_code(
    manifest_location: &PathBuf,
    output: &str,
) -> Result<(), Box<dyn Error>> {
    let manifest = read_manifest(manifest_location)?;

    write_networks(output, &manifest.networks)?;
    write_global(output, &manifest.global, &manifest.networks)?;

    for indexer in manifest.indexers {
        write_indexer_events(output, indexer, &manifest.global)?;
    }

    create_mod_file(Path::new(output))?;

    Ok(())
}

/// Generates code for indexer handlers based on the manifest file.
///
/// # Arguments
///
/// * `manifest_location` - A reference to the path of the manifest file.
/// * `output` - The output directory.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub fn generate_indexers_handlers_code(
    manifest_location: &PathBuf,
    output: &str,
) -> Result<(), Box<dyn Error>> {
    let manifest = read_manifest(manifest_location)?;

    for indexer in manifest.indexers {
        for mut contract in indexer.contracts {
            let is_filter = identify_filter(&mut contract);
            let result = generate_event_handlers(&indexer.name, is_filter, &contract)?;
            let handler_path = format!(
                "indexers/{}/{}",
                camel_to_snake(&indexer.name),
                camel_to_snake(&contract.name)
            );
            write_file(&generate_file_location(output, &handler_path), &result)?;
        }
    }

    create_mod_file(Path::new(output))?;

    Ok(())
}
